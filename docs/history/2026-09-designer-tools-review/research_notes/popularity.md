# Popularity evidence: editor tools for game designers and artists

Research date: 2026-09-25. All counts below were read on that date unless a different date is given.
Metric caveats that apply throughout:
- **Unity Asset Store**: "favourites" (users who clicked the heart) and "reviews" (number of ratings) are cumulative. They favour older assets and are not sales. An asset that was re-released as a new SKU (Gaia, Animancer v8, Behavior Designer Pro) has its counts split across listings, so these numbers undercount it.
- **Fab / UE Marketplace**: review counts come from the Orbital Market API (`POST https://orbital-market.com/api/products/search`, sortField=reviews). Orbital Market is a third-party index of Fab. It only lists products that were updated in the last 6 months and it leaves out listings flagged as AI. Prices are in cents. Fab itself (fab.com) blocks scraping behind a Cloudflare challenge, which I did not try to get around.
- **GitHub stars** measure interest from developers, not how many projects use a tool.

---

## 1. Unity Asset Store: most popular editor extensions and tools

### Takeaway
Unity's own "Top Editor Extensions" list and the per-asset counts agree. The tools people pay for most often are:
- **visual logic authoring**: Playmaker is far ahead of everything else;
- **tweening and game feel**: DOTween, Feel;
- **inspector and editor UX**: Odin;
- **visual shader authoring**: Amplify;
- **save/serialization**: Easy Save;
- **IK**: Final IK;
- **content systems for designers**: Dialogue System, Behavior Designer, A*.

Each has roughly 10k–20k favourites and 700–3,400 reviews. In the 2025 Unity Awards, the tool categories went to editor-UX and data-authoring tools (vHierarchy 2, Scriptable Sheets, Sub-Assets Toolbox) and to in-editor art tools (Cozy Builder, UModeler X, Lattice Modifier, water and outline systems).

### Cited Findings
Unity's curated "Top Editor Extensions" list (https://assetstore.unity.com/lists/top-editor-extensions-59915), in list order, with review counts: Playmaker 3,420 · Editor Console Pro 453 · Final IK 877 · DOTween 1,356 · Amplify Shader Editor 700 · Odin Inspector 790 · DOTween Pro 818 · Easy Save 1,038 · Dynamic Bone 656 · QHierarchy 221 (delisted) · Rewired 826 · I2 Localization 466.

Per-asset pages (favourites / reviews / list price):

| Asset | Category | Favourites | Reviews | Price | URL |
|---|---|---|---|---|---|
| Playmaker (Hutong) | visual scripting FSM | **20,256** | **3,420** | $65 | https://assetstore.unity.com/packages/tools/visual-scripting/playmaker-368 |
| DOTween (Demigiant) | tweening | 15,395 | 1,356 | free (Pro $15, 818 reviews) | https://assetstore.unity.com/packages/tools/animation/dotween-hotween-v2-27676 |
| Easy Save (Moodkie) | save/serialization | 13,359 | 1,038 | $59 | https://assetstore.unity.com/packages/tools/utilities/easy-save-the-complete-save-game-data-serializer-system-768 |
| Amplify Shader Editor | node shader editor | 12,797 | 700 | $80 | https://assetstore.unity.com/packages/tools/visual-scripting/amplify-shader-editor-68570 |
| Final IK (RootMotion) | IK / procedural anim | 12,524 | 877 | $90 | https://assetstore.unity.com/packages/tools/animation/final-ik-14290 |
| Odin Inspector (Sirenix) | inspector / editor UX | 12,341 | 790 | $55 | https://assetstore.unity.com/packages/tools/utilities/odin-inspector-and-serializer-89041 |
| Dialogue System (Pixel Crushers) | dialogue / quests | 11,994 | 848 | $95 | https://assetstore.unity.com/packages/tools/behavior-ai/dialogue-system-for-unity-11672 |
| A* Pathfinding Pro (Granberg) | navigation | 10,611 | 834 | $140 | https://assetstore.unity.com/packages/tools/behavior-ai/a-pathfinding-project-pro-87744 |
| Behavior Designer (Opsive) | behaviour trees | 10,154 | 762 | $95 | https://assetstore.unity.com/packages/tools/visual-scripting/behavior-designer-behavior-trees-for-everyone-15277 |
| Feel (More Mountains) | game feel / juice | 9,067 | 240 | $50 | https://assetstore.unity.com/packages/tools/particles-effects/feel-183370 |
| Rewired (Guavaman) | input | 8,749 | 826 | $45 | https://assetstore.unity.com/packages/tools/utilities/rewired-21676 |
| MicroSplat (Jason Booth) | terrain shading | 6,920 | 258 | free core + paid modules | https://assetstore.unity.com/packages/tools/terrain/microsplat-96478 |
| NodeCanvas (Paradox Notion) | BT/FSM/dialogue graphs | 4,705 | 369 | $120 | https://assetstore.unity.com/packages/tools/visual-scripting/nodecanvas-14914 |
| Obi Rope | physics sim | 3,138 | 151 | $37 | https://assetstore.unity.com/packages/tools/physics/obi-rope-55579 |
| Gaia Pro VS (Procedural Worlds) | terrain/world gen | 1,456 | 111 | $199 | https://assetstore.unity.com/packages/tools/terrain/gaia-pro-vs-terrain-trees-grass-water-for-unity-6-263149 |
| Animancer Pro v8 | animation scripting | 1,023 | 192 | $90 | https://assetstore.unity.com/packages/tools/animation/animancer-pro-v8-293522 |

Other items on "Essential Editor Extensions" (https://assetstore.unity.com/lists/essential-editor-extensions-20598), by review count:
- Adventure Creator 740
- Terrain Composer 2 732
- EasyRoads3D Pro 709
- Relief Terrain Pack 615
- Advanced Tools Mega Pack 590
- Script Inspector 3 586
- Simple Waypoint System 533
- Build Report Tool 452
- SPACE for Unity 406
- Curved UI 293
- Advanced PlayerPrefs Window 254
- Grids Pro 243
- SRDebugger 193
- Slate Cinematic Sequencer 99

**Unity Awards 2025** (announced 2025-12-09; record 36,000 community votes; https://unity.com/blog/17th-unity-awards-wrap-up-2025-winners-revealed):

| Category | Winner | Runners-up |
|---|---|---|
| Best Development Tool | Code Monkey Toolkit | vHierarchy 2, Behavior Designer Pro, Scriptable Sheets, Timeline Mixer, Magic Time, Photon Quantum, Sub-Assets Toolbox |
| Best Artistic Tool | Cozy Builder | UModeler X Plus, KWS2 Dynamic Water, All In 1 3D-Shader, Reactional Music, Lattice Modifier, Oceanis 2024 Pro, Linework (outlines) |
| Publisher of the Year | Synty Studios | Opsive, NatureManufacture, Kronnect, Photon, and others |

Earlier nominations for Best Development Tool (2023; https://x.com/AssetStore/status/1717217647727611999): Hot Reload, Odin Validator, Nova UI, FPS Engine, Motion-Matching Locomotion, Inworld AI NPC.

Publisher economics: publishers keep 70% of revenue. Procedural Worlds (Gaia) is cited at "$1M+ annually" (https://generalistprogrammer.com/tutorials/unity-asset-store-selling-guide-revenue; secondary source, unaudited).

### Inferences
- Playmaker has more favourites than any other asset and about 2.5× the reviews of the next one. Visual logic authoring for non-programmers is the category with the most paid demand. Unity later bought Bolt to fill the same gap (see §2).
- Odin, Editor Console Pro, QHierarchy, vHierarchy 2, Scriptable Sheets and Sub-Assets Toolbox all fix the same problem: the default inspector, hierarchy and data-authoring UX. This category keeps producing hits in every era of the store.
- DOTween and Feel are free or cheap, but their favourite counts show that "juice" and tweening sit on nearly every project's critical path.
- Content systems that designers author (dialogue, behaviour trees, pathfinding) cluster around 10–12k favourites. That is as high as the core editor-UX tools.

### Gaps
- There are no public sales or download numbers. The Asset Store no longer exposes "Top Paid all-time" as a stable list, and its tool-category sort pages render client-side, so I could not fetch them.
- I did not collect counts for Vegetation Studio Pro, Enviro 3, Polybrush or TextMesh Pro, and older Gaia SKUs are split across listings.
- The Unity Awards do not publish vote counts per asset.

---

## 2. Acquisitions as a signal of "essential"

### Takeaway
Both engine vendors bought, or cloned and then bought, third-party tools that were at the top of their marketplaces.

**Unity** bought the top editor extensions and made them built-in and free:
- TextMesh Pro, 2017
- Cinemachine, 2016/17
- ProBuilder + Polybrush + ProGrids, 2018
- Bolt, 2020

**Epic** bought mostly *content-production* technology:
- scanned assets: Quixel/Megascans
- photogrammetry: RealityCapture
- digital humans: 3Lateral, Cubic Motion, Hyprsense, which became MetaHuman
- archviz: Twinmotion
- asset/portfolio platforms: Sketchfab, ArtStation. Epic sold both to KitBash in August 2026.

### Cited Findings
- **Cinemachine**: bought by Unity in Dec 2016. Creator Adam Myhill became Head of Cinematics. Shipped with Unity 2017. Reported ">1M downloads".
- **TextMesh Pro**: joined Unity on 2017-03-20 and was integrated free into Unity 2017 (https://blog.unity.com/games/textmesh-pro-joins-unity). Before that it was among the best-selling Asset Store tools.
- **ProBuilder / Polybrush / ProGrids** (ProCore): Unity acquired them in Feb 2018 and made them free for all plans. The creators joined Unity (https://www.gamedeveloper.com/design/unity-acquires-probuilder-level-design-tool-and-hires-its-creators; https://www.cgchannel.com/2018/02/unity-now-comes-with-probuilder-built-in-for-free/).
- **Bolt** (Ludiq): Unity acquired it in May 2020 and made it free. It became "Visual Scripting" in Unity 2021 (https://www.gamedeveloper.com/business/unity-has-acquired-visual-scripting-solution-bolt).
- **Weta Digital tools**: Unity announced the deal at $1.625B in Nov 2021. The consideration at closing was ≈$1.5B (Unity FY2021 10-K). In the same period Unity also bought SpeedTree (IDV, 2021), SyncSketch, Pixyz and RestAR. **Ziva Dynamics** (soft-tissue/character deformation) followed in Jan 2022 (https://www.cgchannel.com/2022/01/unity-acquires-ziva-dynamics/; https://www.broadcastnow.co.uk/tech/unity-follows-up-weta-acquisition-with-ziva-dynamics/5166901.article).
- **Epic**:
  - 3Lateral (Jan 2019), Twinmotion (May 2019, then made free), Quixel (Nov 2019; Megascans free with UE)
  - Cubic Motion (Mar 2020), Hyprsense (Nov 2020)
  - Capturing Reality / RealityCapture (Mar 2021)
  - ArtStation and Sketchfab (2021)
  - Sources: https://www.fxguide.com/fxfeatured/epic-games-acquires-quixel-megascans/, https://techcrunch.com/2021/03/09/epic-games-buys-photogrammetry-software-maker-capturing-reality/, https://www.epicgames.com/site/en-US/news/hyprsense-team-joins-epic
- **Fab** launched in Oct 2024. It unified the UE Marketplace, the Sketchfab Store, Quixel and ArtStation Marketplace, with an 88/12 revenue split (https://www.gamedeveloper.com/marketing/epic-to-unify-content-marketplaces-and-offer-creators-88-percent-revenue-cut).
- **KitBash acquired ArtStation and Sketchfab from Epic**, announced 2026-08-10. Epic keeps Fab and Megascans (https://digitalproduction.com/2026/08/12/kitbash-buys-artstation-and-sketchfab/).

### Inferences
- Unity's pattern: when a store asset becomes the de facto standard for a *core editor capability*, Unity buys it. So far that has meant text, camera, in-editor modelling/blockout and visual scripting. These four are the clearest "should have been built-in" signals in the market.
- Unity built Shader Graph itself rather than buying Amplify, and Unity Behavior replaced the need for Behavior Designer only recently. The same pattern holds, just by cloning instead of buying.
- Epic's pattern: it buys the *asset supply chain*:
  - scans: Megascans, RealityCapture
  - humans: MetaHuman
  - review/viz: Twinmotion
  
  Its engine-editor gaps are filled by Fab plugins instead (see §3). Selling ArtStation and Sketchfab in 2026 suggests that the portfolio and social side was not core to Epic, while scanned content and the marketplace were.
- Weta and Ziva were high-end film-pipeline bets, not indie-designer needs. They are weak evidence for an indie-focused engine.

### Gaps
- The price of SpeedTree/IDV was not disclosed in the sources I found.
- I did not verify the later status of Unity's Weta and Ziva products. There were reports of product sunsets around 2024, but I have not confirmed them.
- "Havok" is Microsoft-owned. Unity only partnered with it (Havok Physics for Unity) and did not acquire it.

---

## 3. Fab / former UE Marketplace: top plugins

### Takeaway
On Fab the most-reviewed item overall is a **sky/weather system**: Ultra Dynamic Sky, 1,281 reviews. The most-reviewed items in the *tool-and-plugin* category are:
- 2D animation: PaperZD, 572
- graph-editor UX: Electronic Nodes 456, Blueprint Assist 177, Auto Size Comments 135
- quest/dialogue authoring: Narrative Tales 338, Dialogue Plugin 177, Ascent Toolset 171
- combat frameworks
- ocean/terrain/voxel world tools
- save: Easy Multi Save 218

After those, the next cluster is locomotion/IK (ALS V4 1,088, Dragon IK 206) and landscape auto-materials (Landscape Pro 415, Brushify 249, MW Landscape 192). Unreal's editor is stronger out of the box than Unity's, so Fab plugins cluster more in *gameplay systems* and *world look-dev* than in inspector UX. The exception is graph-wiring UX: Blueprint users pay to fix the node editor.

### Cited Findings
Orbital Market API data, category `tool-and-plugin`, sorted by review count (count · rating · price):
1. PaperZD 572 · 4.94 · free
2. Electronic Nodes 456 · 4.92 · $14.99
3. Narrative Tales (node quests + dialogue) 338 · 4.76 · $79.99
4. Ascent Combat Framework 294 · $349.99
5. Oceanology Legacy 284
6. Voxel Plugin Pro Legacy 265 · $349.99
7. Dungeon Architect 264 · $299.99
8. LE Extended Standard Library 264 · free
9. Easy Multi Save 218 · 4.85 · $99.99
10. Dragon IK 206
11. Substance 3D for Unreal 189 · rated **3.33**
12. SteamCore PRO 177
13. Blueprint Assist 177 · 4.79 · $29.99
14. Dialogue Plugin 177
15. Ascent Toolset (quests/dialogue/FSM) 171
16. SKG Shooter Framework 170
17. Advanced Vehicle System 162
18. MetaHuman Plugin 159 · rated **2.64**

Further down the same list:
- Auto Size Comments 135
- Shader World (procedural terrain/oceans/foliage) 116
- Pivot Tool 115
- Blockout Tools 111
- Actor Locker 105
- Prefabricator 104
- NWIRO AI/MCP kit 103

All categories, top by reviews:
- Ultra Dynamic Sky 1,281 (4.88, $49.99)
- Advanced Locomotion System V4 1,088 (free)
- Easy Survival RPG 634
- Landscape Pro 2.0 Auto Material 415
- Brushify Environment Shaders 249
- Easy Building System 225
- Smart Locomotion 214
- Ultimate Quest Manager 206
- AI Behavior Toolkit 198
- Easy Fog 180
- Menu System Pro 163

Separate search snippets gave these counts: Ultra Dynamic Sky 1,313 reviews on its Fab listing; Electronic Nodes 443 on Fab; Blueprint Assist 177.

**Fab 2025 year in review** (https://www.unrealengine.com/news/fab-2025-year-in-review, via search summary; the page returns 403 to direct fetch):
- live listings tripled to more than 420,000
- publishers doubled to more than 20,000
- creators earned more than $24M in the year
- top categories: environments, characters, **engine tools**, **procedural systems**, gameplay features

A seller retrospective (https://www.strayspark.studio/blog/selling-plugins-fab-marketplace) says the strong sellers are environment-scattering tools, complete gameplay systems (inventory, dialogue, save/load) and visual-polish tools. Backend and optimisation tools and niche mechanics sell weakly.

### Inferences
- The Blueprint and Material graph editors have a clear paid UX gap. Electronic Nodes (wire routing), Blueprint Assist (auto-format and keyboard navigation) and Auto Size Comments are all top-25 tools. Electronic Nodes is the most-reviewed paid tool.
- Quest and dialogue authoring appears three times in the top 15 tools on Fab. It also appears in Unity (Dialogue System) and in Godot (Dialogic, the most-starred addon; see §4). This category is one no engine ships.
- Sky/weather/atmosphere and landscape auto-materials are the "make my world look good fast" category. Artists and level designers pay for it heavily even in UE, which has its own sky atmosphere and landscape systems.
- The first-party bridges from Epic and Adobe (MetaHuman plugin 2.64, Substance for UE 3.33) have the lowest ratings among popular tools. Integration quality matters, not only whether the capability exists.

### Gaps
- There are no sales numbers.
- Orbital only indexes items updated in the last 6 months, so dormant classics (Dash, older Brushify packs, Logic Driver) may be missing. Logic Driver Pro did not appear in the top 40 tools.
- Legacy UE Marketplace review history may be split from Fab-era reviews.

---

## 4. Godot: GitHub stars of popular addons (a proxy for Asset Library popularity)

### Takeaway
The most-starred Godot addons are, in order:
- dialogue authoring: Dialogic 6.0k, Dialogue Manager 3.9k
- terrain: Terrain3D 4.3k
- Steam integration
- camera: Phantom Camera 3.6k
- AI graphs: Beehave 3.3k, LimboAI 3.0k
- scatter: Scatter 3.0k
- testing
- physics: Godot Jolt 2.6k. Jolt has since been merged into the engine.

The pattern matches Unity's and Unreal's: dialogue, terrain/world, camera, behaviour trees. The standalone tools that pair with Godot (Aseprite 39.7k, Pixelorama 10.4k, Tiled 12.9k, LDtk 4.2k, Material Maker 5.9k) show a large 2D-art and level-editing workflow that runs outside the engine.

### Cited Findings
GitHub stars, read via `gh api` on 2026-09-25:

| Repo | Stars | Category |
|---|---|---|
| godotengine/godot | 117,712 | (engine baseline) |
| dialogic-godot/dialogic | **6,017** | dialogue/VN authoring |
| TokisanGames/Terrain3D | 4,295 | terrain editor |
| nathanhoad/godot_dialogue_manager | 3,877 | dialogue authoring |
| GodotSteam/GodotSteam | 3,749 | platform |
| ramokz/phantom-camera | 3,572 | camera (Cinemachine-like) |
| bitbrain/beehave | 3,286 | behaviour trees |
| limbonaut/limboai | 3,028 | BT + HSM + editor |
| HungryProton/scatter | 2,991 | procedural scatter |
| bitwes/Gut | 2,739 | testing |
| godot-jolt/godot-jolt | 2,566 | physics |
| SirRamEsq/SmartShape2D | 1,751 | 2D terrain shapes |
| blackears/cyclopsLevelBuilder | 1,618 | in-editor blockout (ProBuilder-like) |
| Ark2000/PankuConsole | 1,436 | runtime console |
| db0/godot-card-game-framework | 1,398 | genre framework |
| viniciusgerevini/godot-aseprite-wizard | 1,370 | Aseprite import |
| dreadpon/godot_spatial_gardener | 1,309 | foliage painting |
| MikeSchulze/gdUnit4 | 1,242 | testing |
| godotengine/godot-git-plugin | 954 | VCS |
| func-godot/func_godot_plugin | 858 | TrenchBroom/.map import |
| QodotPlugin/Qodot | 758 | (predecessor of func_godot) |
| imjp94/gd-YAFSM | 673 | FSM editor |

Engine-agnostic and cross-engine tools:
- 2D art and level editing: aseprite 39,675 · Orama-Interactive/Pixelorama 10,368 · mapeditor/tiled 12,913 · deepnight/ldtk 4,209 · TrenchBroom 2,809
- materials and texturing: RodZill4/material-maker 5,940 · armory3d/armortools 5,183
- narrative: inkle/ink 4,948 · YarnSpinner 2,844
- Unity-side open source: NaughtyAttributes 5,214 · xNode 3,744 · UniTask 11,213 · DOTween repo 2,690 · odin-serializer 1,904
- Unreal-side: hugoattal/ElectronicNodes 129 (paid plugin, so its star count does not reflect its popularity)

**Godot Community Poll 2025** (9,661 respondents; summary at https://ziva.sh/blogs/godot-community-poll-2025; raw data at https://docs.google.com/forms/d/e/1FAIpQLScKWGJoLEeNW1qrsDfZRfk7gHultapacH5ZhQmo9XRZADW1IQ/viewanalytics; the per-question charts did not render for me):
- **Aseprite is used by 46.4%** of respondents (4,269 people)
- 57.1% used Unity before Godot
- 45.9% regularly make 2D and 36.9% make 3D
- 71.5% are solo developers and 86.8% are hobbyists

### Inferences
- In all three ecosystems the same four community-built categories appear at the top: dialogue authoring, terrain/foliage, camera rigs and behaviour-tree editors. These are the most dependable "engine-gap" signals.
- Godot's community is mostly 2D hobbyists, and nearly half of it uses Aseprite. Tight import from pixel-art and tilemap tools (Aseprite Wizard, LDtk and Tiled importers) matters more there than in Unity or Unreal.
- The Godot Asset Library publishes no download counts, so stars are the best proxy available.

### Gaps
- There are no Asset Library download statistics.
- I did not get the Godot poll's per-question results for "tools used alongside Godot" or "plugins used".

---

## 5. Surveys: engines and which tools and features developers use

### Takeaway
There is no major survey that asks developers to rank *editor features*. The surveys do give:
- **engine share**: GDC 2026 Unreal 42% / Unity 30% / proprietary 19%; Perforce 2024 UE 63% / Unity 47% / Godot 9%
- **collaboration pain points**: moving large files 38% is the top one, and 29% find it hard to give feedback on assets
- **backlog tools**: Jira 39%, Trello 24%
- **AI adoption**: GDC 2026 says 36% actively use AI; Unity 2025 says 96% of studios use AI tools

Stack Overflow's survey does not cover game engines or DCC tools.

### Cited Findings
- **GDC State of the Game Industry 2026** (2,300+ respondents; https://gamedevreports.substack.com/p/gdc-the-state-of-the-game-industry-fcf, https://gdconf.com/article/gdc-2026-state-of-the-game-industry-reveals-impact-of-layoffs-generative-ai-and-more/):
  - Unreal 42%, Unity 30%, proprietary 19%. This is the first time UE leads.
  - UE leads at AA (59%) and AAA (47%).
  - 54% of developers at older indie studios still use Unity.
  - 36% actively use generative AI. The top uses are research/brainstorming 81%, email 47%, coding 47%, prototyping 35%.
  - 52% say generative AI is bad for the industry.
- **Perforce/JetBrains 2024 State of Game Technology** (576 respondents, fielded Mar–Apr 2024; PDF https://www.perforce.com/system/files/2025-02/vcs_helix_core_report_2024_state_of_game_technology_report.pdf):
  - Engines: UE 63%, Unity 47%, own engine 11%, Godot 9% (20% in LATAM).
  - Indie vs AAA: UE 66/59, Unity 52/30, own engine 10/27.
  - Collaboration problems: moving large files 38%, reusing assets across teams 29%, giving feedback on assets 29%, remote work/time zones 26%.
  - Innovation blockers: not enough team members 51%, aggressive timelines 33%.
  - 69% use version control for source *and art*.
  - Backlog tools: Jira 39%, Trello 24%, ShotGrid 5%.
  - Tools for finding and reviewing art: indies use their own system 17% and Helix Core 32%; AAA use their own system 23%, Helix Core 55%, ShotGrid 19%.
- **Unity 2025 Gaming Report** (https://unity.com/blog/2025-unity-gaming-report-launch; https://www.mobilemarketingreads.com/2025-unity-gaming-report/): 96% of surveyed studios have integrated AI tools, and 45% name efficiency tools as a primary strategy.
- **Gamedev.js Survey 2025** (445 web-game developers; https://gamedevjs.com/survey/2025/):
  - graphics tools: Aseprite 30.9%, GIMP 28.4%, Blender 27.9%, Photoshop 19%
  - audio: Audacity 58.3%, Bfxr 15.2%
- **Stack Overflow 2025**: the technology section has no engine or DCC questions (https://survey.stackoverflow.co/2025/technology).

### Inferences
- The survey evidence says more about *collaboration infrastructure* than about specific editor features. Large-file handling, asset review and feedback, and asset reuse are the pain points developers name in the survey. Marketplaces barely serve them, because they are team and infrastructure problems rather than solo purchases.
- AI use today is mostly brainstorming and code. The surveys do not show artists adopting AI inside the editor.

### Gaps
- I found no survey that directly ranks editor features (for example "inspector", "prefab workflow", "undo").
- JetBrains' standalone Game Dev survey 2025 and the Unity report's full PDF were not read in detail.
- The GDC report breaks out AI use by role only partly (business 58%, managers 47%). It gives no separate figures for artists or designers.

---

## 6. Non-engine tools that designers and artists use every day

### Takeaway
Artists work in a three-tool combination most of the time; Perforce reports that respondents use three tools at "over 3:1". The most common tools are Photoshop 62%, Blender 59% and Maya 42%. Next come Houdini, After Effects and 3ds Max at 23–24%. Blender had 20M downloads in 2025. In indie and web communities, Aseprite leads 2D (46% of Godot poll respondents). Audio middleware is split between Wwise, which vendor claims put at about 70% of AAA, and FMOD for indies.

### Cited Findings
- **Perforce 2024**, "Which graphic tools or DCCs do you use":
  - Photoshop 62%, Blender 59%, Maya 42%
  - Houdini 24%, After Effects 24%, 3ds Max 23%
  - Nuke 7%, Cinema 4D 6%
  - Substance, ZBrush, Marvelous, Aseprite, Illustrator, GIMP and Procreate were listed under "other"
  - Source: PDF above, p. 20.
- **Blender Foundation Annual Report 2025** (published 2026-09-23; https://download.blender.org/foundation/Blender-Foundation-Annual-Report-2025.pdf): "20 million downloads were registered in 2025". Foundation income was €5.71M in 2025, up from €4.25M. Development Fund patrons (corporate) gave €1.36M, and individual members €0.93M.
- **Wwise**: vendor-sourced claims of "1000+ titles per year" and "70% of the global AAA market". FMOD is described as the most widely used middleware overall and powers Fortnite, Hollow Knight and Celeste (https://www.strayspark.studio/blog/wwise-fmod-metasounds-audio-middleware-comparison; secondary source, treat with caution).
- **Godot poll 2025**: Aseprite 46.4%. **Gamedev.js 2025**: Aseprite 30.9%, GIMP 28.4%, Blender 27.9%, Photoshop 19%.
- **Project management** (Perforce): Jira 39%, Trello 24%, Monday 5%, Asana 5%. 85% of respondents use a PM tool.

### Inferences
- An engine for artists has to treat Blender (and, in pro contexts, Maya) as the primary modelling tool and Photoshop/Aseprite as the texture and sprite source. That makes live, lossless round-trip import (hot reload of .blend/.fbx/.aseprite/.psd) a baseline expectation rather than a feature. In the marketplaces, Unity's Aseprite importer and Godot's Aseprite Wizard (1.4k stars) show demand for exactly this.
- Houdini and procedural tools are used by about a quarter of respondents. Fab's "procedural systems" category and scatter tools (Godot Scatter 3.0k) point the same way.
- Designers' main tools outside the engine (Jira/Trello, spreadsheets, Miro) are not tracked well by these surveys. The data-authoring demand shows up instead as Scriptable Sheets and Odin inside Unity.

### Gaps
- I found no reliable adoption numbers for Substance Painter/Designer, SpeedTree, Gaea/World Machine, Marmoset, articy:draft, Miro or Google Sheets. Adobe does not publish Substance seat counts, and the Perforce survey buried Substance in "other".
- The Wwise and FMOD shares come from vendors or secondary sources.
- The Blender Foundation's "Blender by the Numbers" section was announced but I did not find it in the PDF text.

---

## Ranking

The ranking uses only popularity evidence. The main rule is that a category ranks high when third-party markets *repeatedly* show people paying for it, or when an engine vendor bought or cloned it. Both mean the engine left a gap. For each category, the evidence line lists the strongest signals across markets.

| # | Category | Evidence (strongest numbers) | Signal strength |
|---|---|---|---|
| 1 | **Visual logic / state-machine / behaviour authoring for non-programmers** | Unity: Playmaker 20,256 fav / 3,420 reviews (#1 in the store), Behavior Designer 10,154 / 762, NodeCanvas 4,705. Unity bought Bolt (2020); Behavior Designer Pro was a 2025 award runner-up. Godot: Beehave 3.3k, LimboAI 3.0k. UE: Logic Driver, Ascent Toolset 171, AI Behavior Toolkit 198. | Very high. Bought by Unity; paid for in all 3 ecosystems |
| 2 | **Inspector / editor UX / data authoring** (custom inspectors, hierarchy, graph wiring, spreadsheets of data) | Odin 12,341 / 790; Editor Console Pro 453; QHierarchy 221; vHierarchy 2, Scriptable Sheets and Sub-Assets Toolbox were 2025 award runners-up; NaughtyAttributes 5.2k stars. UE: Electronic Nodes 456 (#2 tool on Fab), Blueprint Assist 177, Auto Size Comments 135. | Very high. A perennial category, and new winners keep appearing |
| 3 | **Narrative: dialogue / quest authoring** | Dialogue System 11,994 / 848; Adventure Creator 740. Fab: Narrative Tales 338 (#3 tool), Dialogue Plugin 177, Ascent Toolset 171, Ultimate Quest Manager 206. Godot: Dialogic 6.0k (**most-starred addon**), Dialogue Manager 3.9k. ink 4.9k, Yarn 2.8k. | Very high. No engine ships it; it is top-3 in every ecosystem |
| 4 | **World look-dev: terrain, sky/weather, vegetation, scatter** | Fab: **Ultra Dynamic Sky 1,281, the most-reviewed Fab item overall**; Landscape Pro auto-material 415; Brushify 249; MW Landscape 192; Shader World 116; Dungeon Architect 264; Voxel Plugin 265. Unity: MicroSplat 6,920 fav; Gaia (~$1M+/yr publisher); Terrain Composer 732; EasyRoads3D 709; RTP 615. Godot: Terrain3D 4.3k, Scatter 3.0k, Spatial Gardener 1.3k. Epic bought Quixel; Unity bought SpeedTree. Fab lists "environment scattering" and "procedural systems" among its top sellers. | Very high for artists and level designers |
| 5 | **Animation: tweening, game feel, IK, locomotion** | DOTween 15,395 fav (highest after Playmaker) + Pro 818 reviews; Final IK 12,524; Feel 9,067; Dynamic Bone 656; Animancer. Fab: ALS V4 1,088 (#2 overall), Dragon IK 206, Smart Locomotion 214. UE's MetaHuman came from acquisitions of 3Lateral, Cubic Motion and Hyprsense. | High |
| 6 | **Visual shader / material authoring** | Amplify Shader Editor 12,797 fav; Unity then built Shader Graph. Material Maker 5.9k and ArmorPaint 5.2k stars. On Fab, material *packs* (auto-landscape, water, glass) sell rather than editors, because UE ships a material graph. | High (the gap closes once an engine ships a graph) |
| 7 | **Camera rigs / cinematics** | Cinemachine was bought by Unity (2016) and had >1M downloads; Slate 99; Cinema Director 172. Godot: Phantom Camera 3.6k. | High (bought) |
| 8 | **In-editor modelling / blockout / level building** | ProBuilder + Polybrush bought by Unity (2018). 2025 Best Artistic Tool: Cozy Builder, with UModeler X runner-up. Godot: Cyclops 1.6k, func_godot 0.9k; TrenchBroom 2.8k. Fab: Blockout Tools 111, Prefabricator 104, Actor Locker 105. | High (bought, and still award-winning) |
| 9 | **Save / serialization** | Easy Save 13,359 fav / 1,038 reviews; Fab: Easy Multi Save 218, Save Extension 113. | High, but this is a programmer tool that designers benefit from |
| 10 | **AI navigation** | A* Pathfinding Pro 10,611 / 834 at $140, even though Unity has a built-in NavMesh. | Medium–high |
| 11 | **Text / UI / localization** | TextMesh Pro bought (2017); I2 Localization 466; Curved UI 293; Fab Menu System Pro 163. | Medium (bought) |
| 12 | **2D pipeline: sprite animation, pixel art, tilemaps** | Fab: PaperZD 572 (**#1 tool on Fab**). Aseprite used by 46.4% of Godot poll respondents and 30.9% of web game developers (39.7k stars); Pixelorama 10.4k; Tiled 12.9k; LDtk 4.2k; Aseprite Wizard 1.4k. | Medium–high (dominant for the indie and Godot audience) |
| 13 | **Input remapping** | Rewired 8,749 / 826. | Medium (programmer-side) |
| 14 | **DCC bridges / round-trip** | Photoshop 62%, Blender 59% (20M downloads in 2025), Maya 42%, Houdini 24%. The bridges vendors ship are rated poorly on Fab (Substance for UE 3.33, MetaHuman plugin 2.64). | Baseline requirement; quality is the differentiator |
| 15 | **Collaboration: large files, asset review/feedback** | Perforce: moving large files 38% is the #1 collaboration pain; asset feedback 29%; 69% keep art in version control. Marketplaces barely address it. | Surveyed pain, not marketplace-visible |

**Reading the ranking for scrap.** The pattern appears in Unity, Unreal/Fab and Godot. The designer- and artist-facing gaps that people pay to fill again and again are:
1. authoring logic without code
2. a better inspector and data-editing UX
3. dialogue and quests
4. world look-dev (terrain, sky, scatter)
5. animation feel (tween, IK)

Camera, blockout modelling and visual shaders are "solved" only in the engines whose vendor bought or cloned the tool. Everywhere else they show up in the market again.
