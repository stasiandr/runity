# Tools game designers rely on: data/balance, debugging, playtesting, in-house practice

Research date: 2026-09-25. Sources: web (Asset Store pages, GitHub, Epic/Unity docs, GDC Vault, studio blogs, Kotaku). Reddit not used.
Marking: **[verified]** = number/claim read on the cited page during this session; **[search-snippet]** = from a search-result summary, page not opened; **[background]** = general knowledge, no citation fetched this session.

---

## 1. Game data & balance authoring

### Takeaway
Across every engine the real centre of gravity is **the spreadsheet** (Excel/Google Sheets). The engine-side format (UE DataTable/CurveTable, Unity ScriptableObject, Godot Resource) is the landing zone. The best-evidenced third-party tool is **Odin Inspector** (Unity): it turns the inspector into a designer-friendly editor and adds validation. The recurring complaints are the same everywhere: editing one asset at a time, binary assets that can't be merged, reimports that silently overwrite in-editor edits, and no "table view" of many objects at once. Dedicated balance simulators (Machinations) are real but niche: they show up in F2P/economy work, not everyday tuning.

### Cited Findings
- **Odin Inspector & Serializer (Sirenix)**: 790 reviews, 12,341 favorites, 5-star, $55 list, latest v4.0.2.4 released 2026-08-27, "Verified Solution". The Asset Store licence covers entities with revenue under $200k; an enterprise tier is sold separately. [verified] https://assetstore.unity.com/packages/tools/utilities/odin-inspector-and-serializer-89041
- Unity itself sells "Odin Enterprise" (Inspector + Validator + source) on unity.com, which effectively endorses it as the standard. Testimonial from Brackeys: "If I had to bring one tool with me to a deserted island it would be Odin." [verified] https://unity.com/products/odin
- JetBrains added dedicated Odin support to Rider (2024-03-20), which signals how widespread Odin is. [search-snippet] https://blog.jetbrains.com/dotnet/2024/03/20/sirenix-s-odin-inspector-support-comes-to-rider-a-jetbrains-ide/
- **Odin Validator**: scans the project for missing references, broken prefabs and so on, with attribute-driven rules and auto-fixes. [search-snippet] https://odininspector.com/tutorials/odin-validator/validator-types-overview
- **Unity ScriptableObjects**: Ryan Hipple's Unite Austin 2017 talk on SO architecture is described as "the most watched Unite conference video" on Unity's channel. Its selling point: designers tweak values in the inspector during Play Mode, and SO changes *persist* after exiting Play Mode (unlike scene changes). [search-snippet] https://unity.com/how-to/architect-game-code-scriptable-objects
- **Unity's missing table view**: multiple tools exist only to show many SOs as a spreadsheet. Examples: Scriptable Sheets (released thread, 2024+), DataForge LITE (OSS), Scriptable Object Data Browser. DataForge's pitch is "balance of game data in one place instead of clicking through assets one by one". [search-snippet] https://discussions.unity.com/t/released-scriptable-sheets/1490189, https://github.com/Ragendom69/dataforge-lite
- **Unity Play Mode changes are lost on exit, with no warning**: a long-standing complaint. People ask "Play mode save, where is it?", and third-party assets such as CoInspector add "Play Mode Save". [search-snippet] https://discussions.unity.com/t/play-mode-save-where-is-it/950394
- **Spreadsheet → Unity**:
  - BG Database ($40): 66 reviews, 707 favorites, v1.9.5 released 2026-06-19. [verified] https://assetstore.unity.com/packages/tools/integration/bg-database-data-editor-with-google-sheets-and-excel-syncing-112262
  - Google Sheets To Unity (free): 270 favorites, last version 2019, so effectively abandoned. [verified] https://assetstore.unity.com/packages/tools/utilities/google-sheets-to-unity-73410
  - Many small importers exist (IDEASAM, UniGame GoogleSpreadsheetsImporter). [search-snippet]
- **CastleDB** (Nicolas Cannasse / Shiro, a structured spreadsheet with schemas and git-friendly JSON): 610 stars. Its standalone editor is now "legacy", folded into Shiro's HIDE editor. [verified] https://github.com/ncannasse/castle
- **Depot** (Kyle Kukshtel): a free CastleDB-style JSON data editor inside VS Code, aimed at card games and roguelikes. [search-snippet] https://kylekukshtel.itch.io/depot
- **Unreal DataTable / DataAsset split**:
  - DataTable is row-based, CSV/JSON-importable and fast to add rows to. DataAsset is typed, subclassable and one asset per entry.
  - The iteration loop is "edit sheet, export, reimport, test".
  - Row structs can't hold UObject instances.
  - Reimport completely overwrites the table, so "pick one source of truth". (Hyperdense, Medium/Substack.) [search-snippet] https://medium.com/@sarah.hyperdense/data-tables-for-game-designers-spreadsheet-driven-game-data-in-ue5-1d1d3caa8534
  - Epic docs page "Data Driven Gameplay Elements" (UE 5.8) covers DataTables and CurveTables. https://dev.epicgames.com/documentation/en-us/unreal-engine/data-driven-gameplay-elements-in-unreal-engine
- **DataTables are binary .uasset files and can't be merged**: "if two designers touched the same table, the second import wins". A commercial three-way CSV merge plugin, "DataTable Merge", exists purely for this. Dolt (a versioned SQL database) ships an Unreal plugin (2024-03-11) aimed at the same problem. [search-snippet] https://csaf.itch.io/datatable-merge, https://www.dolthub.com/blog/2024-03-11-dolt-plus-unreal/
- **GAS attributes**: initialised from CSV CurveTables keyed "GroupName.AttributeSet.Attribute", editable externally or in the editor. The community GASDocumentation repo (tranek) has 6.0k stars and 1.0k forks, which is evidence of both GAS's reach and how hard it is to learn. [verified stars] https://github.com/tranek/GASDocumentation
- **Gameplay Tags**: hierarchical designer-authored IDs, used instead of free-text names and enums. Tom Looman: "Why you should be using GameplayTags". [search-snippet] https://tomlooman.com/unreal-engine-gameplaytags-data-driven-design/
  - At scale (Titan Quest 2, Grimlore, 2023-11-30), tags become messy: no relations between tags, no type safety, typos. The team wrote validators and typed tags, and "had far less tag misuse". [verified] https://jonasreich.de/blog/006-managing-gameplay-tag-complexity.html
- **Unreal Data Validation plugin**: on by default. `IsDataValid` overrides show designers errors on save and at Blueprint compile time. [search-snippet] https://dev.epicgames.com/documentation/en-us/unreal-engine/data-validation-in-unreal-engine
- **Godot**: custom `Resource` + `@export` gives `.tres` files edited in the inspector, the equivalent of ScriptableObjects. Tutorials pitch it as "designers can change values without touching code". There is no built-in table view (older third-party "Godot Data Editor" plugins exist). [search-snippet] https://godotlearning.com/blog/godot-resources-explained, https://godotengine.org/asset-library/asset/73
- **Excel as a primary tool**: gamedeveloper.com opinion piece calls Excel "the brick and mortar tool for the designer". GDC design-track sessions with Brenda Romero and Ian Schreiber on Excel techniques. [search-snippet] https://www.gamedeveloper.com/design/opinion-stop-being-the-useless-designer---excel-and-formulas, https://www.gamedeveloper.com/design/my-approach-to-economy-balancing-using-spreadsheets
- **Machinations.io**: browser-based visual economy simulator. Claims "35,000+ professionals"; lists Amazon Games, Wooga, Hasbro, Zynga, MIT, NYU and USC as users. It now markets AI "describe a loop and simulate it". [search-snippet for 35k; verified logos] https://machinations.io/
- **Katharine Neil, GDC 2017, "Game Design Tools: For When Spreadsheets and Flowcharts Aren't Enough"**: a survey of specialised design tools, framed around the fact that spreadsheets and flowcharts are the default. [verified] https://gdcvault.com/play/1024644/Game-Design-Tools-For-When
- **Live-ops tuning without a build**: Unity Remote Config and Firebase Remote Config are pitched for "tune your game difficulty curve in near real time". [search-snippet] https://docs.unity.com/en-us/remote-config, https://firebase.google.com/learn/pathways/firebase-remote-config-unity-games

### Inferences
- The universal pattern is **typed schema + table view + inspector for one record + validation**. Engines ship the typed-record part. Everyone bolts on the table view (Scriptable Sheets, CastleDB, spreadsheets) and the validation (Odin Validator, IsDataValid, tag validators).
- The spreadsheet wins because of bulk editing, formulas and derived columns, not because designers love Excel. The cost is a lossy import step and loss of mergeability. Text-based, diffable data (CastleDB, Depot, Naughty Dog's DC) avoids the merge problem.
- ScriptableObjects' "changes persist after Play Mode" is loved exactly because Unity scene edits don't persist. That points to "tweak in the running game and keep the value" as the core designer need.

### Gaps
- No public Asset Store sales or download numbers (only reviews and favorites). No survey that ranks designer tools by usage.
- No hard numbers for UE DataTable versus DataAsset adoption.
- Machinations' user count is self-reported.

---

## 2. Gameplay debugging tools for designers

### Takeaway
The most-used designer debugging tools are the unglamorous ones: **in-game console / cheat commands, on-screen debug overlays, and tweakable debug menus**. Unreal ships all three (console with CVars and CheatManager, the Gameplay Debugger on the apostrophe key, `showdebug`). Unity ships none as a runtime feature, so SRDebugger, IngameDebugConsole and Graphy fill the gap. **Recording + timeline scrub** (UE Visual Logger, Rewind Debugger) is the high-value next tier. Users call it a "life saver", but it is under-discovered. In-house studios (Guerrilla, Blizzard) build node-graph and state-script debuggers with history playback.

### Cited Findings
- **UE Gameplay Debugger**: the apostrophe key toggles it; point at an actor to select it; the numpad switches categories (AI, BT, EQS, perception, GAS). [search-snippet] https://dev.epicgames.com/documentation/en-us/unreal-engine/using-the-gameplay-debugger-in-unreal-engine
- **UE Visual Logger**: records spatial debug shapes and logs, and lets you scrub after the fact. One tutorial author, quoting Epic staff, says its "future ... is a little in question" but calls it "incredibly useful for debugging collisions, movement and AI". [search-snippet] https://unreal-garden.com/tutorials/visual-logger/
- **UE Rewind Debugger**: records a PIE session and scrubs a timeline of variable values, animation and blends. It supports custom tracks, and `UE_VLOG` events appear on its timeline.
  - The community tutorial (2025-11-28) has a user calling the VLOG integration "a life saver".
  - A third-party "GAS Rewind Debugger" plugin exists, which shows demand to extend it to gameplay systems. [verified] https://forums.unrealengine.com/t/community-tutorial-rewind-debugger-in-depth/2679912, https://forums.unrealengine.com/t/arg-games-studio-gas-rewind-debugger/2716837
- **GAS debugging**: Epic's tutorial lists the Gameplay Debugger, Visual Logger and `showdebug abilitysystem`. [search-snippet] https://dev.epicgames.com/community/learning/tutorials/Y477/unreal-engine-gameplay-ability-system-debugging-tools
- **UE console variables and CheatManager**: CVars are engine-wide typed variables readable and writable at runtime (the `ECVF_Cheat` flag keeps them out of shipping builds). Subclassing UCheatManager exposes functions to the `~` console. [search-snippet] https://dev.epicgames.com/documentation/en-us/unreal-engine/console-variables-cplusplus-in-unreal-engine, https://unreal-garden.com/tutorials/cheatmanager/
- **UE Live Coding**: patches C++ during PIE and even into packaged desktop builds. [search-snippet] https://dev.epicgames.com/documentation/unreal-engine/using-live-coding-to-recompile-unreal-engine-applications-at-runtime
- **SRDebugger (Stompy Robot)**: $30, 193 reviews, 2,228 favorites, v1.13.1 released 2026-01-08. On-device console, an "Options" tab of tweakable values, and bug reporting. [verified] https://assetstore.unity.com/packages/tools/gui/srdebugger-console-tools-on-device-27688
- **yasirkula/UnityIngameDebugConsole**: 2.7k stars, 267 forks. Runtime log viewer plus command registration. [verified] https://github.com/yasirkula/UnityIngameDebugConsole
- **Graphy**: 2.9k stars. FPS, memory and audio monitor; won "Best Development Asset" at the Unity Awards 2018. [verified] https://github.com/Tayx94/graphy
- **Hot Reload for Unity (The Naughty Cult)**: 296 reviews, 5,031 favorites, v1.13.24 released 2026-09-08. The pitch is that "time lost on compiling was ... the single biggest bottle-neck". The OSS alternative FastScriptReload exists. [verified] https://assetstore.unity.com/packages/tools/utilities/hot-reload-edit-code-without-compiling-254358
- **Godot**: the built-in **Remote scene tree** lets you inspect and *edit* properties of the running game live (e.g., tweak player speed without restarting). Open proposals ask to show the remote scene in the canvas; the community GodotRuntimeDebugTools adds click-to-select in the running game. [search-snippet] https://medium.com/@florian-trautweiler/remote-scene-tree-in-godot-4-af0bf4bc9d35, https://github.com/godotengine/godot-proposals/issues/745, https://github.com/bbbscarter/GodotRuntimeDebugTools
- **Guerrilla/Decima (GDC 2017)**: the new tools included a **node-graph debugger showing real-time execution flow with historical playback**, a GPU profiler and a global profiler, built "with minimal involvement from the tools team". [verified via talk notes] https://hackmd.io/@Lwx37VndSuiqmISuQ3_m7g/BksxvZjlj
- **Blizzard/Overwatch Statescript (Dan Reed, GDC 2017)**: a visual state-machine scripting language for all hero abilities. Prediction and replication are automated for the scripter, and the talk showed in-game Statescript debugging tools. Players later asked for Statescript debugging access in Workshop. [search-snippet] https://www.gdcvault.com/play/1024041/Networking-Scripted-Weapons-and-Abilities, https://us.forums.blizzard.com/en/overwatch/t/suggestion-allow-custom-game-owners-to-access-statescript-debugging-editing/410866
- **EA/Sims, David "Rez" Graham, GDC, "In-Game Debugging and Visualization Tools"**: breakpoints are the wrong tool for gameplay. You need windows into game data, data history, and the ability to change state from tools. [search-snippet] https://www.gdcvault.com/play/1015714

### Inferences
- Value ladder for designers:
  1. See state (overlay, debugger categories).
  2. Change state (console, cheats, tweak menu, remote inspector).
  3. See state over time (VLog, Rewind, Guerrilla's graph history).

  Tier 3 is where AAA in-house tools are ahead and where off-the-shelf tools are least discovered.
- Unity's lack of a runtime inspector or console is exactly why SRDebugger, IngameDebugConsole and Graphy are all popular. The need is universal; only who supplies it differs.
- Godot's editable remote tree is a rare built-in "tweak the running game" feature. Its limit is that edits aren't written back to the scene.

### Gaps
- No usage stats for Visual Logger or Rewind Debugger. Evidence of praise is anecdotal.
- Quantum Console and Unity's own Runtime Inspector were not checked.
- There are no designer-specific surveys of debugging tools.

---

## 3. Playtesting & telemetry

### Takeaway
For telemetry, **GameAnalytics** is the default free choice for indie and mobile teams (self-reported 100k+ studios). Unity's own heatmaps were a playtest-only add-on and are effectively absent from the current UGS Analytics. People ask on the forums and get pointed at third parties. Automated **bots** are an AAA practice (Ubisoft, DICE, Santa Monica, Eidos). Their primary users are QA/CI, but level and mission designers benefit (follow bots, solo testing of multiplayer missions). LLM/agent playtesters for balance are an emerging 2025–26 product category.

### Cited Findings
- **GameAnalytics**: claims "100,000+ studios", "180,000+ users" and 14 years in business. Free tier with no MAU limit; 3D heatmaps of events; now ships an MCP server and AI agent. [verified self-reported] https://www.gameanalytics.com/
- **Unity Analytics heatmaps**:
  - The original Heatmaps add-on was for playtesting only ("after your game has shipped ... not supported"). [search-snippet] https://discussions.unity.com/t/unity-analytics-heatmaps-official-thread/599676
  - A recent forum thread asks whether the new UGS Analytics has heatmaps and gets no official answer; replies suggest Smartlook or Play-Trace. [verified] https://discussions.unity.com/t/analytics-service-that-supports-session-replay-and-heatmaps/1691692
  - The OSS heatmapper survives on GitHub. https://github.com/RVEALR/heatmaps
- **Ubisoft Reflections, GDC 2019, "Client Bots" for The Division**: automated mission playthroughs with reports, **follow bots so level designers can test multiplayer missions solo**, and street-wandering bots for performance data. [verified] https://gdcvault.com/play/1026382/Automated-Testing-Using-AI-Controlled
- **DICE, GDC 2019, AutoPlayers for Battlefield V**: from 64-player soak tests down to scripted cases. [search-snippet] https://www.gdcvault.com/play/1026308/AI-for-Testing-The-Development
- **Santa Monica Studio, GDC 2023 Tools Summit, "TestMonkey"**: a decade-old framework covering smoke, determinism and gameplay verification, locally and in CI. [verified] https://gdcvault.com/play/1028866/Tools-Summit-TestMonkey-Automated-Testing
- **Eidos-Montréal AGT**: "Explicit Plan" tests (initialise → execute → validate). Plans are created by hand, by recording a play session, or from natural language. Primary users are QA. [verified] https://www.eidosmontreal.com/news/automated-game-testing/
- There has been an Automated Testing Roundtable at GDC every year through 2025. 2026 vendors (NodeMori BugHunter, ManaMind) pitch agents that find balance problems. [search-snippet] https://autotestingroundtable.com/, https://aiconjured.com/ai-game-dev-tools/playtesting-qa/

### Inferences
- For systems designers, telemetry is mostly *post-launch* (funnels, economy). During development the equivalent is a **deterministic record/replay + bot run** that produces numbers designers can compare between tuning passes. That only pays off if the game is deterministic or at least replayable.
- Heatmaps are loved in talks but little used in practice outside level design. Unity dropping them suggests low demand for them as a standalone product.

### Gaps
- No independent adoption numbers for GameAnalytics versus UGS versus Firebase.
- No designer-voice evidence (as opposed to QA) on bots.
- No data on how often designers use replays for balance.

---

## 4. In-house AAA engines: what designers say matters

### Takeaway
The single most repeated theme is **iteration time from "change a number" to "see it in game"**. Bad cases are cited as project-threatening: Destiny needed an overnight map import to move one node; Horizon's legacy tools took 20–30 minutes per change. Good cases are an editor that *is* the running game (Decima, Snowdrop) or a data language compiled and hot-loaded (Naughty Dog DC). Second is **designer autonomy from engineers** (Riot's Runeterra scripting, Blizzard's Statescript). Third is **data validation and robustness** (Guerrilla's undo and out-of-process design, Insomniac's immutable data).

### Cited Findings
- **Bungie/Destiny (Kotaku, 2015-10)**: "First they have to load their map overnight. It takes eight hours ... It takes about 20 minutes to open. They go in and they move that node." The engine was called "subpar" as a toolset for designers. [search-snippet quoting Kotaku] https://kotaku.com/the-messy-true-story-behind-the-making-of-destiny-1737556731
- **Guerrilla/Horizon Zero Dawn (Sumaili & van der Steen, GDC 2017)**:
  - Legacy iteration was 20–30 minutes from a designer's change to seeing it in game. The tools were rebuilt *during production* on a common framework.
  - The editor runs the full game in-process: explore in play mode, switch to edit, place encounters, test immediately.
  - 4–6 tools programmers served a 220-person team. Simple transaction-based undo beat complex command patterns.
  - "Redesigning ... while in production was a significant risk, which paid off."
  - [verified] https://www.guerrilla-games.com/read/creating-a-tools-pipeline-for-horizon-zero-dawn, https://hackmd.io/@Lwx37VndSuiqmISuQ3_m7g/BksxvZjlj, https://gdcvault.com/play/1024685/Creating-a-Tools-Pipeline-for
- **Naughty Dog DC**: a Racket/Scheme DSL for data that is "not clearly code or data" (gameplay scripts, particles, animation, sound). Dan Liebgold presented it at GDC 2008 ("Adventures in Data Compilation... Uncharted") and RacketCon 2013. [search-snippet] https://www.gdcvault.com/play/211/Adventures-in-Data-Compilation-and, https://con.racket-lang.org/2013/
- **Insomniac**: "It Stinks and I Don't Like It: Making a Better Engine Experience" (Sean Ahern, GDC 2012) argues that tool UX is treated as "a second class-citizen" and should not be. Also a web tools postmortem (Andreas Fredriksson, GDC 2017: the web is fine for 100–200 items and bad at 30,000) and "Tools for Marvel's Spider-Man: Editing with Immutable Data" (GDC 2019). [verified 2012 abstract; others search-snippet] https://gdcvault.com/play/1015729/It-Stinks-and-I-Don, https://gdcvault.com/play/1024465/Insomniac-s-Web-Tools-A
- **Riot/Legends of Runeterra (2021-01-26)**:
  - With League's BlockBuilder visual ability tool, "designers would ... get bottlenecked because they needed an engineer to create a specific custom block."
  - The replacement was per-card IronPython scripts with a VS Code plugin for autocomplete.
  - The team went from 2 designers / 4 engineers to ~15 designers / 3 engineers.
  - [verified] https://www.riotgames.com/en/news/engineering-tools-designers-legends-runeterra
- **Blizzard/Overwatch Statescript (GDC 2017)**: a visual scripting language that "unlocks designers to create new heroes with an ever-growing library of code-backed building blocks", with networking handled automatically, plus in-game debugging. [search-snippet] https://www.gamedeveloper.com/design/attend-gdc-and-see-how-blizzard-scripts-i-overwatch-i-s-weapons-and-powers
- **Frostbite at BioWare (Kotaku, 2019; press summaries)**: "like an in-house engine with all the problems that entails—it's poorly documented, hacked together". Basic RPG features (saving, third-person camera) were missing. "If it takes you a week to make a little bug fix, it discourages people from fixing bugs." [search-snippet] https://kotaku.com/how-biowares-anthem-went-wrong-1833731964, https://www.techspot.com/news/79501-anthem-development-plagued-mismanagement-frostbite-challenges-says-report.html
- **Ubisoft Snowdrop**: "the game and the editor are unified ... the ongoing project is always playable". The tool suite focuses on iteration speed and short compile times. [search-snippet] https://mcvuk.com/development-news/ubisoft-snowdrop-engine-allows-us-to-work-better-not-bigger/
- **CD Projekt RED, GDC 2024**: "Cyberpunk's Quest Editor in Action" shows designer aspirations shaping REDengine's quest and visual-scripting tooling. [verified via Toolsmiths guide] https://thetoolsmiths.org/2024/02/24/gdc-2024-toolsmiths-guide/

### Inferences
- In postmortems, designers never praise a specific widget. They praise **short loops** and **not needing a programmer**. The strongest negative stories (Destiny, Anthem) are about iteration latency and opacity, not missing features.
- Two architectures recur:
  - "Game = editor" (Decima, Snowdrop, and UE's PIE).
  - "Text data language + hot reload" (Naughty Dog DC, Riot Python).

  Both keep a running game alive while data changes.
- Validation is also consistent: Guerrilla's out-of-process isolation, Insomniac's immutable data and Grimlore's tag validators all protect designers from silent data corruption.

### Gaps
- The Horizon numbers come from third-party notes on the talk, not the slides (the PDF could not be parsed). The Destiny quote is second-hand through a search summary.
- Nothing found from Ubisoft Anvil, Frostbite's internal "live edit", or Blizzard WoW data tools that describes designer-facing tuning in detail.
- No designer-authored "my favourite tool" survey from AAA.

---

## Ranking

### By importance to designers (best evidence of impact)
1. **Live "tweak in the running game" loop** (UE PIE, Keep Simulation Changes, Godot remote inspector, Decima/Snowdrop game-as-editor, SO persistence). Every in-house postmortem (Destiny, Horizon, Anthem) turns on iteration latency.
2. **Spreadsheets (Excel/Google Sheets) plus an import pipeline**. They are where balance actually happens. Called the "brick and mortar tool"; every engine has CSV import, and there is a cottage industry of Sheets→engine tools.
3. **Typed engine data containers** (UE DataTable/DataAsset/CurveTable, Unity ScriptableObject, Godot Resource). The landing zone for all tuning; the Hipple SO talk is Unity's most-watched Unite video.
4. **In-game console, cheats and CVars / debug menu** (UE CheatManager, SRDebugger options tab). This is the cheapest way to change state; in Unity it is a paid or OSS add-on, which shows the need.
5. **Data validation** (Odin Validator, UE IsDataValid, tag validators). Guerrilla, Insomniac and Grimlore all cite robustness; it prevents silent content breakage.
6. **Inspector/editor UX for data** (Odin Inspector). It turns engine data into designer-usable forms, and Unity now resells it.
7. **On-screen state debuggers** (UE Gameplay Debugger, showdebug, Graphy overlays). These answer "why did the AI or ability do that" without a programmer.
8. **Record and scrub timelines** (UE Visual Logger, Rewind Debugger, Guerrilla graph history). Very high value when discovered ("life saver") but under-used.
9. **Designer-owned scripting or state machines with debuggers** (Statescript, Runeterra Python, UE Blueprints/GAS). Removes the engineer bottleneck, and teams re-balance accordingly (Riot went from 4 engineers per 2 designers to 3 per 15).
10. **Balance simulators and telemetry** (Machinations, GameAnalytics, bots). Important for economies and live ops, peripheral to day-to-day tuning.

### By popularity (evidence of adoption)
1. **Excel/Google Sheets**: universal; there is no counter-evidence anywhere.
2. **UE DataTables and Unity ScriptableObjects**: built-in defaults; the Hipple talk is the most-watched Unite video, and GASDocumentation has 6.0k stars.
3. **Unreal console/CVars/cheats and the Gameplay Debugger**: shipped in every UE project.
4. **Odin Inspector**: 790 reviews and 12.3k favorites (the highest favorite count found this session), plus a Unity-sold enterprise tier and a Rider integration.
5. **GameAnalytics**: self-reported 100k+ studios.
6. **Hot Reload for Unity**: 5.0k favorites and 296 reviews.
7. **Graphy / IngameDebugConsole**: 2.9k and 2.7k GitHub stars.
8. **SRDebugger**: 2.2k favorites and 193 reviews.
9. **BG Database**: 707 favorites. Spreadsheet-sync tools are individually small, which suggests most teams roll their own importer.
10. **Machinations**: self-reported 35k professionals, niche to economy design. CastleDB (610 stars) and Depot are cult favourites rather than mainstream.
