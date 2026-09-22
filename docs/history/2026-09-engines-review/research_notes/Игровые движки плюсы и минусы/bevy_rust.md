# Bevy and the Rust gamedev ecosystem: strengths, weaknesses, and what maintainers admit (as of Sept 2026)

Timeline (for context): Bevy 0.14 (Jul 2024), Bevy Foundation became a 501(c)(3) (Sep 2024), 0.15 (Nov 2024), 0.16 (Apr 2025), Fifth Birthday post (Aug 10 2025), 0.17 (Sep 30 2025), 0.18 (Jan 13 2026), 0.19 (Jun 19 2026, 261 contributors / 1185 PRs), 0.19.1 (Aug 2026), Sixth Birthday post (Aug 10 2026) — [Bevy News](https://bevy.org/news/); [docs.rs bevy](https://docs.rs/crate/bevy/latest). Bevy is still pre-1.0 in 2026.

## 1. What developers and maintainers praise

### Takeaway
People praise the ECS and its scheduling, Rust's ergonomics and reliability compared with C++, the open, community-driven development (hundreds of contributors per release), and fast progress on rendering and performance. Hot reloading via hot patching and BSN were added in 2025–2026.

### Cited Findings
- Tiny Glade dev Tomasz Stachowiak on Rust: "It's very ergonomic, doesn't crash in your face, and just feels like a breath of fresh air compared to something like C++." Bevy was chosen because "it was the easiest thing to jump into, and we're still using a modified version of Bevy to this day" (May 2024) — [80.lv interview](https://80.lv/articles/exclusive-tiny-glade-developers-discuss-bevy-proceduralism-publishers-cozy-games)
- Tiny Glade dev on ECS: "ECS has its uses, as do other object models. It fits somewhat well for a lot of our stuff, a bit less well for things that require custom spatial queries or complex hierarchical data structures. We do dig the system part of it a lot, the flexible scheduling & messages." — [GitHub Discussion #18689](https://github.com/bevyengine/bevy/discussions/18689)
- Community scale (Fifth Birthday, Aug 2025): contributors rose from 1,027 to 1,291, GitHub stars from 34,537 to 40,900, and downloads from 1.49M to 2.75M in one year. A "Working Group" model lets "coalitions … form and work to be accomplished relatively unimpeded" — [Bevy's Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
- Each release has a very large contributor count: 0.15 had 294 contributors / 1217 PRs, 0.16 had 261 / 1244, 0.17 had 278 / 1311, 0.18 had 174 / 659, and 0.19 had 261 / 1185 — [Bevy News](https://bevy.org/news/)
- Performance: GPU-driven rendering in 0.16 gave "3x or more" gains in multi-object scenes. Solari real-time raytracing is experimental. Hot reloading was added via "subsecond" hot patching — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
- In 0.19 the many_cubes benchmark went from 49.47 ms to 18.77 ms per frame with culling, and from 93.1 ms to 41.2 ms without culling. 0.19 also added contact shadows, PBR screen-space reflections and rectangular area lights — [Bevy 0.19](https://bevy.org/news/bevy-0-19/)
- The Sixth Birthday (Aug 2026) lists as achievements: BSN, upstream core widgets, Solari, GPU-driven rendering of "millions of entities", virtual geometry with BVH culling, DLSS, WESL shaders, and automated metrics that run "on every Bevy commit, tracking Bevy's runtime performance, compile times, and binary sizes" — [Bevy's Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)
- BSN in practice: after porting to BSN, the Exofactory developer reported saving 7,364 lines of code — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)
- Rendering maintainer JMS55 calls development "the smoothest it's ever been" thanks to working groups and incremental release notes — [JMS55 blog, Sep 2025](https://jms55.github.io/posts/2025-09-03-bevy-fifth-birthday/)
- Bevy maintainer pcwalton, arguing against the LogLog pessimism on HN: engines take a long time to build. He noted "0.13 doesn't even have support for animation blending yet (I landed that for 0.14)" and pointed to shipped titles such as Tunnet — [HN thread](https://news.ycombinator.com/item?id=40172033)
- Even LogLog, the harshest critic, credits Rust for easy refactoring and generational arenas — [LogLog Games](https://loglog.games/blog/leaving-rust-gamedev/)

### Inferences
- The things praised are mostly the engine's architecture and technology (ECS, scheduler, renderer, open development). Content-production workflow is rarely praised. The typical profile of a successful Bevy user is a strong programmer who takes the ECS and replaces or extends other parts, as Tiny Glade did.

### Gaps
- I found no hard, current (2026) survey numbers on developer satisfaction with Bevy.

## 2. Recurring complaints (editor, breaking changes, compile times, immaturity, docs, borrow checker)

### Takeaway
As of Sept 2026 the main complaints still stand. Bevy has no official editor. Breaking releases come roughly every 3–5 months (now about quarterly) and ecosystem crates lag behind them. Several subsystems remain immature: UI (improving fast), animation, asset processing, materials and rendering docs. Rust's strictness also slows the iteration loop for gameplay. Some specific items have been fixed or partly fixed: hot reloading, text input in UI, BSN scenes, and animation blending.

### Cited Findings
**Editor (still unresolved)**
- Fifth Birthday (Aug 2025): "Bevy still has no editor (gasp!)". What exists is groundwork: Figma designs, core widgets, the Bevy Feathers UI library (0.17), the Remote Protocol, and BSN — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
- Sixth Birthday (Aug 2026): still no upstream editor MVP. The "Upstream Editor MVP" is now called the overriding priority for 2026–27. The community prototype Jackdaw shows scene editing, ECS inspection and plugins — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)
- Jackdaw is a third-party 3D level editor for Bevy 0.19, built as a Bevy plugin. It has brush/CSG geometry, a material browser, terrain, a .jsn scene format and undo/redo, and is "very early in development" — [Jackdaw repo](https://github.com/jbuehler23/jackdaw); [Jackdaw docs](https://jbuehler23.github.io/jackdaw/). The official prototype experiments live in [bevy_editor_prototypes](https://github.com/bevyengine/bevy_editor_prototypes)
- Bevy 0.17 introduced Feathers, a widget set "that will be used to build the upcoming Bevy Editor" — [GameFromScratch 0.17](https://gamefromscratch.com/bevy-0-17-released/)
- CONFLICT: a third-party blog says that in 0.18 "the editor preview finally landed" — [StraySpark](https://www.strayspark.studio/blog/bevy-rust-game-engine-2026-indie-guide). Cart's official Sixth Birthday post (Aug 2026) contradicts this: there is no upstream MVP — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/). Treat the StraySpark claim as unreliable.
- JMS55: "Editor absence continues to be a big, big hole for Bevy… it's hard to recommend Bevy for UI-heavy games and apps" — [JMS55](https://jms55.github.io/posts/2025-09-03-bevy-fifth-birthday/)

**Breaking changes and migration pain (ongoing)**
- A breaking release lands about every 3 months. Migration guides are provided, but migrations "are not guaranteed to always be easy" — [docs.rs bevy](https://docs.rs/crate/bevy/latest)
- 0.19 contains large internal restructuring: the render graph moved to ECS schedules, and resources are now stored as components on singleton entities. A 0.18→0.19 migration guide is required — [Bevy 0.19](https://bevy.org/news/bevy-0-19/)
- GitHub Discussion "I am very sorry but going on like that is impossible" (user neshume, Nov 14 2025): a long-time user quit Bevy for games because "I always find myself chasing the next version, never get to stabilize the games, it always breaks". Core things like colors, windows and components change too often. Other contributors agreed — [Discussion #21838](https://github.com/bevyengine/bevy/discussions/21838)
- Among community reports, one developer with 3–4 years on Bevy said each migration takes "20 minutes to a couple hours", and that the most painful part is waiting for ecosystem crates to update. This was found via search summary only; I did not verify the primary quote. — [search context; Discussion #21838](https://github.com/bevyengine/bevy/discussions/21838)

**Compile times, iteration and hot reloading (partly improved)**
- LogLog (Apr 2024): overall compile times improved, but procedural macros "destroy incremental builds". The author built comfy-ldtk only to isolate serde monomorphization, which otherwise cost 8 seconds. Hot reloading via hot-lib-reloader was unreliable — [LogLog Games](https://loglog.games/blog/leaving-rust-gamedev/)
- Status: hot reloading via subsecond hot patching landed in the 2024–25 cycle — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/). Compile times are now tracked on every commit — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)

**UI (historically a major weakness, improving fast)**
- Fifth Birthday says UI moved "at a rapid pace" after earlier stagnation. Additions include rounded corners, box shadows, picking, scrolling and required components — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
- 0.19: "Bevy UI _finally_ has upstream support for text entry via the new `EditableText` component", with IME, selection and multiline support — [Bevy 0.19](https://bevy.org/news/bevy-0-19/)
- JMS55 (Sep 2025): "no third-party crate (including my own bevy_dioxus) has proven out a good high-level API for declaring and updating UI trees." — [JMS55](https://jms55.github.io/posts/2025-09-03-bevy-fifth-birthday/)
- LogLog (2024) calls Rust in-game GUI a "catastrophe": the ecosystem focuses on data binding and reactivity, while games need sprites, animation, shaders and tweakability — [LogLog](https://loglog.games/blog/leaving-rust-gamedev/)

**Other immaturity (asset pipeline, animation, materials, docs) per maintainer JMS55 (Sep 2025)**
- "Hard to say Bevy is production ready when the only texture compressor it has is an outdated version of BasisU." — [JMS55](https://jms55.github.io/posts/2025-09-03-bevy-fifth-birthday/)
- Animation is "lacking", with a "clunky" API — [JMS55](https://jms55.github.io/posts/2025-09-03-bevy-fifth-birthday/)
- Custom materials are "servicable, but not enjoyable" and need workarounds such as MeshTag and ShaderStorageBuffer — [JMS55](https://jms55.github.io/posts/2025-09-03-bevy-fifth-birthday/)
- "Rendering in particular needs a lot more docs" — [JMS55](https://jms55.github.io/posts/2025-09-03-bevy-fifth-birthday/)
- The 2026–27 roadmap includes Assets as Entities, a .bsn asset loader, and shader/material workflows. These admit that the asset and material pipelines are still unfinished — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)

**Rust borrow checker and gameplay iteration**
- LogLog: the borrow checker forces refactors at bad moments: "I don't want 'better code', I want 'game faster'". Global state needs `Lazy<AtomicRefCell<T>>` ceremony. Overlapping ECS queries or RefCell double borrows cause runtime crashes after refactors. The orphan rule hurts application code. Procedural macros are no substitute for C#-style reflection — [LogLog](https://loglog.games/blog/leaving-rust-gamedev/)
- HN commenter johnnyanmac: "Rust's biggest strength is correctness. But games aren't mission critical, and gamers are very tolerant towards bugs." Commenter Animats: "The people using C# and Unity on the same problem are making much faster progress." — [HN](https://news.ycombinator.com/item?id=40172033)

### Inferences
- Complaints are moving from "missing basics" (2023–24: no animation blending, no text input, no hot reloading) to "no editor, no stability, pipelines unfinished" (2025–26). The editor has been promised for more than 3 years and is still absent, which makes it the most damaging item for Bevy's reputation.
- A quarterly breaking-release cadence combined with third-party crate lag is structurally bad for projects that take several years. This is the main risk for any commercial project on Bevy.

### Gaps
- I found no measured benchmarks of Bevy clean or incremental compile times for 2026. Only the existence of automated tracking is confirmed.
- I did not verify r/bevy or r/rust_gamedev threads directly. Reddit was not fetched.

## 3. LogLog Games "Leaving Rust gamedev after 3 years" and discussion

### Takeaway
This April 2024 post is the key critical text on Rust gamedev. Its core thesis: Rust optimizes for correctness and maintainability, while indie gamedev needs iteration speed. The Rust and Bevy ecosystem "lives on hype, rather than shipped projects." Several of its technical points, such as hot reloading and UI text input, have since been partly addressed. The cultural and language-level points remain valid.

### Cited Findings
- Author and context: LogLog Games, a 2-person team, published the post Apr 26 2024. They wrote 100k+ lines of Rust over 3+ years, used Bevy, Macroquad and their own engine (Comfy), and shipped games on Steam — [LogLog](https://loglog.games/blog/leaving-rust-gamedev/)
- Main arguments:
  - Constant borrow-checker refactors.
  - ECS treated as dogma, while generational arenas (e.g. thunderdome) give most of the benefit. Real games need "fat components".
  - Over-generalized systems hurt game design.
  - Hot reloading matters (Tomorrow Corporation is the gold standard).
  - GUI is a "catastrophe".
  - Proc macros slow compiles.
  - Global state is painful.
  - Dynamic borrow panics.
  - There is no reflection.
  - The orphan rule.
  - Community positivity shields it from criticism.
  - Bevy "dominates despite undelivered promises (editor, scalable UI)", while practical libraries such as Macroquad get dismissed for unsafe globals.
  - rapier is praised but "quite unstable" in practice.
  - The community is "overwhelmingly focused on tech, to the point where the 'game' part of game development is secondary."
  — [LogLog](https://loglog.games/blog/leaving-rust-gamedev/)
- The author is leaving Rust but not gamedev — [HN](https://news.ycombinator.com/item?id=40172033)
- HN discussion themes:
  - Compile times and the borrow checker learning curve.
  - Parallels to how long C++ took to be adopted in gamedev.
  - "I don't know a single gamedev who's fond of 'modern C++'" (flohofwoe).
  - Bevy maintainer pcwalton argued that Bevy is young and cited Tunnet as a shipped game.
  - Several commenters said the real mistake is using a systems language for rapid gameplay scripting.
  — [HN item 40172033](https://news.ycombinator.com/item?id=40172033)
- The post was also widely discussed elsewhere — [Devtalk](https://forum.devtalk.com/t/lessons-learned-after-3-years-of-fulltime-rust-game-development-and-why-were-leaving-rust-behind/153139); [YouTube reaction](https://www.youtube.com/watch?v=trnvX4j91Sg)
- Status of the points as of 2026:
  - Hot reloading: partly resolved by subsecond hot patching — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
  - UI text input: added in 0.19 — [Bevy 0.19](https://bevy.org/news/bevy-0-19/)
  - Editor: still missing — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)
  - Borrow checker, orphan rule and reflection: language-level, not changed by Bevy (inference)

### Inferences
- The article remains the most-cited "anti" source, so a Russian report should present it as a 2024 historical milestone and separate the language-level criticism, which is durable, from the ecosystem criticism, which is partly outdated.

### Gaps
- I did not fetch Reddit (r/rust, r/gamedev) threads on the article. I could not reliably get comment counts or points for the HN thread; one tool summary gave a "1,484 points" figure attached to a comment, which looks wrong and should not be used.

## 4. Maintainers' own admissions (Cart's birthday posts, funding, BSN, editor, process)

### Takeaway
Cart (Carter Anderson) is unusually candid. He admits that Bevy is "drastically under-funded", that staff are paid about 50% below market, that BSN took 2 years because of his perfectionism and secrecy, that the editor still does not exist, and that governance mistakes happened (the Goals rollout and an overly strict AI policy). In late 2025 he publicly accepted blame for letting contributors' "arbitrary whims" direct scarce resources.

### Cited Findings
- Fifth Birthday (Aug 10 2025):
  - "we are drastically under-funded for our ambitions". Both full-time employees (Cart and Alice Cecile) take an "over 50% pay cut". François Mockers works part-time. The project is "severely under-staffed".
  - On BSN taking 2 years from proposal to draft: "keeping my cards too close to my chest prevents collaboration from occurring."
  — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
- Sixth Birthday (Aug 10 2026):
  - The Foundation is still "drastically underfunded for our ambitions", with staff at about 54% below baseline compensation. No revenue services (asset store or consulting) were launched.
  - The Project Goals rollout "caused some (well-founded) concern and anxiety" and has since been loosened.
  - The strict no-AI contribution policy created "toxic witch hunts, incentivizing lying to maintainers", and a new policy is being drafted.
  - There is still no editor MVP.
  — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)
- Cart in Discussion #21838 (Dec 18 2025):
  - "I do think I suffer from perfectionist tendencies". He said he spent too much time vetting and reworking community PRs and that the project is "incredibly resource constrained".
  - "I've let the arbitrary whims of individual contributors dictate how those resources are spent. This needs to change."
  - He plans a process that lets leadership say "no" earlier in 2026.
  — [Discussion #21838](https://github.com/bevyengine/bevy/discussions/21838)
- Funding and structure:
  - The Bevy Foundation became a 501(c)(3) public charity (Sep 25 2024) — [Bevy News](https://bevy.org/news/)
  - Alice Cecile joined as a full-time paid developer (Sep 2024) — [Bevy News](https://bevy.org/news/)
- BSN (Bevy Scene Notation) timeline:
  - Targeted for partial landing in 0.18 — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
  - Actually shipped as the headline feature of 0.19 (Jun 2026): the `bsn!` macro, with scenes that are "composable, patchable, and dependency aware" — [Bevy 0.19](https://bevy.org/news/bevy-0-19/)
  - A file-based .bsn asset loader is still future work — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)
- Required Components landed in 0.15 (Nov 2024). This was a major ergonomics change that replaced Bundles — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
- Release cadence moved from about 5 months (0.15 to 0.16) to roughly quarterly (0.17, 0.18, 0.19) — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/); [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)
- The 2026–27 priorities are: Assets as Entities, the .bsn loader, the Upstream Editor MVP (the "overriding priority"), shader/material workflows, and a reactivity ecosystem — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)

### Inferences
- Bevy's bottleneck is organizational rather than technical: about 2 underpaid full-time staff and a lead-architect review bottleneck, while hundreds of volunteers contribute. This explains why large coordinated features (editor, BSN) take years while self-contained features (rendering) move fast.

### Gaps
- The Fourth Birthday (Aug 2024) post was not fetched in detail. Exact current Foundation revenue and donation numbers were not found.

## 5. Notable shipped games on Bevy and their feedback

### Takeaway
The biggest Bevy success, Tiny Glade (616k copies sold in under a month), uses Bevy's ECS with a custom Vulkan renderer, not stock Bevy. Other shipped titles are small indies. Enterprise and non-game use (spatial visualization, hardware testing) is growing.

### Cited Findings
- Tiny Glade (Pounce Light, 2 devs, Sep 23 2024):
  - 616,000 copies sold in less than a month, 1,375,441 wishlists at launch, $15 price, 97% positive reviews, about 2 years of development — [GameDiscoverCo](https://newsletter.gamediscover.co/p/how-tiny-glade-built-its-way-to-600k)
  - 10k+ concurrent players at launch — [WN Hub](https://wnhub.io/news/stores-and-publishing/item-45586)
  - Tech: "Initially, Anastasia's prototype was using Bevy for everything, including rendering, but eventually, our needs outscaled what Bevy could provide." They moved to a custom Vulkan renderer to control "the whole pipeline", and run a modified Bevy — [80.lv](https://80.lv/articles/exclusive-tiny-glade-developers-discuss-bevy-proceduralism-publishers-cozy-games)
- Titles named by Bevy leadership:
  - 2025: Tiny Glade, Death Trip, LongStory 2, Greenfeet Haven (released), plus Jarl, Polders, Exofactory and Sunny Shores. Companies: Foresight (spatial data) and Nominal (hardware testing) — [Fifth Birthday](https://bevy.org/news/bevys-fifth-birthday/)
  - 2026 Steam releases: Toroban, Exofactory, Gob Johnson's Downhill Marmalade, Simulo, Insanio, Court Wizard, Wee Boats. In development: Jarl, Polders, Unhaunter. Enterprise: Foresight, Nominal, Design Once — [Sixth Birthday](https://bevy.org/news/bevys-sixth-birthday/)
- Tunnet was cited by pcwalton as a shipped Bevy game (2024) — [HN](https://news.ycombinator.com/item?id=40172033)
- LogLog's own Steam games were built partly with Bevy, and the team left Rust — [LogLog](https://loglog.games/blog/leaving-rust-gamedev/)

### Inferences
- Bevy has no "flagship" game that uses the whole stock engine at commercial scale. Tiny Glade shows "Bevy ECS plus your own renderer, written by a veteran rendering engineer", which is a different use case from Unity or Godot users. Shipped Bevy games are mostly small indies, and most of them are code-heavy (sim, puzzle, builder).

### Gaps
- I found no sales data or postmortems for Death Trip, Exofactory, Toroban and others. The Tiny Glade devs gave no detailed public complaints about Bevy upgrades in the sources found.

## 6. Rust gamedev ecosystem: arewegameyet, Fyrox, Macroquad, comparisons

### Takeaway
Arewegameyet still answers "Almost". Fyrox is the Rust option with a Unity-like editor and reached a stable 1.0 in March 2026. Macroquad (simple, immediate-mode, with global state) is the pragmatic choice for small 2D games and is favored by critics such as LogLog. Bevy dominates mindshare.

### Cited Findings
- arewegameyet.rs: "Almost. We have the blocks, bring your own glue." "The ecosystem is still very young". It lists 59 crates under game engines — [arewegameyet.rs](https://arewegameyet.rs/)
- Fyrox:
  - Described as "a modern game engine written in Rust… using native editor; it is like Unity, but in Rust."
  - 1.0.0 was the first stable release after about 7 years of development. Features: editor with scene/UI, animation (auto-keying), ABSM, terrain, prefabs and materials; a CLI for export and CI; prefab hot reload; dynamic editor plugins.
  - The developers note the small team size means bugs may exist.
  — [Fyrox 1.0.0 blog](https://fyrox.rs/blog/post/fyrox-game-engine-1-0-0/); [RC1](https://fyrox.rs/blog/post/fyrox-game-engine-1-0-0-rc-1/)
  - Release date: March 2026, timed to the engine's 7th birthday on Mar 19 2026 — [GameFromScratch](https://gamefromscratch.com/after-7-years-of-development-fyrox-1-0-is-here/); [E-Ink News, 2026-03-25](https://news.e-ink.me/en/archive/2026-03-25/article/fyrox-1-0-0). One fetch-tool summary of the Fyrox post said "2024", which is almost certainly an error.
  - I found no notable commercial Fyrox games in the 1.0 announcement — [Fyrox 1.0.0](https://fyrox.rs/blog/post/fyrox-game-engine-1-0-0/)
- Macroquad: LogLog says the community dismisses it for using "unsafe" global state, even though it is practical for shipping — [LogLog](https://loglog.games/blog/leaving-rust-gamedev/)
- rapier (physics) is widely praised but "quite unstable" in practice according to LogLog — [LogLog](https://loglog.games/blog/leaving-rust-gamedev/)
- Amethyst, historically a major Rust engine, appears only as a legacy reference on arewegameyet. It was discontinued years ago; this is my knowledge, not verified in this session — [arewegameyet.rs](https://arewegameyet.rs/)

### Inferences
- Rust gamedev in 2026 splits into three niches:
  - Bevy: the largest community, a code-first ECS with no editor and an unstable API.
  - Fyrox: stable 1.0 with an editor, but a small team and little shipped proof.
  - Lightweight frameworks such as Macroquad and ggez, for small 2D games.
- None of them yet matches Godot, Unity or Unreal in content tooling or console support.

### Gaps
- I did not verify Macroquad's or ggez's current (2026) maintenance status, or Bevy's console support. Neither was researched in this session.
- I found no 2026 comparative survey of Rust engine usage.
