# Content-authoring tools for designers and artists (beyond environment art)

Research date: 2026-09-25. Engines covered: Unity, Unreal, Godot, in-house engines (CDPR REDengine, Larian Divinity Engine, Obsidian OEI Tools, Bethesda Creation Kit).
Star counts were taken from the GitHub API on 2026-09-25. Asset Store numbers were read from the store pages on 2026-09-25.
"Cited" means the fact comes from the linked page. "Inference" means it is my own synthesis.

---

## 1. Narrative, dialogue and quests

### Takeaway
There are two kinds of narrative tool, and both are popular:
- **Text-first scripting languages** (Ink, Yarn Spinner, Godot Dialogue Manager). These are free and open source. The script is plain text, so it diffs in git, merges, and can be searched with grep.
- **Node-graph databases** (articy:draft, Pixel Crushers Dialogue System, Dialogic, and the proprietary editors in large engines).

In-house RPG studios (CDPR, Larian, Obsidian, Bethesda) all built their own graph-based conversation and quest editors. Those editors do much more than dialogue: they also hold quest state, global variables, VO scripts and the localization string database. Professionals ask for the same few things every time:
- stable line IDs;
- export to VO recording scripts and localization (PO/XLSX);
- source-control friendliness;
- a flowchart view to see branching at a glance;
- play-as-you-write iteration.

### Cited Findings
- **Ink** (inkle). The GitHub repo `inkle/ink` has about 4.9k stars and `ink-unity-integration` about 0.7k (GitHub API, 2026-09-25). The design principle is that "text comes first, code and logic are inserted within". Inky gives "play as you write", and ink is positioned as middleware that slots into any engine. It is proven in inkle's own games (80 Days, Sorcery, Heaven's Vault; Expelled! shipped 2025-03-12). https://www.inklestudios.com/ink/ ; https://en.wikipedia.org/wiki/Expelled! ; Ink for Unity 2.0 requires Unity 2022.3+: https://github.com/inkle/ink-unity-integration
- **Dink** (wildwinter, MIT) is a third-party production pipeline built on Ink. It exists because raw Ink lacks one. It adds `#id:` line tags, Excel VO recording scripts (speaker, direction, recording status), PO/POT and XLSX localization exports, draft/final writing status, and scratch/TTS audio status per scene and character. https://github.com/wildwinter/dink
- **Yarn Spinner.** The core repo has about 2.8k stars and the Unity repo about 0.8k (2026-09-25). YS3 shipped in May 2025 with once-only content, enums, smart variables, and storylets/saliency. 3.1 added async runners and Text Animator integration. Shipped titles include DREDGE, A Short Hike, Night in the Woods, Lost in Random, Venba, Little Kitty Big City, Rift of the NecroDancer, Unbeatable, Frog Detective 2/3 and Escape Academy. https://yarnspinner.dev/docs/readme/02-ys3 ; https://yarnspinner.dev/showcase/ ; https://yarnspinner.dev/blog/yarn-spinner-3-1-release/
- **articy:draft X.** A standalone desktop narrative database: flow graphs, entities, world-building, and exports to Unity and Unreal. Used on Disco Elysium and Hogwarts Legacy. Disco Elysium's lead writer credited it for how wordy the game became, and PC Gamer reported the text volume pushed the software to its limits. There is a free tier. https://www.articy.com/en/showcase/disco-elysium/ ; https://www.pcgamer.com/games/rpg/disco-elysium-had-so-much-text-it-broke-the-branching-narrative-software-we-were-writing-too-much/ ; https://www.articy.com/en/articydraft/free/
- **Pixel Crushers Dialogue System for Unity.** 848 reviews and 11,994 favorites; $95 list price. Version 2.2.74 released 2026-09-12. It covers dialogue trees, quests and localization, and imports from articy, Ink, Yarn and others. https://assetstore.unity.com/packages/tools/behavior-ai/dialogue-system-for-unity-11672
- **Godot.** Dialogic has about 6.0k stars and Dialogic 2 requires Godot 4.5+. It is a timeline-and-character editor aimed at visual novels and RPGs. Nathan Hoad's Godot Dialogue Manager is a text-first script format with about 3.9k stars. Both figures are from 2026-09-25. https://github.com/dialogic-godot/dialogic ; https://docs.dialogic.pro/getting-started.html ; https://github.com/nathanhoad/godot_dialogue_manager
- **Twine.** twinejs has about 2.9k stars, and itch.io lists 22,761 games "made with Twine" (2026-09-25). It is mainly used for prototyping and IF, not in-engine use. https://itch.io/games/made-with-twine
- **CDPR (REDengine).** GDC 2024 talk by Sarah Gruemmer: "Where Story and Tech Meet: Cyberpunk's Quest Editor in Action". It covers REDengine 4's narrative systems and the "advantages and disadvantages of complex visual scripting tools" across tool iterations. The Witcher 3 REDkit (2024) ships reworked REDengine 3 tools, including a Quest Debugger. https://www.gdcvault.com/play/1034181/ ; https://cdprojektred.atlassian.net/wiki/spaces/W3REDkit/pages/6328642/TOOL+Quests+Debbuger ; see also GDC 2016 "Behind the Scenes of Cinematic Dialogues in The Witcher 3": https://www.gdcvault.com/play/1022988/
- **Larian (Divinity Engine).** The Dialog Editor is a visual tool "for writers and scripters alike". The Story Editor edits Osiris scripts, which hold quest and world logic. https://docs.larian.game/Dialog_editor ; https://docs.larian.game/Story_editor . BG3 has about 174 hours of cinematics (GDC 2024 talk by Jason Latino, "Larian Cinematics: A Top-Down Look at Our Bottom-Up Approach"). https://gdcvault.com/play/1034616/ ; https://gamerant.com/baldurs-gate-3-cutscenes/
- **Obsidian (OEI Tools).** The editor uses a flowchart layout (Visio-like) rather than a tree. Trees were adequate for "75%" of dialogues, but flowcharts won for the complex ones. The same tool also sets up quests, global variables, AI behavior trees and game data. It is integrated with Perforce and the string DB, and exports XML/JSON, VO actor scripts, screenplay documents and flowchart PDFs. Source: Josh Sawyer, Tumblr, 2018. https://www.tumblr.com/jesawyer/175082312536/im-curious-as-to-what-the-conversation-editor . In a GDC 2019 talk (Patel and Szymczyk), Obsidian said a good editor must "shape and view the structure… seamlessly write and edit text… integrate dialogue with game scripting". https://gdcvault.com/play/1026384/
- **Bethesda Creation Kit.** A stripped-down version of the internal editor. Quests, dialogue and scenes are all edited in one tool. The Starfield CK reception from modders was mixed: it arrived late, and paid "Creations" drew controversy (2024). https://80.lv/articles/expect-more-mods-with-bethesda-s-starfield-creation-kit ; https://www.techdirt.com/2024/06/20/bethesda-reignites-the-paid-mods-controversey-with-starfield-creation-kit/

### Inferences
- The "must-haves" converge: stable line IDs, VO/loc export, a flowchart view, variables and conditions, and quick in-context playtest. Obsidian's list (2018) and Dink's feature set (2020s) are almost identical.
- Text-first languages win in indie and mid-size teams because they merge well. Graph databases win where writers are not programmers and the content is huge (Disco Elysium, Hogwarts Legacy). Big RPG studios end up with a hybrid: a graph editor whose nodes contain rich text and script conditions.
- In in-house engines, the dialogue editor tends to grow into the main designer tool (quests, AI, data). That is a sign the narrative tool is really a general "designer database + graph" tool.

### Gaps
- There is no public usage share of Ink vs Yarn vs articy. The itch.io "made-with" tags for ink and Yarn did not resolve. GDC Vault talk bodies are paywalled, so the internals of the CDPR and Larian tools are known only from their abstracts.

---

## 2. Cinematics and sequencing

### Takeaway
UE Sequencer is the clear leader. It keeps getting investment: Take Recorder, Movie Render Queue/Graph, Mocap Manager, and the Cinematic Assembly Toolset in 5.6. Unity Timeline is widely used but is seen as barebones and neglected, and Unity has wound down its adjacent Sequences package. Large RPG studios (CDPR, Larian) build dialogue-cinematic tools on top of the dialogue graph, because hand-keyed cutscenes cannot scale to more than 100 hours of branching dialogue.

### Cited Findings
- UE 5.6 (June 2025) added the Cinematic Assembly Toolset (CAT, experimental) for shot and pipeline management, used alongside Take Recorder and Movie Render Queue. It also added Quick Render, Mocap Manager, and Sequencer audio scrubbing. https://www.unrealengine.com/news/unreal-engine-5-6-is-now-available ; https://dev.epicgames.com/documentation/unreal-engine/take-recorder-in-unreal-engine
- Take Recorder records gameplay, live performance and mocap into Sequencer takes. https://dev.epicgames.com/documentation/unreal-engine/take-recorder-in-unreal-engine
- Forum thread "Unity Timeline feels years behind other engines" (2026-04-28). Complaints: no keyframe snapping; keys have to be managed in a separate Animation window; weak multi-select; poor documentation for binding and gameplay integration. There was no staff reply. A community reply said it is "a barebones tool… not comparable to what Unreal has". https://discussions.unity.com/t/unity-timeline-feels-years-behind-other-engines-missing-essential-features-and-many-bottlenecks/1718213
- Unity's Sequences package (shot and sequence management on top of Timeline) was "prepared for end of support as of Unity 6.1". https://docs.unity3d.com/Packages/com.unity.sequences@2.1/changelog/CHANGELOG.html
- Timeline Signals connect a timeline to game systems. A custom track can need up to 5 C# classes. Pausing Timeline for dialogue is awkward. https://unity.com/blog/engine-platform/how-to-use-timeline-signals ; https://www.blog.radiator.debacle.us/2019/11/practical-primer-to-using-unity.html
- Larian and CDPR both presented dialogue-cinematic pipelines at GDC (2024 and 2016). BG3 has 174 hours of cinematics. https://gdcvault.com/play/1034616/ ; https://www.gdcvault.com/play/1022988/

### Inferences
- Engines need two different sequencing tools: (a) a linear, film-style track editor for authored cutscenes; (b) a systemic "dialogue cinematic" layer (camera and animation presets per dialogue line, generated automatically and then polished). Only in-house RPG engines have (b). This is a real gap in the commercial engines.
- "Record gameplay into a timeline" (Take Recorder) is a feature designers value highly and cheaply.

### Gaps
- There is no quantitative adoption data for Timeline vs third-party sequencers (Slate, Cutscene Engine). Details of Larian's procedural cinematic tiers are behind the GDC paywall.

---

## 3. Cameras

### Takeaway
Cinemachine set the model for designer-authored cameras: virtual cameras with priorities, blends, framing composers and follow/look-at. Unreal is now converging on it with the Gameplay Cameras plugin (5.5+, still experimental with breaking changes). In Godot, Phantom Camera is openly "Cinemachine for Godot" and is one of the most-starred Godot add-ons.

### Cited Findings
- Cinemachine was acquired by Unity in December 2016. Its creator, Adam Myhill, became Head of Cinematics. It was first built for Homeworld: Deserts of Kharak. Cinemachine 3 came out in 2023. https://www.provideocoalition.com/unity-introducing-era-procedural-cinematography/ ; https://unity.com/blog/engine-platform/see-whats-new-with-cinemachine-3 ; package repo about 0.7k stars: https://github.com/Unity-Technologies/com.unity.cinemachine
- UE Gameplay Cameras shipped as experimental in 5.5. The stated plan was beta in 5.6 and production-ready in 5.7 or 5.8. Changes from 5.5 to 5.6 were breaking: camera rigs were split into separate assets. It uses node-based rigs, a camera director, transitions and parameterization. Source: Ludovic Chabant's blog, 2025-01 to 2025-11. https://ludovic.chabant.com/blog/2025/06/06/ue5-gameplay-cameras-upgrading-to-5-6/ ; https://ludovic.chabant.com/blog/2025/11/14/ue5-gameplay-cameras-upgrading-to-5-7/
- Phantom Camera for Godot has about 3.6k stars (2026-09-25) and requires Godot 4.4+. It is inspired by Cinemachine and provides: priority system, follow modes (glued, simple, group, path, framed, third-person), look-at, tweened transitions, and a Viewfinder preview. https://github.com/ramokz/phantom-camera

### Inferences
- The camera authoring model the whole industry has settled on is: many virtual cameras + priority + blend + composer/dead zones + a preview. It is the most copied designer tool across engines, which is strong evidence it is the right abstraction.

### Gaps
- There is no shipped-title count for Cinemachine. Unity does not publish one.

---

## 4. Animation authoring for designers and animators

### Takeaway
State-machine graphs (Unity Animator/Mecanim, UE AnimGraph and state machines, Godot AnimationTree) are universal and universally complained about: magic strings, poor debuggability, and transition explosion. Unreal leads on next-generation authoring:
- Motion Matching (5.4, production use with GASP's 500+ free animations);
- Control Rig for in-engine keying and procedural rigs;
- IK Rig/Retargeter with auto-chains.

Unity abandoned Kinematica (last update 0.8 in August 2020) and is building a new animation system, still unreleased in 2025, to replace Mecanim. In the Unity ecosystem, Animancer (code-driven playback) and Final IK are the de facto fixes.

### Cited Findings
- UE 5.4 Motion Matching (Pose Search plugin) and the Game Animation Sample Project: 500+ free animations, commercial-use licence. https://www.unrealengine.com/blog/game-animation-sample ; https://dev.epicgames.com/documentation/en-us/unreal-engine/motion-matching-in-unreal-engine
- UE IK Rig/Retargeter: auto retarget chains, auto align, and retarget directly from the Content Browser. https://dev.epicgames.com/documentation/en-us/unreal-engine/auto-retargeting-in-unreal-engine ; https://dev.epicgames.com/documentation/unreal-engine/ik-rig-animation-retargeting-in-unreal-engine
- Kinematica (Unity motion matching) was last updated as 0.8.0-experimental in August 2020. Development was "suspended until at least 2022" and was never resumed. https://discussions.unity.com/t/what-happened-to-kinematica/862682
- Unity's new animation system was announced at Unite 2024. The status update of 2025-07-29 describes redesigned state machines (about 30% fewer states), layering (about 75% fewer clips in tests), 30–86% CPU reduction, and a graph built on Graph Toolkit. It did not mention motion matching or retargeting. Community concerns: no migration tool. https://discussions.unity.com/t/animation-status-update-summer-2025/1672386 ; https://www.cgchannel.com/2024/09/unity-previews-its-roadmap-for-unity-6-1-and-beyond/
- Animancer's case against Mecanim: too many setup steps; opaque, with a decision log that cannot be read; one controller per character; magic strings; hidden dependencies; `Play` silently ignored or delayed. https://kybernetik.com.au/animancer/docs/introduction/mecanim-vs-animancer/why/ . Animancer Pro is now at v8.4 on the Asset Store.
- Final IK (RootMotion): 877 ratings, 12,524 favorites, $90 list, v2.5 from 2026-04-10. https://assetstore.unity.com/packages/tools/animation/final-ik-14290
- Godot AnimationTree: open proposals ask for runtime debugging of state machines and blend nodes; for better code access (bone filters and progress are not settable); and fixes for transitions through intermediate states and for ignored playback speed. https://github.com/godotengine/godot-proposals/issues/8820 ; https://github.com/godotengine/godot-proposals/issues/10324 ; https://github.com/godotengine/godot-proposals/issues/3795

### Inferences
- The pain point everywhere is the hand-authored transition graph. Motion matching reduces how many transitions have to be authored. Code-first playback (Animancer) removes the graph for programmers. Designers still want a visual debugger that shows which state is active and why.
- Retargeting and in-engine keying (Control Rig) matter most for small teams that reuse mocap or marketplace animations.

### Gaps
- There are no adoption numbers for Motion Matching in shipped titles outside Epic's marketing, and no Godot motion-matching data (community add-ons exist but were not surveyed).

---

## 5. Game feel and juice

### Takeaway
Game feel became a named discipline through two talks: "Juice It or Lose It" (2012) and "The Art of Screenshake" (2013). The tooling expression in Unity is Feel (MoreMountains), a stack of more than 150 "feedbacks" that designers compose in the inspector, including shake, freeze-frame/hit-stop, time scale, post-processing and haptics. Underneath it sit the tween libraries (DOTween, plus newer allocation-free LitMotion and PrimeTween). Godot has Tween built in. Unreal covers this with built-in camera shake assets, time dilation and timelines, rather than one dominant plugin.

### Cited Findings
- Feel: $50 list, 240 reviews, 9,067 favorites, v6.1 from 2026-08-31. Features: MMF Player with more than 150 feedbacks, screen shake, Freeze Frame (hit-stop), Time Modifier, haptics. https://assetstore.unity.com/packages/tools/particles-effects/feel-183370 ; https://feel.moremountains.com/
- DOTween Pro: 818 ratings, 8,048 favorites, v1.0.430 from 2026-06-23. The free DOTween repo has about 2.7k stars. https://assetstore.unity.com/packages/tools/visual-scripting/dotween-pro-32416 ; https://github.com/Demigiant/dotween
- Newer Unity tween libraries: LitMotion about 2.3k stars and PrimeTween about 2.0k stars (GitHub, 2026-09-25). https://github.com/annulusgames/LitMotion ; https://github.com/KyryloKuzyk/PrimeTween
- The talks: Jonasson and Purho, "Juice It or Lose It" (2012); Nijman/Vlambeer, "The Art of Screenshake" (2013), covering about 30 tricks including hit-stop and camera kick. https://gamejuice.co.uk/resources/juice-it-or-lose-it ; https://valdemird.com/blog/game-feel-on-the-web/

### Inferences
- The key design idea in Feel is the one to copy: named, reusable, data-authored "feedback stacks" triggered by one call, with play-in-editor preview. Tweens are the underlying primitive.

### Gaps
- Feel's shipped-title list is not public beyond its demo games.

---

## 6. UI authoring

### Takeaway
UI authoring is weak in every engine.
- **Unreal:** UMG plus CommonUI (needed for gamepad focus and input routing) plus UMG Viewmodel (MVVM, beta since 5.3). It is powerful but has a steep learning curve and is poorly documented.
- **Unity:** the UGUI to UI Toolkit transition dragged on for years. By Unity 6.7 (2026) UI Toolkit is the stated recommendation, but world-space UI, navigation/focus and binding boilerplate are still weak points.
- **Godot:** Control nodes plus containers. The anchors/margins versus containers split confuses users.

Designers want CSS/Figma-like styling, vector shapes, data binding, and good gamepad navigation.

### Cited Findings
- "State of UI Toolkit in Unity 6.7" (2026) says UI Toolkit "covers most use cases for game UI". World-space UI arrived in 6.5 via PanelRenderer. Complaints: runtime USS variables are missing; focus and navigation; event-registration boilerplate; data binding for lists; debugger changes are not saved; UGUI LayoutGroup breakage in 6.6. https://discussions.unity.com/t/state-of-ui-toolkit-in-unity-6-7/1736756 ; comparison page: https://docs.unity3d.com/6000.2/Documentation/Manual/UI-system-compare.html
- CommonUI is Epic's layer on UMG, built for Fortnite and Paragon. Developers struggle with focus, input routing and activatable widgets, and gamepad focus loss and double input are common. https://miltoncandelero.github.io/focus-navigation-input ; https://x157.github.io/UE5/CommonUI/
- UMG Viewmodel (MVVM) has been built in since 5.3, is in beta and thinly documented. https://dev.epicgames.com/documentation/en-us/unreal-engine/umg-viewmodel-for-unreal-engine ; https://blog.rushdownstudio.com/wrangle-your-ui-data-intro-to-unreal-engines-umg-viewmodel-plugin/
- Danny McGee, a UI/UX engineer (2023-05-25), lists these UMG complaints: dual Slate/UMG implementations, fragmented container widgets, sparse docs, useful bindings discouraged for performance reasons, and no procedural vector styling (rounded corners and gradients need textures or shaders). https://dannymcgee.dev/posts/unreal-engine-deserves-a-better-ui-story
- Godot proposals: remove the anchors/margins workflow in favour of containers (#4994), and unify Control and Container (#14222). "Anchors and Margins and Containers, Godot My!" (2023). https://github.com/godotengine/godot-proposals/issues/4994 ; https://github.com/godotengine/godot-proposals/issues/14222 ; https://joshanthony.info/2023/04/22/anchors-and-margins-and-containers-godot-my/

### Inferences
- The pattern that keeps winning is web-like: flex layout, stylesheets, vector rounded rects, reactive binding. That is what UI Toolkit copied and what the UMG critics ask for. Gamepad focus and navigation are a separate hard problem that every engine under-serves.
- Many studios still use third-party UI middleware (e.g., Coherent Gameface, NoesisGUI) to get HTML/XAML workflows. Their popularity was not quantified here.

### Gaps
- No survey data on UI tool satisfaction. Middleware (Coherent, Noesis) was not researched.

---

## 7. Audio for designers

### Takeaway
FMOD and Wwise dominate. The GameSoundCon survey says Wwise leads in AAA and custom engines, and FMOD leads in indie and Unity. The most common pairings are Wwise+Unreal and FMOD+Unity. MetaSounds (free, procedural, node-based) is eroding middleware's share in UE projects. Unity's built-in audio is being rebuilt from the foundations up (Scriptable Audio Pipeline), but higher-level authoring tools come later. Sound designers need:
- a standalone authoring app with events, parameters and randomization containers;
- live-connect to the running game for mixing and profiling;
- bank/build management;
- a cheap licence.

### Cited Findings
- GameSoundCon 2023 survey (645 responses). Wwise is very popular with AAA and FMOD with indie and mid-size studios. For Unity games, FMOD is the most-used engine, slightly ahead of Wwise, and both are well ahead of Unity's built-in audio. For Unreal games Wwise is most used, ahead of built-in. For custom engines Wwise is most used, ahead of custom solutions. The survey counts users, not games. https://www.gamesoundcon.com/post/game-audio-industry-survey-2023
- Sony acquired Audiokinetic in January 2019. Audiokinetic claims 70% of the global AAA market and more than 1,000 titles a year. A free Wwise indie licence (budget under $250k) was introduced in 2022. https://sonyinteractive.com/en/press-releases/2019/sony-interactive-entertainment-to-acquire-audiokinetic-a-leading-provider-of-interactive-audio-solutions-to-the-gaming-industry/ ; https://en.wikipedia.org/wiki/Audiokinetic_Wwise ; https://www.audiokinetic.com/en/community/blog/free-wwise-indie-license/
- Licensing summary (2026-03-25). MetaSounds is free. FMOD is free under $200k revenue and $600k budget; paid tiers are about $2k, $6k and $18k per title. Wwise is free under a $250k budget; commercial pricing is by quote. https://www.strayspark.studio/blog/wwise-fmod-metasounds-audio-middleware-comparison
- Unity audio status Q2 2026: audio clips implement IAudioGenerator (scriptable generator trees); a new audio foundation layer is in testing; scriptable effects come next. Unity's position is that the foundations come before authoring tools. Users cite middleware cost as the reason to want native tools. https://discussions.unity.com/t/audio-status-update-q2-2026/1723396 ; https://discussions.unity.com/t/audio-status-update-q3-2025/1681867

### Inferences
- The sound-designer workflow that should not be broken: a separate authoring tool, **events plus parameters** as the contract with gameplay code, and live mixing against the running game. MetaSounds shows that an in-editor node graph is acceptable when it is free and integrated. An engine could offer an FMOD/Wwise integration first and native events later.

### Gaps
- No per-game counts (PCGamingWiki returned 403). The GameSoundCon 2025 survey middleware figures were not retrieved.

---

## 8. VFX (brief)

### Takeaway
Niagara (UE) and VFX Graph (Unity, GPU, millions of particles) are both node-based, and each is standard in its own engine. Cascade is deprecated. Artists choose by engine, not by tool. Godot has GPUParticles plus shaders, with no equivalent graph tool. Environment and VFX art are covered elsewhere, so this section is kept short.

### Cited Findings
- Comparison and status: https://www.rebelway.net/unity-vs-unreal-engine/ ; Niagara Fluids beta since 5.2: https://nastyrodent.com/niagara-vs-cascade/ ; Godot users asking for equivalents: https://forum.godotengine.org/t/how-can-i-create-advanced-visual-effects-like-what-would-result-from-unity-vfx-graph-and-ue5-niagara/7602

### Inferences
- A VFX node graph is expected in any engine that competes for 3D artists.

### Gaps
- Not researched in depth, on purpose.

---

## Ranking

Top 11, ordered by importance to designers and artists and by popularity:

1. **UE Sequencer (+ Take Recorder)** — the de facto in-engine cinematics tool, still getting heavy investment (CAT, Mocap Manager in 5.6). Unity's equivalent is widely called "years behind".
2. **FMOD / Wwise** — about all professional game audio. Wwise claims around 70% of AAA; FMOD is the top choice in Unity and indie (GameSoundCon 2023).
3. **Cinemachine-style virtual cameras** (Cinemachine, UE Gameplay Cameras, Phantom Camera at 3.6k stars) — the most copied designer tool across engines.
4. **Animation state machines** (Animator / AnimGraph / AnimationTree) — every game uses them and everyone complains about them. This is where engines differ most.
5. **Motion Matching + IK Retargeter / Control Rig (UE 5.4+)** — changes how locomotion is authored; the free GASP (500+ animations) drove adoption. Unity has no answer yet.
6. **Ink / Yarn Spinner** — free text-first narrative scripting (ink about 4.9k stars; Yarn about 2.8k, used in DREDGE, A Short Hike, Night in the Woods). Git-friendly.
7. **articy:draft** — the standard standalone narrative database for large branching RPGs (Disco Elysium, Hogwarts Legacy).
8. **In-house quest/dialogue editors** (REDengine quest graph, Larian Dialog/Story editors, Obsidian OEI Tools, Creation Kit) — the benchmark for what large narrative games need: dialogue, quest state, VO/loc export and cinematics in one graph.
9. **UI authoring** (UMG+CommonUI, UI Toolkit/UGUI, Godot Control) — essential and universally weak; designers want web/Figma-like layout, styling and binding.
10. **Feel + tween libraries** (Feel with 9k favorites; DOTween Pro with 8k favorites and 818 ratings) — the standard way to add juice in Unity; small tools with large leverage.
11. **Godot Dialogic / Dialogue Manager** (6.0k and 3.9k stars) — among the most-starred Godot add-ons, showing narrative tooling is a top community need in engines that lack it.
