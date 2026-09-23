//! The window, its layout, and the session it has open.
//!
//! Everything the editor *does* lives in `runity-editor`: one document open
//! for editing, and every change to it as a function, tested without a
//! window. This binary owns the window, the layout and the keyboard, and
//! calls those functions — the same ones an agent calls over MCP, so a
//! person's edit and an agent's are one edit (DNA, postulate 5).
//!
//! ```text
//! runity-studio [scene.ron]
//! ```
//!
//! With no scene named it opens the engine's reference scene, so that a
//! fresh clone shows a picture rather than an empty grid.
//!
//! The layout is Unity's: the Hierarchy on the left, the Inspector on the
//! right, the Scene view between them over the Console, a toolbar on top
//! and a status line under it all — each edge draggable. The look is
//! Nocturne ([`crate::theme`]).

use std::path::{Path, PathBuf};

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, ClickEvent, Context, Entity, TitlebarOptions,
    Window, WindowBounds, WindowOptions,
};
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::Root;
use runity::gizmo::Tool;
use runity_editor::console::Level;
use runity_editor::{Pivot, Session, Space};

use crate::console::Console;
use crate::hierarchy::Hierarchy;
use crate::inspector::Inspector;
use crate::theme::{self, *};
use crate::ui::{button, icon, icon_button, segment, segmented, separator};
use crate::viewport::SceneView;

/// The engine's reference scene: every builtin, no import step.
pub const REFERENCE_SCENE: &str = "examples/valley/scenes/first-light.ron";

/// The editor window: the Scene view and the panels around it, all over
/// one session.
pub struct Studio {
    session: Entity<Session>,
    scene_view: Entity<SceneView>,
    hierarchy: Entity<Hierarchy>,
    inspector: Entity<Inspector>,
    console: Entity<Console>,
}

impl Studio {
    /// The window's contents, over a session with a document open.
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session = cx.new(|_| session);
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let scene_view = cx.new(|cx| SceneView::new(session.clone(), cx));
        let hierarchy =
            cx.new(|cx| Hierarchy::new(session.clone(), scene_view.clone(), window, cx));
        let inspector = cx.new(|cx| Inspector::new(session.clone(), window, cx));
        let console = cx.new(|cx| Console::new(session.clone(), cx));
        Self {
            session,
            scene_view,
            hierarchy,
            inspector,
            console,
        }
    }

    /// The document this window has open.
    pub fn session(&self) -> &Entity<Session> {
        &self.session
    }

    /// Do something to the session from a toolbar button, and give the
    /// keyboard back to the Scene view: the button was a detour.
    fn act(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        what: impl FnOnce(&mut Session) -> Result<(), String>,
    ) {
        self.session.update(cx, |session, cx| {
            if let Err(message) = what(session) {
                session.say(Level::Error, message);
            }
            cx.notify();
        });
        self.scene_view
            .update(cx, |view, cx| view.focus(window, cx));
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let tool = session.tool();
        let playing = session.is_playing();
        let paused = session.is_paused();
        let local = session.space() == Space::Local;
        let center = session.pivot() == Pivot::Center;
        let grid = session.show_grid();
        let can_undo = session.can_undo();
        let can_redo = session.can_redo();
        let modified = session.is_modified();
        let name = session
            .scene_path()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());

        let tools = [
            (Tool::Move, "move-3d", "Move"),
            (Tool::Rotate, "rotate-3d", "Rotate"),
            (Tool::Scale, "scale-3d", "Scale"),
        ];
        let tool_switch = segmented(tools.into_iter().enumerate().map(|(i, (t, glyph, word))| {
            segment(("tool", i), glyph, Some(word), tool == t, i == 0)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.act(window, cx, |s| {
                        s.set_tool(t);
                        Ok(())
                    })
                }))
                .into_any_element()
        }));

        let play = icon_button("play", if playing { "square" } else { "play" }, playing).on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.act(window, cx, |s| {
                    if s.is_playing() {
                        s.stop();
                    } else {
                        s.play();
                    }
                    Ok(())
                })
            }),
        );
        let pause = icon_button("pause", "pause", paused).on_click(cx.listener(
            |this, _: &ClickEvent, window, cx| {
                this.act(window, cx, |s| {
                    if !s.is_playing() {
                        s.play();
                    }
                    let now = s.is_paused();
                    s.pause(!now);
                    Ok(())
                })
            },
        ));
        let step = icon_button("step", "step-forward", false).on_click(cx.listener(
            |this, _: &ClickEvent, window, cx| {
                this.act(window, cx, |s| {
                    if !s.is_playing() {
                        s.play();
                    }
                    s.step_once();
                    Ok(())
                })
            },
        ));

        let undo = icon_button("undo", "undo-2", false)
            .when(!can_undo, |b| b.opacity(0.45))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.act(window, cx, |s| {
                    s.undo().map(|_| ()).map_err(|e| e.to_string())
                })
            }));
        let redo = icon_button("redo", "redo-2", false)
            .when(!can_redo, |b| b.opacity(0.45))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.act(window, cx, |s| {
                    s.redo().map(|_| ()).map_err(|e| e.to_string())
                })
            }));
        let save = button("save", "Save", modified)
            .child(icon("save", if modified { rgb(ACCENT) } else { label() }))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.act(window, cx, |s| {
                    s.save_scene(None).map_err(|e| e.to_string())?;
                    s.say(Level::Info, "saved");
                    Ok(())
                })
            }));

        div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .h(px(40.0))
            .flex_none()
            .px(SPACE_3)
            // The brand and the document: `.nav-brand`, then the name,
            // muted, with the unsaved mark.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .min_w(px(180.0))
                    .child(div().size(px(8.0)).rounded_full().bg(rgb(ACCENT)))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child("runity"),
                    )
                    .child(div().text_size(px(13.0)).text_color(muted()).child(name))
                    .when(modified, |d| {
                        d.child(div().size(px(6.0)).rounded_full().bg(rgb(ACCENT_400)))
                    }),
            )
            .child(tool_switch)
            .child(separator())
            .child(
                icon_button("space", if local { "box" } else { "globe" }, false)
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(
                            "Handles: world or the entity's own (X)",
                        )
                        .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.act(window, cx, |s| {
                            let next = if s.space() == Space::Local {
                                Space::Global
                            } else {
                                Space::Local
                            };
                            s.set_space(next);
                            Ok(())
                        })
                    })),
            )
            .child(
                icon_button(
                    "pivot",
                    if center { "circle-dot" } else { "crosshair" },
                    false,
                )
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(
                        "Handles: pivot or selection's centre (Z)",
                    )
                    .build(window, cx)
                })
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.act(window, cx, |s| {
                        let next = if s.pivot() == Pivot::Center {
                            Pivot::Pivot
                        } else {
                            Pivot::Center
                        };
                        s.set_pivot(next);
                        Ok(())
                    })
                })),
            )
            .child(icon_button("grid", "grid-3x3", grid).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| {
                    this.act(window, cx, |s| {
                        let now = s.show_grid();
                        s.set_show_grid(!now);
                        Ok(())
                    })
                },
            )))
            // Play in the middle, as in Unity: the one control that changes
            // what the whole window means.
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_1)
                    .p(px(2.0))
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(if playing { mix(ACCENT, 60) } else { divider() })
                    .child(play)
                    .child(pause)
                    .child(step),
            )
            .child(div().flex_1())
            .child(undo)
            .child(redo)
            .child(separator())
            .child(save)
    }

    fn status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let selected = session.selection().len();
        let entities = session.entity_count();
        let last = session.undo_label();
        let playing = session.is_playing();
        let paused = session.is_paused();
        let (_, warnings, errors) = session.console_counts();
        let mut left = div()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .child(format!("{entities} entities"))
            .child(if selected == 0 {
                "nothing selected".to_string()
            } else {
                format!("{selected} selected")
            });
        if let Some(last) = last {
            left = left.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(icon("undo-2", mix(TEXT, 40)))
                    .child(last),
            );
        }
        div()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .h(px(22.0))
            .flex_none()
            .px(SPACE_4)
            .text_size(px(11.0))
            .text_color(muted())
            .child(left)
            .child(div().flex_1())
            .when(warnings > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_color(rgb(WARNING))
                        .child(icon("triangle-alert", rgb(WARNING)))
                        .child(format!("{warnings}")),
                )
            })
            .when(errors > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_color(rgb(ERROR))
                        .child(icon("circle-alert", rgb(ERROR)))
                        .child(format!("{errors}")),
                )
            })
            .child(
                div()
                    .text_color(if playing { rgb(ACCENT) } else { muted() })
                    .child(match (playing, paused) {
                        (true, true) => "paused",
                        (true, false) => "playing",
                        _ => "editing",
                    }),
            )
    }
}

/// A panel's place in the layout, with the gap Nocturne leaves between
/// cards: the ground shows between them.
fn slot(child: impl IntoElement) -> gpui::Div {
    div().size_full().p(px(3.0)).child(child)
}

impl Render for Studio {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let playing = self.session.read(cx).is_playing();
        let scene = div()
            .size_full()
            .rounded(RADIUS_MD)
            .overflow_hidden()
            .border_1()
            // While playing, the view is outlined in the accent: what
            // happens in it now is undone when play stops.
            .border_color(if playing {
                gpui::Hsla::from(rgb(ACCENT))
            } else {
                gpui::transparent_black()
            })
            .child(self.scene_view.clone());

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .font_family(FONT)
            .text_size(px(13.0))
            .child(self.toolbar(cx))
            .child(
                div().flex_1().min_h_0().px(px(5.0)).child(
                    h_resizable("studio-columns")
                        .child(
                            resizable_panel()
                                .size(px(260.0))
                                .size_range(px(160.0)..px(520.0))
                                .child(slot(self.hierarchy.clone())),
                        )
                        .child(
                            resizable_panel().child(
                                v_resizable("studio-middle")
                                    .child(resizable_panel().child(slot(scene)))
                                    .child(
                                        resizable_panel()
                                            .size(px(190.0))
                                            .size_range(px(60.0)..px(600.0))
                                            .child(slot(self.console.clone())),
                                    ),
                            ),
                        )
                        .child(
                            resizable_panel()
                                .size(px(330.0))
                                .size_range(px(220.0)..px(640.0))
                                .child(slot(self.inspector.clone())),
                        ),
                ),
            )
            .child(self.status(cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

/// What every window of the editor needs before it opens: GPUI Kit's
/// components, and Nocturne over them.
pub fn install(cx: &mut App) {
    gpui_kit::init(cx);
    theme::install(cx);
}

/// The window's root: GPUI Kit's `Root` (which carries dialogs, focus and
/// the kit's text inputs) around the studio.
pub fn window_root(session: Session, window: &mut Window, cx: &mut App) -> Entity<Root> {
    let studio = cx.new(|cx| Studio::new(session, window, cx));
    // The keyboard starts in the Scene view: W, E, R and F work before
    // anything is clicked.
    let view = studio.read(cx).scene_view.clone();
    view.update(cx, |view, cx| view.focus(window, cx));
    cx.new(|cx| Root::new(studio, window, cx))
}

/// Open the window and run until it closes.
pub fn run() {
    let scene = std::env::args().nth(1).map(PathBuf::from);
    let scene = scene.unwrap_or_else(|| PathBuf::from(REFERENCE_SCENE));

    let session = match open(&scene) {
        Ok(session) => session,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let title = scene
        .file_name()
        .map(|name| format!("runity — {}", name.to_string_lossy()))
        .unwrap_or_else(|| "runity".to_string());

    gpui_platform::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx: &mut App| {
            install(cx);
            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            let window = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some(title.into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| window_root(session, window, cx),
            );
            if let Err(error) = window {
                eprintln!("no window: {error}");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}

/// A session with `scene` open, or why it could not be.
///
/// The size here is a first guess: the view resizes the session to whatever
/// the window gives it on the first frame.
pub fn open(scene: &Path) -> Result<Session, String> {
    let mut session = Session::offscreen(1280, 720).map_err(|e| {
        format!("no renderer: {e}\nRUNITY_RENDERER and a working adapter are what this needs.")
    })?;
    let missing = session
        .open_scene(scene)
        .map_err(|e| format!("cannot open {}: {e}", scene.display()))?;
    for problem in missing {
        session.say(Level::Warning, problem);
    }
    Ok(session)
}
