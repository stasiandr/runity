# Level design tools: what level designers value and what is popular

Research date: 2026-09-25. Sources: web search + fetched pages. GitHub stars pulled via `gh api` on 2026-09-25.
Where a date is "n.d." the page gave no date. "(secondary)" = aggregator/tutorial site, not a primary source.

---

## 1. Blockout / greybox

### Takeaway
Every major engine now ships an in-editor blockout tool, and it is always the first thing level designers reach for. The tools that win are either *brush/CSG* (add a cube, subtract a hole: TrenchBroom, Hammer, RealtimeCSG, Godot CSG, Roblox unions) or *grid-push/pull* (Unreal CubeGrid). Poly-modelling-style tools (ProBuilder, UE Modeling Mode) are the default because they are built in, but designers repeatedly say they are slower for architectural blockout than brush tools. The common requirements are a coarse grid based on metrics, snapping (grid + vertex), world-aligned grid textures, and geometry that lives *inside the level* rather than as separate assets.

### Cited Findings
- **Unity ProBuilder** — Unity acquired ProCore (ProBuilder, ProGrids, Polybrush) and shipped ProBuilder built into Unity 2018.1 beta, free on all plans including Personal; ProBuilder was first released in 2012. [CG Channel, 2018-02](https://www.cgchannel.com/2018/02/unity-now-comes-with-probuilder-built-in-for-free/); [Game Developer: Unity acquires ProBuilder, 2018-02](https://www.gamedeveloper.com/design/unity-acquires-probuilder-level-design-tool-and-hires-its-creators); [GameFromScratch, 2018-02](https://gamefromscratch.com/unity-acquire-probuilder-release-tools-for-free/).
- Unity's own **level design e-book** (2023-11-08, by the level designers Stefan Horvath and Christo Nobbs) is built mostly around ProBuilder ("grey-box, prototype, and playtest… without 3D modeling software"), plus Terrain and Visual Scripting. [Unity blog, 2023-11-08](https://unity.com/blog/games/e-book-for-level-designers).
- The ProBuilder package repo has only 378 GitHub stars ([Unity-Technologies/com.unity.probuilder](https://github.com/Unity-Technologies/com.unity.probuilder), still active, last push 2026-09-23). The star count is low because people get it through Package Manager, not GitHub.
- **ProBuilder complaints**: in a 2020-05 Unity forum thread, users complain that you cannot delete edges/vertices and that it "falls apart" for environment work. CSG tools (SabreCSG, RealtimeCSG, Chisel) are recommended for architecture ("add a cube, and that's a building"). [Unity Discussions: ProBuilder vs SabreCSG, 2020-05](https://discussions.unity.com/t/probuilder-vs-sabrecsg-for-level-design/789065).
- **ProGrids deprecated** in Unity 2020.1 and replaced by the built-in grid. Users say the built-in grid lacks ProGrids features, for example grid scaling for ProBuilder element moves. [Unity Manual ProGrids 2020.1](https://docs.unity3d.com/2020.1/Documentation/Manual/com.unity.progrids.html); [Unity Discussions: Unity grids missing ProGrids features, ~2021](https://discussions.unity.com/t/unity-grids-missing-important-progrids-features/826722).
- **Unreal**: BSP brushes were the UE4 blockout standard. They still exist in UE5 but are considered deprecated. Modeling Mode is the replacement, and the recommended workflow is Modeling Mode with **Dynamic Mesh** output so the geometry lives in the level, not as Content Browser assets ("very similar to BSP brushes"). [World of Level Design, 2022-10-05](https://worldofleveldesign.com/categories/ue5/blockouts-in-ue5.php).
- **UE CubeGrid** (Modeling Mode tool): push/pull blocks on a repositionable grid. Tutorials pitch it as "build levels FAST". It shows up in many student blockout devlogs. [Epic docs, UE 5.8](https://dev.epicgames.com/documentation/en-us/unreal-engine/cubegrid-tool-in-unreal-engine); [itch devlog example, 2024-12](https://calsav43.itch.io/a1/devlog/851543/level-design-blockout-unreal-engine-5); [YouTube: Build levels FAST with Cube Grid](https://www.youtube.com/watch?v=jYdSDwkWMj8).
- **The Level Design Book** (the de-facto reference; n.d., continuously updated) recommends by engine: Unity → ProBuilder, or SabreCSG/RealtimeCSG for low-poly. Unreal → CubeGrid + Modeling Tools, Blueprint Splines. Quake-likes → TrenchBroom. Grid: "use a big coarse grid size based on metrics", e.g. Quake 32–64u, Unity 1–2 m, Unreal 50–128u. Playtest "with full player gravity, collision, and speed", not in editor fly mode. [LDB: Blockout](https://book.leveldesignbook.com/process/blockout).
- **TrenchBroom**: GPLv3, cross-platform Quake-family editor with 2,809 GitHub stars (checked 2026-09-25), actively developed (release 2026.2). Praised for 3D-first brush editing, "SketchUp-like handling of shared planes", and vertex editing. The LDB calls it "widely used across several modding communities as well as commercial projects". Weak points: scripting and choreography are painful, and it needs configs for non-Quake engines. [TrenchBroom repo](https://github.com/TrenchBroom/TrenchBroom); [TB manual 2026.2](https://trenchbroom.github.io/manual/latest/); [LDB: TrenchBroom](https://book.leveldesignbook.com/appendix/tools/trenchbroom).
- **TrenchBroom as a front-end for other engines**: the LDB lists bridges for Godot (FuncGodot, Qodot), Unity (Tremble, Qunity, Scopa) and Unreal (HammUEr). [LDB: Tools](https://book.leveldesignbook.com/appendix/tools). Stars: func_godot_plugin 858, Qodot 981 + 758 (two repos), godot-tbloader 260 (gh, 2026-09-25).
- **Godot CSG**: the official docs say "Level prototyping is one of the main uses of CSG". Limitations: no UV editing, and complex meshes are slow. Advice: convert to MeshInstance3D when done. [Godot docs: CSG](https://docs.godotengine.org/en/stable/tutorials/3d/csg_tools.html). In Godot 4.4 the CSG internals were replaced with the Manifold library for robustness. [secondary summary](https://www.mexc.com/en-GB/news/godot-4-4-dev-6-collisionshape3d-debug-color-customization-and-more/102584). The community plugin **Cyclops Level Builder** (brush editor for Godot) has 1,618 stars ([blackears/cyclopsLevelBuilder](https://github.com/blackears/cyclopsLevelBuilder)). A third-party "CSG Blockout" plugin adds pie menus, arrays and one-click bake ([GitHub](https://github.com/SuzukaDev/Godot-CSG-Blockout)). Both suggest built-in CSG is not enough on its own.
- **Hammer (Source 2)**: rebuilt from scratch for Half-Life: Alyx, with polygon mesh editing (extrude, bevel, fill) and **hotspot texturing** (automatic trim-sheet UV fitting), which Valve used heavily for modular bevelled geometry. [VDC: Hotspot texturing](https://developer.valvesoftware.com/wiki/Hotspot_texturing); [RoadToVR, 2020](https://roadtovr.com/half-life-alyx-source-2-modding-tools-hammer-vr/). Forum sentiment on legacy Hammer is mixed ("awfully complicated") ([Steam discussion](https://steamcommunity.com/discussions/forum/7/144512942752611120/?l=latam)).
- **Roblox Studio**: Parts plus Solid Modeling (union/negate/intersect CSG). CSG speed and robustness improvements reached Studio on 2025-07-14. [Roblox DevForum announcement, 2025](https://devforum.roblox.com/t/studio-solid-modeling-csg-performance-and-robustness-improvements/3256495/71); [Creator docs](https://create.roblox.com/docs/parts/solid-modeling).
- **Dreams** (Media Molecule): the CSG-based sculpt tool grew out of an internal game jam and became the core of the toolset. Clone-Repeat (e.g. perfect stairs) is a signature feature. The design avoids menus and sliders in favour of gestures. [Game Developer: How Media Molecule designed Dreams' toolset](https://www.gamedeveloper.com/design/how-media-molecule-designed-a-fun-and-robust-toolset-for-i-dreams-i-).
- **2D: Tiled vs LDtk** — Tiled has 12,913 stars and is the de-facto interchange format (TMX/TSX) that nearly every 2D engine reads. LDtk (by Sébastien Bénard, lead designer of Dead Cells, MIT) has 4,209 stars; it is entity- and field-driven with auto-layers. [mapeditor/tiled](https://github.com/mapeditor/tiled); [deepnight/ldtk](https://github.com/deepnight/ldtk); [ldtk.io](https://ldtk.io/); [Egmatic comparison, 2026 (secondary)](https://egmatic.com/blog/best-level-design-software). The LDB recommends an engine's built-in 2D editor first, then Tiled/LDtk. [LDB: Tools](https://book.leveldesignbook.com/appendix/tools).
- **Modular kits + snapping (AAA)**: Bethesda's GDC 2013 "Skyrim's Modular Approach to Level Design" (Joel Burgess, Nate Purkeypile) describes kits of snapping art pieces that let designers build many unique spaces "requiring minimal art support", with a tight art–design contract. [Burgess transcript, 2013-04](http://blog.joelburgess.com/2013/04/skyrims-modular-level-design-gdc-2013.html); [level-design.org mirror](https://level-design.org/?p=1643).
- **Metrics and scale tools**: the LDB recommends a "metrics zoo" or gym map, repeating world-aligned grid textures, a player-scale reference (usually a static player model or capsule), and in-editor jump-arc previews that run at edit time. [LDB: Metrics](https://book.leveldesignbook.com/process/blockout/metrics); [LDB: Quake metrics](https://book.leveldesignbook.com/process/blockout/metrics/quake). id's idStudio (Doom Eternal mod tools, 2024) ships tutorial maps that include "Scale Standards". [id Studio](https://idstudio.idsoftware.com/); [TweakTown, 2024](https://www.tweaktown.com/news/99834/id-softwares-new-mod-tools-for-doom-eternal-offer-fans-skyrim-levels-of-customization/index.html).
- **Blocktober**: a hashtag started 2017-10 by Naughty Dog's Michael Barclay to share blockouts. It is now an annual October event, which shows how central blockout is to how the discipline sees itself. [CGMagazine](https://www.cgmagonline.com/news/an-inside-look-at-game-design-with-blocktober/); [World of Level Design guide](https://www.worldofleveldesign.com/categories/level_design_tutorials/guide-to-blocktober.php).

### Inferences
- Built-in beats better. ProBuilder and UE Modeling Mode are the most *used* because they ship with the engine, while brush/CSG tools (TrenchBroom, RealtimeCSG, Hammer, Cyclops) are the most *loved* by level designers specifically.
- The recurring must-haves: a metric grid, snapping (grid, vertex, surface), non-destructive subtract/hole-punching, world-space grid materials, geometry stored in the level (not as assets), and one-click "bake to mesh" for the art pass.
- Popularity of TrenchBroom-to-engine bridges (Godot plugins at around 2k stars combined) shows that level designers will leave the engine to get a real brush editor.

### Gaps
- No survey that ranks blockout tools directly. Popularity is inferred from stars, built-in status and tutorial volume.
- No download or active-user numbers for ProBuilder or UE Modeling Mode.
- Could not fetch Polycount threads (403).

---

## 2. World structure and collaboration

### Takeaway
Big-world structure has converged on "one level made of many independently saved/streamed pieces". Unreal's World Partition + One File Per Actor (OFPA) + Data Layers + Level Instances is the most complete built-in answer. Unity relies on additive/multi-scene editing plus nested prefabs. OFPA is praised for killing map-file locking but criticised for opaque file explosions and editor performance.

### Cited Findings
- **OFPA**: each actor saves to its own external file, so editing actors does not require checking out the map. It is on by default with World Partition. [Epic docs: OFPA, UE 5.8](https://dev.epicgames.com/documentation/unreal-engine/one-file-per-actor-in-unreal-engine?lang=en-US); [Epic docs: World Partition](https://dev.epicgames.com/documentation/unreal-engine/world-partition-in-unreal-engine).
- Anchorpoint (2023-06-02): smaller commits and parallel work, but `__ExternalActors__` files are "hashed and a black box in Windows Explorer" and need OFPA-aware VCS tooling. [Anchorpoint blog](https://www.anchorpoint.app/blog/ue5-world-partition).
- **World Partition complaints**: delay before PIE starts after conversion, 8–10 fps editing a 4 km landscape, hard selection, landscapes always loaded. [Epic forums: Increased delay since WP conversion, 2022](https://forums.unrealengine.com/t/increased-delay-since-level-converted-to-world-partition/624052); [Epic forums: WP performance issue, 2023](https://forums.unrealengine.com/t/world-partition-performance-issue/1214386); [Polycount: 5.1 WP, HLODs and crashes](https://polycount.com/discussion/232455/5-11-world-partition-landscape-hlods-and-crashes).
- **Data Layers** replace UE4 Layers. They split gameplay and art in the editor, can be toggled at runtime for quest/progression states (e.g. holiday variants of a house), and reduce file checkouts. [Epic docs: Data Layers](https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition---data-layers-in-unreal-engine).
- **Level Instances / Packed Level Actors**: reusable sub-levels edited in context, where changes propagate to all instances. PLAs pack static meshes into ISM/HISM and can be nested (block → wall → building). [Epic docs: Level Instancing](https://dev.epicgames.com/documentation/en-us/unreal-engine/level-instancing-in-unreal-engine); [Epic forums: LI vs PLA vs ISM](https://forums.unrealengine.com/t/when-to-use-level-instance-packed-level-actor-or-ism-hism-in-ue5/2681508); [Unreal Directive: LI vs Packed Level Blueprint](https://unrealdirective.com/resources/unreal-decoder/systems/level-instance-vs-packed-level-blueprint/).
- **Unity**: multi-scene editing and additive loading let each discipline or area own a scene. Merge conflicts in the same scene still happen, and UnityYAMLMerge fixes are "manual and tedious". Nested prefabs (2018.3+) shrink the conflict surface. [Unity Manual: multi-scene](https://docs.unity3d.com/Manual/MultiSceneEditing.html); [Diversion blog](https://www.diversion.dev/blog/avoid-merge-conflicts-by-using-a-multi-scene-workflow-in-unity); [Unity Discussions: several people in one scene](https://discussions.unity.com/t/how-do-several-people-work-inside-the-same-scene-and-resolve-merging-conflicts-under-git/563709).
- **AAA in-house split**: at Naughty Dog, geometry was built in Maya, while gameplay metadata was placed in a custom tool, **Charter**. A server-based pipeline meant sub-30-second gameplay builds, and "a very fast pipeline… frees up your workflow tremendously". [Benson Russell, Game Developer, 2010-08-03](https://www.gamedeveloper.com/design/designing-combat-encounters-in-i-uncharted-2-i-).

### Inferences
- Fine-grained files per object (OFPA, prefabs, scenes) are now table stakes for team level design. The unsolved parts are human-readable file naming and editor performance at scale.
- "Instance a chunk of level and edit it in place" (Level Instances, nested prefabs, Hammer prefabs/instances) is the core modular-kit mechanic above the single-asset level.
- Separating geometry from gameplay layers (ND Charter vs Maya, Data Layers, Unity scenes per discipline) is a recurring AAA pattern.

### Gaps
- No quantitative adoption data for World Partition vs legacy sublevels.
- Did not find a primary source on Godot scene-instancing for large levels (it uses scene instancing natively; not researched here).

---

## 3. Level scripting and logic for designers

### Takeaway
Designers want *local, attachable* logic: triggers wired to targets (Hammer I/O, UEFN devices, Roblox scripts on parts, Dreams wires) rather than one global script per level. Unreal's Level Blueprint is widely used by beginners but experts actively discourage it in favour of reusable Blueprint actors. Unity's official Visual Scripting (ex-Bolt) is effectively in maintenance mode, with low adoption.

### Cited Findings
- **Hammer I/O**: every entity is driven by Outputs → target → Input (+ parameter, delay), with logic entities such as logic_relay, logic_timer, math_counter, logic_case and logic_branch, and special targets like `!activator`, `!caller`. [VDC: logic_playerproxy](https://developer.valvesoftware.com/wiki/Logic_playerproxy); [TopHATTwaffle I/O overview](https://www.tophattwaffle.com/hammer-tutorial-v2-series-12-input-and-output-overview/); [ModDB Hammer logic tutorial](https://www.moddb.com/groups/level-design-group/tutorials/hammer-logic-tutorial).
- **Unreal Level Blueprint**: a widely cited post, "Please, stop using Level Blueprints. Now." (Ula Kustra, Medium), argues for Blueprint actors instead. Forum consensus: actor BPs for doors, switches and props, and the Level BP only for truly level-wide one-offs. [Medium](https://ukustra.medium.com/please-stop-using-level-blueprints-now-691246c3ad41) (403 on fetch; title only); [Epic forum thread](https://forums.unrealengine.com/t/noob-question-should-i-place-all-actors-in-the-level-blueprints-so-that-i-can-reference-everything-instead-of-having-a-blueprint-for-each-actor/494972).
- **Unity Visual Scripting**: Bolt was acquired in 2020 and has been built in since 2021. A 2025 forum thread says feature development stopped years ago, the last official communication was 2022-11, and "the visual scripting community in Unity isn't particularly large or active". [Unity Discussions, 2025](https://discussions.unity.com/t/where-is-the-visual-scripting-community/1629898). No official usage percentage was found.
- **UEFN / Fortnite Creative devices**: placeable, configurable gameplay devices wired together, with Verse for custom devices. Around 260k UEFN islands and $722M in cumulative creator payouts by late 2025. Creator islands were 47% of Fortnite hours in 2026-05 (35% a year earlier). [Epic: Using devices](https://dev.epicgames.com/documentation/fortnite/using-devices-in-fortnite); [Creative Blok 2026 report (secondary)](https://thecreativeblok.com/uefn-fortnite-creative-community-report-2026/); [tech-insider (secondary)](https://tech-insider.org/uefn-setup-guide-2026/).
- **Roblox**: Studio plus Luau scripts on parts. The DevEx payout was around $1.5B in 2025 (up from $922.8M). RDC 2025 was held 2025-09-05/06. [Statista](https://www.statista.com/statistics/1376672/roblox-developer-payout/); [Roblox RDC 2025](https://about.roblox.com/newsroom/2025/09/roblox-rdc-2025).
- **Dreams**: gadgets and wires for logic. Creators move between art, logic and testing "in real time". [GamesRadar](https://www.gamesradar.com/the-next-generation-of-games-begins-with-dreams-how-media-molecule-is-empowering-new-gamemakers/); [TheGamer](https://www.thegamer.com/ps4-dreams-powerful-development-tool/).
- **Smart Objects (UE5)**: level-placed objects with slots and tags that AI and players query. Paired with StateTree as the data-driven, designer-facing AI path. [Epic docs: Smart Objects](https://dev.epicgames.com/documentation/unreal-engine/smart-objects-in-unreal-engine---overview?lang=en-US); [Unreal Fest 2023 talk](https://dev.epicgames.com/community/learning/talks-and-demos/mox7/unreal-engine-state-trees-and-smart-objects-data-driven-state-machine-workflows-for-open-world-ai-designs-unreal-fest-2023).
- **Naughty Dog**: first combat pass is "the most basic pass", with no fancy scripting, so failed ideas can be thrown away cheaply. [Russell, 2010](https://www.gamedeveloper.com/design/designing-combat-encounters-in-i-uncharted-2-i-).

### Inferences
- The shape designers prefer is "placeable object with exposed parameters + event wiring" (Hammer I/O ≈ UEFN devices ≈ Dreams gadgets ≈ Unreal actor BPs with exposed vars). One monolithic level script scales badly.
- General-purpose visual scripting (Unity VS) did not stick. Domain-specific, spatial logic (devices, entities, triggers) did.

### Gaps
- No hard adoption stats for Level Blueprint or Unity VS use.
- The LDB page on scripting (sequences/cutscenes) was not fetched.

---

## 4. Navigation, validation, playtest-from-here

### Takeaway
The single most cited iteration feature is instant playtesting from the current spot with real player physics (UE Play From Here / PIE / Simulate + Possess/Eject; Unity Play Mode made faster by skipping domain reload). The second is on-demand visualisation overlays: navmesh (P in Unreal), collision, scale references. Dedicated sightline/cover analysis tools are mostly in-house or custom.

### Cited Findings
- **Unreal PIE / Play From Here / Simulate**: right-click → Play From Here spawns the player at the cursor, with no save needed. SIE runs the world without a player, and Possess/Eject toggles between the two in one session "so that you can quickly iterate". [Epic docs: In-Editor Testing, UE 5.8](https://dev.epicgames.com/documentation/unreal-engine/ineditor-testing-play-and-simulate-in-unreal-engine?lang=en-US); [UE4.27 version](https://docs.unrealengine.com/4.27/en-US/BuildingWorlds/LevelEditor/InEditorTesting); [Designer 01 blockout tutorial](https://dev.epicgames.com/documentation/unreal-engine/designer-01-project-setup-and-level-blockout-in-unreal-engine?lang=en-US).
- **Unity Play Mode speed**: domain and scene reload grow with project size. "Configurable Enter Play Mode" (2019.3) can save "up to 50–90%" of wait time, but not all packages support it. [Unity blog, 2019-11-05](https://unity.com/blog/engine-platform/enter-play-mode-faster-in-unity-2019-3).
- **Navmesh visualisation**: in Unreal, pressing P shows the green navmesh, it also sits in Show → Navigation, and it auto-rebuilds when the Nav Mesh Bounds Volume changes. World-partitioned navmesh exists for large worlds. [Epic docs: Basic Navigation](https://dev.epicgames.com/documentation/en-us/unreal-engine/basic-navigation-in-unreal-engine); [Epic docs: WP Navmesh](https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partitioned-navigation-mesh).
- **"Walk it, don't fly it"**: the LDB says to evaluate blockouts with full gravity, collision and speed, and recommends edit-time jump-arc previews. [LDB: Blockout](https://book.leveldesignbook.com/process/blockout); [LDB: Metrics](https://book.leveldesignbook.com/process/blockout/metrics).
- **Fast build loop as a core AAA value**: ND had sub-30 s gameplay builds and a parameter-by-parameter tuning loop. [Russell, 2010](https://www.gamedeveloper.com/design/designing-combat-encounters-in-i-uncharted-2-i-).
- **Sightlines / flow**: David Shaver (Naughty Dog) and Robert Yang, GDC 2018 "Invisible Intuition: Blockmesh and Lighting Tips" — blockmesh + lighting to guide players, and blockout methods that "guarantee proper playtest feedback". [GDC Vault](https://gdcvault.com/play/1025360/Level-Design-Workshop-Invisible-Intuition); [Director's cut PDF](http://davidshaver.net/DShaver_Invisible_Intuition_DirectorsCut.pdf). The Source 2 guide says to focus blockout on "sightlines, choke points, and rotation paths". [Melikhov, 2026-06-25](https://melikhov.fr/blog/source-engine-2-level-design).
- **Iteration as process**: Joel Burgess (Bethesda), GDC 2014 "The Iterative Level Design Process" (Fallout 3/Skyrim). [SlideShare](https://www.slideshare.net/slideshow/3-10gdc2014-iterativeleveldesignprocess/34994256).
- **UEFN Live Edit**: editing a running session, highlighted as a key workflow in UEFN v41. [tech-insider (secondary)](https://tech-insider.org/uefn-setup-guide-2026/).
- **Dreams**: the edit ↔ play toggle is instant. [Game Developer](https://www.gamedeveloper.com/design/how-media-molecule-designed-a-fun-and-robust-toolset-for-i-dreams-i-).

### Inferences
- The ideal is a seamless edit ↔ play loop in which the level keeps its state, you start from anywhere, you can pause and eject to inspect, and you can edit live (UEFN Live Edit, Dreams). This matters more than any single geometry tool.
- Validation tools that level designers need, beyond navmesh: reachability/jump arcs, player-scale gizmos, and automatic checks (map check, missing collision, spawn validity). Only navmesh overlays are universal built-ins.

### Gaps
- No public detail on sightline or cover-analysis tools at AAA studios (Ubisoft, Guerrilla, IOI). Arkane (Void engine, a heavy id Tech 5 rewrite) and id in-house editor workflows were not documented beyond marketing.
- No primary source found for Valve's playtesting tooling in Alyx.

---

## 5. Talks and blog posts: what matters most

### Takeaway
Across GDC talks, the LDB, Naughty Dog, Bethesda and Media Molecule, the same priorities keep coming up: (1) iteration speed, meaning time from change to playing it; (2) cheap, disposable blockout that can be walked at real scale; (3) metrics and modular kits so blockout converts to art without redesign; (4) clean separation and handoff between design geometry and art.

### Cited Findings
- Naughty Dog: fast pipeline "frees up your workflow tremendously", and the first passes are deliberately basic. [Russell, 2010-08-03](https://www.gamedeveloper.com/design/designing-combat-encounters-in-i-uncharted-2-i-).
- Bethesda: modular kits give many unique spaces with minimal art support and set up the art–design contract. [Burgess GDC 2013](http://blog.joelburgess.com/2013/04/skyrims-modular-level-design-gdc-2013.html). Also the iterative level design process, [GDC 2014](https://www.slideshare.net/slideshow/3-10gdc2014-iterativeleveldesignprocess/34994256).
- Naughty Dog / Shaver GDC 2018: blockmesh and lighting guide the player. [GDC Vault](https://gdcvault.com/play/1025360/Level-Design-Workshop-Invisible-Intuition).
- LDB: keep blockouts "cheap" until ready for the expensive art pass, and set the grid by metrics. [LDB: Blockout](https://book.leveldesignbook.com/process/blockout).
- Media Molecule: toolset grew from game jams, with a playful, gesture-based design ("Stealth Create"). [Game Developer](https://www.gamedeveloper.com/design/how-media-molecule-designed-a-fun-and-robust-toolset-for-i-dreams-i-).
- Epic Unreal Fest talk on "Blockout and Asset Production in UE5". [Epic](https://dev.epicgames.com/community/learning/talks-and-demos/8k52/blockout-and-asset-production-in-unreal-engine-5).
- Level Design Lobby podcast (Max Pears) covers planning, iteration and tools. [Game Developer listing](https://www.gamedeveloper.com/design/level-design-lobby---podcast); Level Design Podcast LD029, Barclay on Blocktober. [Spotify](https://creators.spotify.com/pod/profile/leveldesign/episodes/LD029---Naughty-Dogs-Michael-Barclay-talks-Blocktober-emu14r).
- Engine context (GDC State of the Industry 2026): Unreal is the primary engine for 42% of developers, Unity 30%, Godot 11%. Unreal reaches 59% at AA and 47% at AAA studios. [GDC, 2026](https://gdconf.com/article/gdc-2026-state-of-the-game-industry-reveals-impact-of-layoffs-generative-ai-and-more/). About 51% of 2024 Steam releases were made with Unity, 28% Unreal and 5% Godot (secondary summary: [gamedevreports](https://gamedevreports.substack.com/p/video-game-insights-game-engines)).
- Mark Brown / GMTK: 1.69M subscribers (2025-11). Made the puzzle game Mind Over Magnet (released 2024-11-13) and documented it in the "Developing" series. No tooling-specific claims were found. [Wikipedia](https://en.wikipedia.org/wiki/Game_Maker's_Toolkit); [GMTK Substack](https://gmtk.substack.com/p/i-made-a-game-about-magnets).

### Inferences
- "Fast play-in-editor" and "blockout-to-art pass" are the two poles. Tools are judged by how quickly a designer can test an idea, and by how little of the blockout is thrown away when art arrives: kits, hotspot texturing, bake-to-mesh, geometry-stays-in-level.
- UGC platforms (Roblox, UEFN, Dreams) put the *entire* loop (build, script, play, publish) in one surface. They are also by far the largest level-design populations by headcount.

### Gaps
- Could not reach a primary Arkane or id level design talk describing their editors. Did not fetch the Level Design Lobby episodes or GMTK videos (video/audio).
- No structured survey of level designers' tool preferences exists that I could find.

---

## Ranking
Top level design tools and features, by importance to level designers and popularity:

1. **Instant play-in-editor from anywhere (UE Play From Here / PIE + Simulate/Possess-Eject; Unity fast Enter Play Mode; UEFN Live Edit; Dreams edit↔play)**: every source ranks iteration speed first, and the loop time defines the tool (ND, LDB, Unity 2019.3 work).
2. **Built-in blockout geometry (Unity ProBuilder, UE Modeling Mode/CubeGrid, Godot CSG, Roblox Parts/CSG)**: ships with every major engine and is the default first step. ProBuilder was bought and bundled to fill exactly this gap.
3. **Brush/CSG editing with subtract (TrenchBroom, Hammer, RealtimeCSG/Chisel, Cyclops)**: most loved by specialists. TrenchBroom (2.8k★) and Godot bridges (~2k★) show designers leave the engine to get it.
4. **Metric grid + snapping (grid, vertex, surface) + world-aligned grid materials**: universal LDB advice; the loss of ProGrids features drew complaints; the basis of modular kits.
5. **Modular kits / prefabs / Level Instances & Packed Level Actors**: the bridge from blockout to art (Skyrim GDC 2013); nested, edit-in-place instancing is standard in UE and Unity.
6. **Placeable logic with event wiring (Hammer I/O, UEFN devices, Roblox scripts on parts, UE actor BPs)**: how designers actually script. Monolithic Level Blueprints and general visual scripting (Unity VS) are discouraged or stagnant.
7. **One File Per Actor / multi-scene / nested prefabs for collaboration**: fixes map locking and conflicts. It is the default in UE World Partition, with complaints about opacity.
8. **World Partition streaming + Data Layers (and Unity additive scenes)**: essential for open worlds and state variants, but the top source of editor-performance complaints.
9. **Navmesh and debug visualisation overlays (UE "P", show flags)**: the universal built-in validation tool. Other checks (jump arcs, reachability) are usually custom.
10. **Scale/metrics references (player capsule/mannequin, metrics gym, jump-arc preview)**: cheap and always recommended (LDB, idStudio "Scale Standards"), but rarely a first-class engine feature.
11. **2D level editors (Tiled 12.9k★, LDtk 4.2k★)**: dominant outside engine editors for 2D; LDtk's entity/field model and auto-layers are the modern benchmark.
