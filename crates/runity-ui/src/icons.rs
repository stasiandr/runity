//! The icons the UI ships with: Lucide (ISC, `assets/icons/LICENSE-LUCIDE.txt`),
//! by file name. Generated from `assets/icons/`; add a file there and a line
//! here.

/// Every icon, by name, sorted: an icon's number is its place here.
pub(crate) static ICONS: &[(&str, &[u8])] = &[
    (
        "arrow-up-from-line",
        include_bytes!("../assets/icons/arrow-up-from-line.svg"),
    ),
    ("box", include_bytes!("../assets/icons/box.svg")),
    ("boxes", include_bytes!("../assets/icons/boxes.svg")),
    ("camera", include_bytes!("../assets/icons/camera.svg")),
    ("check", include_bytes!("../assets/icons/check.svg")),
    (
        "chevron-down",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    (
        "chevron-right",
        include_bytes!("../assets/icons/chevron-right.svg"),
    ),
    (
        "chevron-up",
        include_bytes!("../assets/icons/chevron-up.svg"),
    ),
    (
        "circle-alert",
        include_bytes!("../assets/icons/circle-alert.svg"),
    ),
    (
        "circle-dot",
        include_bytes!("../assets/icons/circle-dot.svg"),
    ),
    (
        "clipboard-paste",
        include_bytes!("../assets/icons/clipboard-paste.svg"),
    ),
    ("component", include_bytes!("../assets/icons/component.svg")),
    ("cone", include_bytes!("../assets/icons/cone.svg")),
    ("copy", include_bytes!("../assets/icons/copy.svg")),
    ("crosshair", include_bytes!("../assets/icons/crosshair.svg")),
    ("cylinder", include_bytes!("../assets/icons/cylinder.svg")),
    (
        "ellipsis-vertical",
        include_bytes!("../assets/icons/ellipsis-vertical.svg"),
    ),
    ("expand", include_bytes!("../assets/icons/expand.svg")),
    ("eye", include_bytes!("../assets/icons/eye.svg")),
    ("eye-off", include_bytes!("../assets/icons/eye-off.svg")),
    ("file", include_bytes!("../assets/icons/file.svg")),
    ("file-box", include_bytes!("../assets/icons/file-box.svg")),
    ("file-plus", include_bytes!("../assets/icons/file-plus.svg")),
    ("focus", include_bytes!("../assets/icons/focus.svg")),
    ("folder", include_bytes!("../assets/icons/folder.svg")),
    (
        "folder-open",
        include_bytes!("../assets/icons/folder-open.svg"),
    ),
    ("globe", include_bytes!("../assets/icons/globe.svg")),
    ("grid-3x3", include_bytes!("../assets/icons/grid-3x3.svg")),
    ("hand", include_bytes!("../assets/icons/hand.svg")),
    ("house", include_bytes!("../assets/icons/house.svg")),
    ("image", include_bytes!("../assets/icons/image.svg")),
    ("info", include_bytes!("../assets/icons/info.svg")),
    ("layers-2", include_bytes!("../assets/icons/layers-2.svg")),
    ("lightbulb", include_bytes!("../assets/icons/lightbulb.svg")),
    ("list-tree", include_bytes!("../assets/icons/list-tree.svg")),
    ("lock", include_bytes!("../assets/icons/lock.svg")),
    ("lock-open", include_bytes!("../assets/icons/lock-open.svg")),
    ("magnet", include_bytes!("../assets/icons/magnet.svg")),
    ("menu", include_bytes!("../assets/icons/menu.svg")),
    ("mountain", include_bytes!("../assets/icons/mountain.svg")),
    (
        "mouse-pointer-2",
        include_bytes!("../assets/icons/mouse-pointer-2.svg"),
    ),
    ("move", include_bytes!("../assets/icons/move.svg")),
    ("move-3d", include_bytes!("../assets/icons/move-3d.svg")),
    ("music", include_bytes!("../assets/icons/music.svg")),
    ("package", include_bytes!("../assets/icons/package.svg")),
    ("pause", include_bytes!("../assets/icons/pause.svg")),
    ("pencil", include_bytes!("../assets/icons/pencil.svg")),
    ("play", include_bytes!("../assets/icons/play.svg")),
    ("plus", include_bytes!("../assets/icons/plus.svg")),
    ("redo-2", include_bytes!("../assets/icons/redo-2.svg")),
    ("rotate-3d", include_bytes!("../assets/icons/rotate-3d.svg")),
    ("rotate-cw", include_bytes!("../assets/icons/rotate-cw.svg")),
    ("route", include_bytes!("../assets/icons/route.svg")),
    ("save", include_bytes!("../assets/icons/save.svg")),
    ("scale-3d", include_bytes!("../assets/icons/scale-3d.svg")),
    ("scissors", include_bytes!("../assets/icons/scissors.svg")),
    ("search", include_bytes!("../assets/icons/search.svg")),
    ("settings", include_bytes!("../assets/icons/settings.svg")),
    (
        "sliders-horizontal",
        include_bytes!("../assets/icons/sliders-horizontal.svg"),
    ),
    ("sparkles", include_bytes!("../assets/icons/sparkles.svg")),
    ("square", include_bytes!("../assets/icons/square.svg")),
    (
        "step-forward",
        include_bytes!("../assets/icons/step-forward.svg"),
    ),
    ("terminal", include_bytes!("../assets/icons/terminal.svg")),
    ("trash", include_bytes!("../assets/icons/trash.svg")),
    (
        "triangle-alert",
        include_bytes!("../assets/icons/triangle-alert.svg"),
    ),
    ("undo-2", include_bytes!("../assets/icons/undo-2.svg")),
    (
        "wand-sparkles",
        include_bytes!("../assets/icons/wand-sparkles.svg"),
    ),
    ("x", include_bytes!("../assets/icons/x.svg")),
];

/// An icon's number, by name.
pub(crate) fn find(name: &str) -> Option<u16> {
    ICONS
        .binary_search_by(|(n, _)| n.cmp(&name))
        .ok()
        .map(|i| i as u16)
}
