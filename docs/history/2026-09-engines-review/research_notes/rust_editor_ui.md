# Rust GUI toolkits for the runity editor (September 2026)

Research date: 2026-09-22. Scope: replace the native Swift macOS editor with a
cross-platform Rust editor (macOS / Windows / Linux) that hosts the engine's own
wgpu 30 viewport. egui is out by owner decision ("ugly and poorly customizable").

Data sources: crates.io API (versions, dependency requirements, pulled
2026-09-22), project changelogs/blogs/READMEs, GitHub issues/PRs. Dependency
counts were measured locally with `cargo generate-lockfile` + `cargo tree`
(no compilation). Anything marked **[unverified]** could not be confirmed from a
primary source.

runity facts that matter here: `wgpu = "30"`, `winit = "0.30.13"` (optional),
`hecs 0.11`, `rapier3d 0.26`, license MIT OR Apache-2.0.

---

## 0. The constraint that decides most of this: wgpu version lockstep

To share a `wgpu::Device` / `wgpu::Texture` between the engine and the UI
without copies, both must link the **same wgpu major version** (different
majors are different Rust types; interop would need native-handle export/import
through wgpu-hal, which is per-backend `unsafe` work).

wgpu requirement of each toolkit's latest release (crates.io, 2026-09-22):

| Toolkit | Latest | GPU path | wgpu |
|---|---|---|---|
| Slint | 1.18.1 (2026-09-21) | FemtoVG-WGPU / Skia / Vello / software | **30** (`unstable-wgpu-30`), 29 also available |
| egui (reference only) | 0.36.2 | egui-wgpu | 30 |
| Bevy | 0.19.1 / 0.20.0-rc.1 | bevy_render | 29 / 30 |
| iced | 0.14.0 (2025-12-07) | iced_wgpu | 27 (master `0.15.0-dev`: 29) |
| GPUI (gpui-pre 0.3.6, used by GPUI Kit) | 0.3.6 (2026-09-21) | **Linux: wgpu 29; macOS: Metal; Windows: Direct3D (windows crate)** | 29 (Linux only) |
| Dioxus Native / Blitz | 0.7.10; blitz 0.3.0-beta.2 | anyrender_vello -> vello | 29 (via anyrender_vello 0.14) |
| Vello | 0.10.0 | compute 2D | 29 |
| Xilem / Masonry | 0.4.0 (2025-10-29) | vello 0.6 (main: vello 0.8, wgpu 28) | 28 on main |
| Kas | 0.17.0 (2026-01-27) | kas-wgpu | 28 |
| Floem | 0.2.0 (crates, 2024-11); main | vger/vello/skia | 22 (crates) / 27 (main) |
| Freya | 0.4.3 / 0.5.0-rc.7 | Skia (OpenGL via glutin) | none |
| Vizia | 0.4.0 (2026-04-23) | Skia (GL via glutin) | none |
| Makepad | 1.0.0 (2025-05-13) + active main | own: Metal / DX11 / OpenGL / WebGL | none |
| Fyrox UI | 1.0.1 (2026-03-28) | Fyrox renderer (OpenGL) [unverified: backend details] | none |

Sources: https://crates.io/api/v1/crates/i-slint-core/1.18.1/dependencies,
https://github.com/slint-ui/slint/blob/master/CHANGELOG.md,
https://crates.io/api/v1/crates/iced_wgpu/0.14.0/dependencies,
https://github.com/iced-rs/iced/blob/master/Cargo.toml,
https://crates.io/api/v1/crates/gpui-pre-wgpu/0.3.6/dependencies,
https://github.com/zed-industries/zed/pull/46758,
https://github.com/linebender/xilem/blob/main/Cargo.toml,
https://github.com/lapce/floem/issues/1086.

Takeaway: **only Slint 1.18 is on wgpu 30 today** among real candidates. Every
other wgpu-based toolkit would force runity to either pin wgpu to the toolkit's
version (29 for most; iced stable is 27) or copy frames. wgpu majors ship
roughly quarterly, so lockstep is an ongoing cost of any "shared device"
design. Worth writing down as an engine rule: the editor UI crate decides the
wgpu major, or the viewport goes through a version-independent boundary.

---

## 1. Candidate notes

### 1.1 Slint 1.18 (SixtyFPS GmbH)

- **Cadence / maturity**: 1.x API-stable since 2023. 2026 releases: 1.15 (Feb 4),
  1.16 (Apr 16), 1.17 (Jun 24), 1.18 (Sep 16), 1.18.1 (Sep 21) — a feature
  release every ~2–3 months. https://github.com/slint-ui/slint/releases,
  https://slint.dev/blog. Commercially backed company. ~500k recent downloads
  on crates.io.
- **Looks**: built-in styles (fluent, cupertino, material, cosmic, qt); in March
  2026 they changed the default away from "native-looking" styles
  (https://slint.dev/blog/default-native-style-change). Declarative styling,
  gradients, shadows (inner/outer, spread), clipping, `Path`, animations incl.
  new spring and path animations in 1.18, runtime z-order, FlexboxLayout
  (https://slint.dev/blog/slint-1.18-released). Custom widgets are written in
  the `.slint` language; custom drawing via `Path` or by importing a texture.
  Renderers: FemtoVG (GL or WGPU), Skia (GL/Vulkan/Metal/D3D/WGPU), software,
  experimental Vello (1.18).
- **Viewport embedding (strongest of all candidates)**:
  `BackendSelector::new().require_wgpu_30(WGPUConfiguration::…)` then either
  Slint creates the device or you pass your own (`WGPUConfiguration::Manual`);
  `set_rendering_notifier` gives `GraphicsAPI::WGPU30 { device, queue, .. }`;
  `slint::Image::try_from(wgpu::Texture)` imports an engine-rendered texture
  zero-copy; multiple `Image` elements = multiple viewports.
  https://docs.slint.dev/latest/docs/rust/slint/wgpu_30/. Restrictions: texture
  must be created on Slint's device; format `Rgba8Unorm` with
  `TEXTURE_BINDING | RENDER_ATTACHMENT` (https://docs.slint.dev/latest/docs/rust/slint/wgpu_28/ —
  the texture-import error docs); feature flag is `unstable-*` and is renamed
  with each wgpu major (27/28 removed, 29 and 30 present in 1.18).
  Bevy-in-Slint example announced in 1.12 (https://slint.dev/blog/slint-1.12-released).
  Community friction: https://github.com/slint-ui/slint/discussions/10681
  (Windows ICU link clash with Skia + Bevy; rendering Bevy into Slint poorly
  documented). Input forwarding to the viewport is manual (pointer/key events
  on a `TouchArea`/`FocusScope` -> engine camera).
- **Editor widgets**: ListView/StandardListView (virtualized), StandardTableView,
  TabWidget, LineEdit/TextEdit with IME (IME fixes in 1.18), PopupWindow,
  `ContextMenuArea`, `MenuBar`, `DragArea`/`DropArea` with `DataTransfer`
  (in-window since 1.17; cross-app on Qt backend and file paths in 1.18),
  multiple windows (each component is a window). **Missing: docking** —
  issue open since 2022, roadmap label, no assignee
  (https://github.com/slint-ui/slint/issues/1723). **No TreeView** in std-widgets
  [unverified absence; not found in changelog/docs search] — buildable from a
  ListView over a flattened tree.
- **Extensibility by third-party Rust modules**: UI must be written in the
  Slint DSL, but it can live inline in Rust (`slint!{}` macro, no build.rs) or
  be loaded at runtime with `slint-interpreter` (dynamic components, good for
  plugin panels and hot reload). Data crosses via properties/callbacks/models.
  A plugin API would likely be: plugin supplies `.slint` source + Rust model ->
  editor instantiates via interpreter.
- **Agents / testing**: best-in-class. Built-in MCP server (1.17+; element tree,
  screenshots, click/drag/keys, headless mode —
  https://slint.dev/blog/slint-and-AI-MCP), `i-slint-backend-testing`
  `ElementHandle` queries, `slint-viewer --screenshot`, `Window::take_snapshot()`,
  LSP + live preview with property editing, published agent "skills".
- **License (critical)**: triple license GPLv3 / Royalty-free 2.0 / commercial.
  Royalty-free permits proprietary desktop apps with attribution (AboutSlint
  widget or badge), but **forbids distributing an application "that exposes the
  APIs, in part or in total, of the Software"**
  (https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md).
  An editor whose plugin API lets third parties write Slint UI is plausibly
  exactly that. GPLv3 would force GPL on the editor (runity is MIT/Apache).
  -> Needs a written answer from SixtyFPS or a commercial license before
  committing. Wrapping Slint behind runity's own panel abstraction (plugins
  never touch Slint types) may avoid "exposing APIs" — [unverified legal
  interpretation].
- **Compile time / size**: 368 crates on Linux, 272 on macOS for
  `slint + unstable-wgpu-30` (dependency graph only). 1.18 claims leaner
  generated code and faster compiles. Skia renderer pulls prebuilt Skia
  binaries; FemtoVG-WGPU avoids that.
- **Production**: mostly embedded/industrial + some desktop apps; no known
  game-engine editor in production [unverified].

### 1.2 GPUI (Zed) + GPUI Kit / gpui-component (Longbridge)

- **What exists**: GPUI lives in the Zed monorepo, pre-1.0, "frequent breaking
  changes" (https://github.com/zed-industries/zed/tree/main/crates/gpui).
  crates.io `gpui` is stuck at 0.2.2 (2025-10-22). Real ecosystem runs on
  `gpui-pre` (weekly snapshot of Zed's in-tree gpui, 0.3.6 on 2026-09-21,
  **single-maintainer republish**, not official —
  https://github.com/0xErwin1/dbflux/issues/601). Community fork `gpui-ce`
  (https://github.com/gpui-ce/gpui-ce, crates 0.2.2 still Metal on macOS).
  gpui-component was renamed **GPUI Kit** (`longbridge/gpui-kit`, 0.6.6,
  Apache-2.0, 14.6k stars) — three layers: `gpui-component` (styled),
  `gpui-base` (unstyled primitives), `gpui-shell` (JS extensions)
  (https://gpui-kit.com/, https://github.com/longbridge/gpui-kit).
- **Looks**: modern shadcn/Zed-like out of the box, semantic theming; 120 fps
  target. Custom drawing via `canvas`/paths; Tailwind-like styling API in Rust.
- **Editor widgets (best available)**: Dock (draggable tab groups, nested splits,
  left/right/bottom docks, zoom, serde-persisted layout; no floating windows
  documented — https://gpui-kit.com/component/dock/), Tree, VirtualList,
  DataTable (virtualized), Menu/context menus, Input with IME (Zed-grade text),
  code Editor with tree-sitter/LSP, ColorPicker, Settings, Sidebar, Toolbar,
  TitleBar, NumberInput, Resizable, etc. Multi-window: yes (Zed).
- **Viewport embedding (weakest point)**: Zed replaced Blade with **wgpu only on
  Linux** (merged 2026-02-13; maintainers do not plan to replace Metal/Windows
  renderers — https://github.com/zed-industries/zed/pull/46758). gpui-pre 0.3.6:
  Linux -> `gpui-pre-wgpu` (wgpu 29); macOS -> Metal; Windows -> Direct3D via
  `windows` crate (crates.io deps). gpui-ce has `Window::gpu_context()` +
  `paint_surface()` for compositing a wgpu texture, but only on its
  wgpu-backed platforms, "type-erased, not documented" (issue opened 2026-09-06:
  https://github.com/gpui-ce/gpui-ce/issues/224). Zero-star fork
  `chitin-dev/gpui-wgpu` claims wgpu on all platforms with `WgpuSurface`
  (https://github.com/chitin-dev/gpui-wgpu). On macOS, Zed's `surface()`
  element displays a CVPixelBuffer (IOSurface) — an engine could render into an
  IOSurface-backed Metal texture via wgpu-hal and hand it over [unverified
  path, would need custom code; Windows equivalent needs a DXGI shared handle].
  Net: 3 different integration paths, all custom, no official support.
- **Extensibility**: plain Rust structs implementing `Render`; a plugin crate
  can add a panel by implementing GPUI Kit's `Panel` trait. No DSL. Best
  ergonomics for "a module adds a panel".
- **Agents / testing**: `TestAppContext`/`VisualTestContext` for headless logic
  tests [unverified: offscreen pixel screenshots across platforms]. No UI hot
  reload. Rust code is verbose but regular; agents handle it well (Zed itself).
- **Maturity**: production — Zed (editor), Longbridge Pro (trading app), many
  apps (dbflux etc.). But upstream owner optimises for Zed only.
- **Compile time**: heaviest measured — 551 crates on Linux, 449 on macOS for
  `gpui-kit` (includes web/JS shell layers).

### 1.3 iced 0.14 (+ libcosmic)

- **Cadence**: 0.13 (Sep 2024), 0.14.0 (2025-12-07); nothing released in 2026
  so far; master is `0.15.0-dev` on wgpu 29, rust 1.93
  (https://github.com/iced-rs/iced/releases, https://github.com/iced-rs/iced/blob/master/Cargo.toml).
  Breaking changes every minor. ~590k recent downloads. MIT.
- **0.14 features**: reactive rendering, **hot reloading**, **headless mode
  testing**, first-class end-to-end testing, time-travel debugging, input
  method (IME) support, `table`, `grid`, `float`, `pin`, `sensor` widgets
  (https://github.com/iced-rs/iced/releases/tag/0.14.0). Multi-window since 0.12.
- **Looks**: clean but plain by default; themes with palette generation;
  functional per-widget styling. COSMIC desktop (libcosmic, iced fork with its
  own design system, frosted glass in COSMIC 1.3, 2026) proves it can look
  polished (https://github.com/pop-os/libcosmic). libcosmic is Linux/COSMIC
  focused and tracks a forked iced — not a good cross-platform base.
- **Viewport embedding**: very good *architecturally*: `widget::shader` gives a
  `Program` whose primitives render with iced's own `wgpu::Device`/`Queue` into
  the widget's viewport — many shader widgets = many viewports
  (https://docs.iced.rs/iced/widget/shader/index.html); or embed iced into your
  own wgpu loop (`integration` example). **But wgpu 27 (stable) / 29 (master)
  vs runity's 30.**
- **Editor widgets**: `pane_grid` (split/drag/resize, no tab stacks — docking
  needs building on top), `table` (0.14), lazy/virtual lists limited
  [unverified: true row virtualization in 0.14 table], text_editor, combo_box,
  context menus via community crates (iced_aw), drag & drop inside app limited.
  No tree view built in.
- **Extensibility**: Elm architecture; plugins must map their messages into the
  editor's message type — workable (`Element::map`) but ceremony-heavy.
- **Compile**: lightest — 238 crates Linux / 153 macOS.
- **Production**: COSMIC desktop (System76), Halloy IRC, Kraken desktop
  [unverified], Sniffnet.

### 1.4 Xilem / Masonry (Linebender)

- 0.4.0 (2025-10-29); main moved Masonry to the `imaging` abstraction (Vello,
  Vello CPU, etc.), new layout system, widgets Split, CollapsePanel, StepInput,
  Canvas (https://linebender.org/blog/tmil-25/). Still self-described
  experimental (https://github.com/linebender/xilem). wgpu 28 on main.
  No docking, no tree, small widget set. Great long-term tech (Vello, Parley,
  AccessKit), not an editor base in 2026.

### 1.5 Freya 0.4 (marc2332)

- 0.4 (July 2026) dropped Dioxus for its own reactive core; hot reload via
  subsecond/dx; new crates for animation, icons, material design, router
  (https://freyaui.dev/posts/0.4 — fetch rate-limited, content via search
  snippet; https://docs.rs/crate/freya/latest). 0.5.0-rc.7 in progress. Skia over
  OpenGL (glutin). Beautiful output, testing crate exists.
- **Viewport**: no wgpu texture import; request opened 2026-09-02, milestone
  0.5 (https://github.com/marc2332/freya/issues/2232). Would need GL<->wgpu
  interop or copies. No docking. Single maintainer.

### 1.6 Vizia

- 0.4.0 (2026-04-23), Skia/GL, CSS-like styling, used in audio plugins (Meadowlark
  history) [unverified current users]. No wgpu, no docking. Low downloads.

### 1.7 Floem (Lapce)

- crates.io 0.2.0 is from 2024-11; main updated 2026 but on wgpu 27
  (https://github.com/lapce/floem/issues/1086). Powers Lapce. Fine-grained
  reactivity, pure Rust. No docking. Release cadence effectively stalled on
  crates.io.

### 1.8 Dioxus Native / Blitz

- dioxus-native 0.7.10 (2026-07-31), 0.8.0-alpha.1; Blitz = HTML/CSS engine
  (Stylo + Taffy + Parley) rendering via anyrender/Vello (wgpu 29 in
  blitz 0.3 beta). Has a `CustomPaintSource` that receives a wgpu device/queue
  to paint into an element — the right hook for a viewport
  (https://docs.rs/dioxus-native/latest/dioxus_native/). HTML/CSS styling means
  "beautiful" is easy and agents write it fluently; Dioxus hot reload +
  subsecond hot-patching. Still experimental; no docking library for native
  (web ones need a JS runtime). 468 crates on Linux.

### 1.9 Makepad

- 1.0.0 on crates (2025-05); main pivoted to "AI-accelerated app and game dev
  environment" (https://github.com/makepad/makepad). Own GPU layer (Metal, DX11,
  OpenGL, WebGL) — **not wgpu**, so embedding runity's viewport needs native
  interop. Shader-based styling (very pretty), `live_design!` DSL hot-reloads,
  built-in Dock widget used by Makepad Studio, very fast compiles. Tiny team,
  docs thin. Production: Robrix (Matrix client) [unverified maturity].

### 1.10 Cushy, Kas

- Cushy 0.4.0 (2024-08) — stale. Kas 0.17 (2026-01), wgpu 28, niche. Not viable.

### 1.11 Web UI (Tauri / wry, or Electron-style) with native viewport

- How others do it: Cocos Creator's editor is Electron (Node + Chromium)
  with the scene view in a renderer process
  (https://docs.cocos.com/creator/3.8/manual/en/getting-started/introduction/);
  PlayCanvas / Babylon.js editors are web-native (viewport is WebGL/WebGPU in
  the same page) [unverified for Babylon's current editor stack].
- Tauri 2.11 / wry 0.57 (Sep 2026). A native wgpu surface cannot be composited
  *inside* the DOM. Options: (a) wgpu surface under a transparent webview in the
  same window (Tauri multi-webview; transparency/z-order platform-specific —
  https://github.com/tauri-apps/tauri/issues/8246,
  https://github.com/orgs/tauri-apps/discussions/11944); (b) stream frames to the
  webview (copies, latency); (c) separate child native window. Electrobun has a
  `<electrobun-wgpu>` element (https://blackboard.sh/blog/wgpu-in-electrobun/) — Bun/TS, not Rust.
- Pros: best-looking, huge widget ecosystem (dockview/FlexLayout, virtualized
  trees, perfect IME), trivial Playwright screenshot tests, agents excel at
  HTML/CSS/TS. Cons: two languages + IPC; Rust plugins can't add panels in
  plain Rust (would need a schema-driven inspector or ship JS); WebKitGTK on
  Linux is slow/buggy; viewport compositing is the fragile part (multi-viewport
  worse). Also Slint can host Servo (https://slint.dev/blog/servo-with-slint-update)
  — an interesting hybrid but immature [unverified].

### 1.12 Reference points (engines / tools)

- **Bevy**: no official editor yet. Bevy 0.19 (June 2026) shipped BSN scenes and
  a large set of **Feathers** editor widgets ported to BSN; 6th-birthday post
  (2026-08-10) says the editor working group spins up after a few more
  foundational pieces (https://bevy.org/news/bevys-sixth-birthday/,
  https://bevy.org/news/bevy-0-19/). UI = bevy_ui (ECS entities) + style-less
  core widgets + Feathers. Only usable if the editor runs Bevy's renderer/app.
- **Jackdaw** (community Bevy editor, MIT/Apache, nightly-only): bevy_ui +
  `jackdaw_feathers` widgets, docking panels, hierarchy/inspector/3D viewport,
  runtime-loadable signed `.jdext` extensions in plain Rust
  (https://github.com/jbuehler23/jackdaw, Cargo.toml checked). The closest thing
  to "Rust game editor with plain-Rust plugins" — but tied to Bevy.
- **Fyrox**: 1.0 released March 2026; editor is built on its own `fyrox-ui`
  (retained, docking, tree, inspector via reflection; ~3x UI perf gain in 1.0)
  (https://fyrox.rs/blog/post/fyrox-game-engine-1-0-0/). Proves "roll your own
  UI on the engine renderer" works but took years; look is dated.
- **Rerun**: egui (+ egui_tiles). **Zed**: GPUI. **Lapce**: Floem.
  **Ambient**: archived; had its own ECS-based UI [unverified detail].
  **Tiny Glade** tools: could not find a primary source on its tool UI
  [unverified].

---

## 2. Scoring against the requirements

Legend: ++ excellent, + good, 0 workable with effort, - weak, -- blocker.

| Req | Slint | GPUI+Kit | iced | Web (Tauri) | Dioxus Native | Makepad | Freya |
|---|---|---|---|---|---|---|---|
| 1 Looks + customization | + | ++ | 0 | ++ | + | + | + |
| 2 wgpu 30 viewport(s) | ++ (native wgpu 30, zero-copy) | - (Linux-only wgpu 29; Metal/D3D elsewhere) | + (shader widget; wgpu 27/29) | - (overlay windows) | + (CustomPaint, wgpu 29) | -- (no wgpu) | -- (GL Skia) |
| 3 Editor widgets | 0 (no dock, no tree) | ++ | 0 (pane_grid, no tabs/tree) | ++ | - | + (Dock) | - |
| 4 Plain-Rust plugins | 0 (DSL, but interpreter) | ++ | + | - | + | 0 (DSL) | + |
| 5 Maturity / license | + / **license risk** | 0 (pre-1.0, snapshot crates) | 0 (slow, breaking) | ++ | - | - | 0 |
| 6 Agents / headless | ++ (MCP, testing, preview) | 0 | + (headless, e2e, hot reload) | ++ | + | + | + |
| 7 Compile weight (Linux crates) | 368 | 551 | 238 | low Rust side | 468 | small [unverified] | 315 |

(egui_dock/eframe would be 263 — rejected, listed only for scale.)

---

## 3. Ranked recommendation

**1. Slint 1.18 — if the license question is resolved.**
Only toolkit whose release today matches runity's wgpu 30, with an officially
documented zero-copy texture import and access to the shared device; multiple
viewports are just multiple `Image`s. Strongest agent/headless story (MCP
server, testing backend, screenshots, live preview) and a steady ~quarterly
release cadence. Costs: build docking + tree view ourselves (a few weeks, done
once in `.slint`); UI in a DSL (inline `slint!` or runtime-interpreted, so no
extra compiler step); wgpu feature is `unstable-*` and bumps with every wgpu
major. **Blocker to clear first**: Royalty-free 2.0 forbids distributing an app
that exposes Slint's APIs — an extensible editor may qualify; get SixtyFPS'
written interpretation or budget a commercial license.

**2. GPUI + GPUI Kit — best editor UX and plugin ergonomics, worst viewport story.**
Dock, Tree, VirtualList, DataTable, menus, IME-grade text, beautiful default
theme, plain-Rust panels (ideal for third-party modules), Apache-2.0, proven in
Zed and Longbridge Pro. Costs: viewport embedding needs per-platform native
interop (Metal IOSurface on macOS, D3D shared handle on Windows, wgpu 29 on
Linux) or a fork (gpui-ce / gpui-wgpu); depends on a single-maintainer weekly
snapshot (`gpui-pre`); pre-1.0 breakage; heaviest dependency tree; no hot
reload. Pick this if a spike proves the macOS/Windows texture handoff in < 1–2
weeks.

**3. iced (0.14 now, 0.15 when released) — safest pure-wgpu engineering choice.**
Shader widget shares iced's wgpu device, so viewports are first-class; hot
reload, headless + e2e tests, IME, multi-window, lightest compile. Costs: will
not satisfy "beautiful out of the box" without our own design system (COSMIC
shows it is possible); docking with tabs, tree view and rich DnD must be built;
wgpu version lags (27 stable, 29 master) — runity would have to pin to iced's
wgpu; slow, breaking releases.

Not recommended now: Dioxus Native (right hooks, too experimental — revisit in
2027), Makepad/Freya/Vizia (no wgpu path), Xilem/Floem/Kas/Cushy (maturity),
web UI (viewport compositing and plain-Rust plugins both fight the design),
Bevy UI/Feathers (requires Bevy runtime; but watch Jackdaw/Bevy editor for
patterns: reflection-driven inspectors, dynamic plugin loading).

Suggested next step: two 3-day spikes in parallel — (a) Slint: runity renders
two viewports into `Rgba8Unorm` textures on Slint's wgpu 30 device + a minimal
dock splitter + headless screenshot test in CI; (b) GPUI Kit: same scene
through `surface()`/IOSurface on macOS. Plus one email to SixtyFPS on the
plugin-API licensing question.

---

## 4. Claims I could not verify

- Slint has no TreeView widget (absence inferred from docs/changelog search).
- Whether Slint's royalty-free "exposes the APIs" clause applies to an
  extensible editor whose plugins never see Slint types (legal reading).
- GPUI headless pixel screenshots; macOS IOSurface / Windows shared-handle
  handoff from wgpu into GPUI (plausible, untested).
- gpui-ce / chitin gpui-wgpu claims of wgpu on macOS/Windows (gpui-ce 0.2.2 on
  crates.io still depends on `metal` for macOS).
- iced 0.14 `table` row virtualization; iced production users other than COSMIC,
  Halloy, Sniffnet.
- Freya 0.4 details (release post rate-limited; used search snippets).
- Makepad's current release status beyond crates 1.0.0 and README wording.
- Fyrox UI's render backend details; Tiny Glade and Ambient editor UI stacks;
  Babylon.js editor stack.
- Compile-time numbers are dependency-graph sizes, not measured build times.
