# Technical artists: pain points and praise across engines (UE, Unity, Godot, in-house), as of Sep 2026

Method note: I ran about 22 search/fetch calls. Items marked "(snippet)" come from search-result summaries and were not fetched in full, so treat them as lower confidence. Dates are given where the source shows them.

## 1. Shader authoring: node graphs vs code, permutations/variants

### Takeaway
In Unreal, the Material Editor is still the node system TAs like best, and Substrate became production-ready in 5.7. The costs are per-pixel expense and a huge number of permutations and compiles. In Unity, the big TA pain is structural. SRPs have no surface shaders, and "Block Shaders", the promised replacement, has been in limbo since 2022. On top of that come shader-variant explosion (Unity itself acknowledges it) and Shader Graph output that runs slower than hand-written HLSL. Godot's shader language is simple and well liked, but the visual shader editor is seen as a teaching tool, and the render pipeline is rigid.

### Cited Findings
**Unreal**
- Shader compilation is described as ranging from "a minor inconvenience to completely inhibiting, especially for those on lower-end hardware". Projects can have "several hundred thousand shaders", and landscape components alone can produce tens of thousands. The editor can use up to 80% CPU while compiling. — [techarthub (Nick Mower)](https://techarthub.com/speed-up-shader-compilation-in-unreal-engine/)
- Epic's own mitigation is On-Demand Shader Compilation (ODSC), which compiles only the shaders visible on screen in the editor. It is on by default since UE 5.1, and Epic reports about 60% fewer required permutations. — [Epic public roadmap: ODSC](https://portal.productboard.com/epicgames/1-unreal-engine-public-roadmap/c/847-odsc-on-demand-shader-compilation); [techarthub](https://techarthub.com/speed-up-shader-compilation-in-unreal-engine/)
- Common TA workarounds are ini hacks: `r.ShaderCompiler.JobCacheDDC=1`, `NumUnusedShaderCompilingThreads`, and raising ShaderCompileWorker priority. Epic hosts several community tutorials that exist only to speed up "Compiling Shaders". — [techarthub](https://techarthub.com/speed-up-shader-compilation-in-unreal-engine/); [Epic community tutorial](https://dev.epicgames.com/community/learning/tutorials/7B09/unreal-engine-speed-up-compiling-shaders); [Epic community tutorial 2](https://dev.epicgames.com/community/learning/tutorials/baE3/compiling-shaders-quickly-in-unreal-engine)
- Epic has publicly addressed runtime shader stutter with a tech-blog post on PSO precaching. The system was experimental in 5.2 and "prevents most kinds of shader compilation stutters" (snippet; the page returned 403 on fetch). — [Unreal tech blog: Game engines and shader stuttering](https://www.unrealengine.com/tech-blog/game-engines-and-shader-stuttering-unreal-engines-solution-to-the-problem)
- Substrate timeline: experimental in 5.2, beta in 5.5, production-ready in 5.7. A 4-layer Substrate material is "categorically more expensive" than a legacy material with the same texture count, so teams need per-platform tiering. (Snippet from a third-party studio blog.) — [StraySpark blog](https://www.strayspark.studio/blog/substrate-materials-production-pipeline-ue5-7); [Epic docs: Substrate overview (5.8)](https://dev.epicgames.com/documentation/unreal-engine/overview-of-substrate-materials-in-unreal-engine?lang=en-US)
- Epic backed Substrate with a free pack of more than 280 automotive Substrate materials for UE 5.7.3+ (Mar 2026). — [Unreal news](https://www.unrealengine.com/news/get-over-280-production-ready-automotive-substrate-materials-for-ue-5-7-free-on-fab); [Digital Production, 2026-03-02](https://digitalproduction.com/2026/03/02/280-free-automotive-substrate-materials-for-ue-5-7-epic-drops-a-ready-made-lookdev-kit/)
- TAs describe the Unreal Material Editor as more flexible than Unity Shader Graph, and "by far the friendliest to use, customize, and experiment with" (snippet; forum and devlog opinions, low weight). — [Moonjump forum](https://moonjump.com/forum/game-dev/shader-graph-vs-hand-written-glsl-hlsl-when-visual-node-tools-start-costing-you-e6160b)

**Unity**
- Unity acknowledges the problem officially (blog, 2024-05-28). It says "we often see shaders with over 100 keywords, leading to an unmanageable number of resulting variants, often referred to as shader variants explosion". It cites variant spaces "in the millions before any filtering", more than 1 GB of runtime memory lost to unmanaged variants, and builds that take multiple hours. `#pragma dynamic_branch` is offered as an alternative, with a warning about weaker GPU performance. — [Unity blog: Shader variants optimization & troubleshooting](https://unity.com/blog/engine-platform/shader-variants-optimization-troubleshooting-tips)
- Stripping is left to the user. Options are URP Asset feature toggles, "Strip Unused Post Processing Variants", `shader_feature` instead of `multi_compile`, and scripted `IPreprocessShaders.OnProcessShader` callbacks. — [Unity Manual: Reduce shader variants in URP](https://docs.unity3d.com/6000.0/Documentation/Manual/urp/shader-stripping.html); [Unity Manual: Strip shader variants](https://docs.unity3d.com/6000.0/Documentation/Manual/shader-variant-stripping.html). Unity staff also maintain a community tool for this: [cinight/ShaderVariantTool](https://github.com/cinight/ShaderVariantTool)
- No surface shaders in SRP. "Block Shaders (Surface Shaders replacement)" was announced as a public demo in 2022 on Unity Discussions. Its purpose is to "unify shader authoring across render pipelines" and to extend shaders without editing the original source. As of the 2024 roadmap coverage, "when Block Shaders will be production ready is unknown", and new shader-authoring tools were pushed to the "next major release generation". The roadmap page now returns 404 (checked Sep 2026). — [Unity Discussions thread](https://discussions.unity.com/t/block-shaders-surface-shaders-for-srps-and-more-public-demo-now-available/897616); [CG Channel, Sep 2024](https://www.cgchannel.com/2024/09/unity-previews-its-roadmap-for-unity-6-1-and-beyond/); [roadmap URL (404)](https://unity.com/roadmap/1375-block-shaders-surface-shaders-replacement-); [older thread "Are Surface Shaders coming to URP?"](https://discussions.unity.com/t/are-surface-shaders-or-an-equivalent-coming-to-urp-whats-the-roadmap-for-this/821869)
- Hand-written shaders break across SRP versions. Unity publishes a separate guide for upgrading custom shaders for URP. In URP 17 (Unity 6), custom passes "need to be rewritten using the render graph API". Compatibility Mode is only temporary, and Unity no longer develops the non-Render-Graph path. — [URP custom shader upgrade guide](https://docs.unity3d.com/Packages/com.unity.render-pipelines.universal@14.0/manual/urp-shaders/birp-urp-custom-shader-upgrade-guide.html); [Upgrade to URP 17 (Unity 6)](https://docs.unity3d.com/6000.4/Documentation/Manual/urp/upgrade-guide-unity-6.html); [Compatibility Mode](https://docs.unity3d.com/6000.1/Documentation/Manual/urp/compatibility-mode.html)
- Anecdote: a Shader Graph water shader took 2 ms on mobile, versus 0.4 ms for a hand-written version with no visual difference. Custom Function nodes need "workaround gymnastics" for SV_Position and screen-space derivatives. (Snippet; single anecdote from a forum.) — [Moonjump forum](https://moonjump.com/forum/game-dev/shader-graph-vs-hand-written-glsl-hlsl-when-visual-node-tools-start-costing-you-e6160b)

**Godot**
- GDShader is based on GLSL ES 3.0 and adds uniforms that bind directly to inspector properties. Godot 4.0 improved the shader editor with a creation dialog, warnings, and visual shader work. — [Godot blog: shader improvements in 4.0](https://godotengine.org/article/improvements-shaders-visual-shaders-godot-4/); [Ziva guide 2026 (snippet)](https://ziva.sh/blogs/godot-shaders)
- The visual shader editor is described as "mostly a teaching tool", and its errors point to generated code rather than to the graph. (Snippet, secondary blogs.) — [Ziva](https://ziva.sh/blogs/godot-shaders); [Bugnet](https://bugnet.io/blog/fix-godot-shader-not-compiling-visual-shader)
- An older forum complaint (pre-4.0) calls Godot "locked into a generic shading pipeline": fixed vertex attributes, few blend modes, no multipass. — [Godot Forums](https://godotforums.org/d/28102-will-godot-4-have-a-richer-shading-language)
- Pipeline-compilation stutter was the "most-complained-about runtime issue". It was addressed from Godot 4.4 onward with ubershaders and pipeline precompilation (snippet). — [Ziva](https://ziva.sh/blogs/godot-shaders)

### Inferences
- The common thread is that TAs pay for permutations in every engine. Unreal charges it as editor and cook compile time, and Unity charges it as build time, memory, and hand-written stripping code. Epic's fix was automatic (ODSC, PSO precache). Unity's fix is largely documentation plus APIs for the user.
- Unity's loss of surface shaders, with Block Shaders still undelivered after about 4 years, is probably the single most TA-specific Unity grievance.

### Gaps
- I found no hard 2025–2026 data on Unity keyword limits. Global keyword limits were reportedly relaxed in 2021.2, but I did not verify this.
- UE Custom node limits (no includes/functions without hacks) were not sourced in this pass.
- The current (2026) status of Block Shaders and Unity's new shader system is unverified because the roadmap page returns 404.

## 2. Tools and pipeline scripting, DCC, validation

### Takeaway
In Unreal, TAs value Python plus Editor Utility Widgets for automation. Examples include batch import, material assignment, and menu extension, and a whole ecosystem of community recipe books has grown around it. The evidence found here is mostly how-to material, not complaints. The Unity editor-scripting side (IMGUI vs UI Toolkit, AssetPostprocessor) and DCC round-trips were not well covered.

### Cited Findings
- UE Python in EUWs is used to store user preferences, access data outside Unreal, and do tasks "more efficiently than in a Blueprint Graph". The EditorUtilitySubsystem can launch, query, and close tools. Python can also extend the main menu so artists can find tools. — [bralkor/unreal_python_recipe_book (5.2)](https://github.com/bralkor/unreal_python_recipe_book/blob/5.2/documentation/07_editor_utility_widgets.md); [example widget](https://github.com/bralkor/unreal_python_recipe_book/blob/5.2/documentation/08_editor_widget_example.md)
- Typical TA automations: batch material creation and assignment, FBX import sorted into skeletal or static meshes, and repeated re-import of tiled terrains. — [ArtStation blog (UE5 + Python)](https://www.artstation.com/blogs/jolchawa/boPo1/making-a-data-fetching-utility-editor-widget-ue5-python); [George Hulm](https://b00merang.artstation.com/blog/yPVv/basic-ue4-editor-scripting-1-creating-objects-with-python-in-an-editor-utility-widget)
- Friction: loading or launching an EUW from Python is a recurring question on tech-artists.org. The UE Python API docs were long labelled "Experimental" (4.26/4.27). — [tech-artists.org thread](https://www.tech-artists.org/t/how-do-you-load-an-editor-utility-widget-with-python/13588); [UE 4.26 Python API](https://docs.unrealengine.com/4.26/en-US/PythonAPI/class/EditorUtilityWidget.html)
- Frostbite (in-house) builds TA-facing procedural tools centrally. Its Terrain Procedural Framework is a non-destructive, GPU-based layer compositing system used by five EA titles, including Battlefield 2042, with "automated workflows in the editor" so that "artists iterate faster". — [Frostbite at GDC 2023](https://www.ea.com/frostbite/news/frostbite-presents-at-gdc-2023)

### Inferences
- UE's documented, officially supported Python layer is a real TA advantage over Unity, where tooling is C#-only. That makes it a shared TA/programmer skill (inference, not directly sourced here).

### Gaps
- No sources were gathered this pass on the UE Data Validation plugin, Unity AssetPostprocessor, IMGUI vs UI Toolkit, the Houdini Engine, Maya/Blender round-trips, or USD. These need a follow-up.

## 3. Performance/profiling for art

### Takeaway
"Texture streaming pool over budget" is a perennial UE forum topic. The durable fix is asset discipline (max sizes, texture groups, compression), not raising the pool. Overall, the evidence gathered on profiling tools is thin.

### Cited Findings
- The over-budget warning means the textures need more memory than the pool allows, so Unreal serves low mips. Raising `r.Streaming.PoolSize` helps only if spare VRAM exists. The durable fix is capping sizes, avoiding 4K textures on small props, and using texture stats to find the one or two assets that dominate. — [Bugnet](https://bugnet.io/blog/how-to-fix-unreal-texture-streaming-pool-over-budget); [techarthub](https://techarthub.com/fixing-texture-streaming-pool-over-budget-in-unreal/); [Epic community tutorial](https://dev.epicgames.com/community/learning/tutorials/Dl70/unreal-engine-texture-streaming-pool-guide-fix-over-budget-issues-optimize-your-textures)
- Repeated forum threads across UE4 and UE5 show the issue persisting. — [UE forum 1](https://forums.unrealengine.com/t/texture-streaming-pool-over-budget/495712); [UE forum 2](https://forums.unrealengine.com/t/how-to-debug-a-case-of-texture-streaming-pool-over-budget/2120091); [UE5 thread](https://forums.unrealengine.com/t/texture-streaming-pool-ue5/598399)
- A TA-blog guide on shader optimization in UE (Dec 2023) covers shader-complexity-driven optimization. — [Luna's Technical Art Blog](https://calvinatorrtech.art.blog/2023/12/20/optimizing-shaders-in-unreal-engine/)
- Unity recommends the Memory Profiler package and "Log Shader Compilation" for tracking shader memory and variants. — [Unity blog 2024-05-28](https://unity.com/blog/engine-platform/shader-variants-optimization-troubleshooting-tips)

### Gaps
- No specific sources were gathered on Unreal Insights usability for artists, Nanite/Lumen debug views, HLOD pain, or Unity Frame Debugger/Profiler opinions.

## 4. Build/iteration pain and version control

### Takeaway
Iteration speed is where TAs feel each engine's architecture. For Unreal, that means shader compiles (section 1). For Unity, it means domain reload and import times, which Unity says CoreCLR/"Code Reload" will fix; that is still in progress. For Unreal, One File Per Actor also creates hashed, unreadable file names that frustrate people doing merges and reviews, and the complaints run from 2021 to 2025 with no Epic reply in the thread.

### Cited Findings
- Unity: a user (Dec 22, 2024) reports 20–30 s domain reloads on every play, "no less than an hour a day", and says they are moving their university away from Unity. Community replies suggest disabling domain reload (Enter Play Mode settings), using asmdefs, removing packages, and point to CoreCLR: "It's being worked on". Related threads span 2021–2026. — [Unity Discussions: "Reloading Domain, Importing Assets - Killing Development"](https://discussions.unity.com/t/reloading-domain-importing-assets-killing-development/1573916)
- Unity's plan: CoreCLR/.NET 8 integration, and "Code Reload is replacing Domain Reload" (Unite 2024). — [Unity Discussions: CoreCLR and .NET Modernization (Unite 2024)](https://discussions.unity.com/t/coreclr-and-net-modernization-unite-2024/1519272?page=12)
- The Library folder holds engine-format imported assets. Windows Defender scanning Library and Temp slows every import measurably (snippet). — [Akash Bhatt blog](https://theakashbhatt.com/unity-editor-slow-fixes/); [Unity Manual: Refreshing the Asset Database](https://docs.unity3d.com/2023.1/Documentation/Manual/AssetDatabaseRefreshing.html); [dev.to: reducing import times](https://dev.to/attiliohimeki/reducing-assets-import-times-in-unity-2kn2)
- UE OFPA: external actor files get hashed names such as `KCBX0GWLTFQT9RJ8M1LY8.uasset` under `__ExternalActors__`. Users say this makes it hard to identify changed actors or review changes in Git/SVN/Plastic. The feature request dates from Aug 2021, complaints continue through Jul–Aug 2025 ("dumpster fire"), no Epic staff response appears, and third-party tools such as Anchorpoint fill the gap. — [UE forum feature request](https://forums.unrealengine.com/t/feature-request-one-file-per-actor-include-actor-name-in-external-actor-file-names/243514); [Epic docs: OFPA](https://dev.epicgames.com/documentation/en-us/unreal-engine/one-file-per-actor-in-unreal-engine)
- Binary .uasset files cannot be merged, so teams choose one version and rely on Perforce exclusive locking. Blueprint merge was reportedly possible only for data assets as of 5.3 (vendor blog). — [Diversion blog](https://www.diversion.dev/blog/how-to-handle-merge-conflicts-in-unreal-engine-5); [Perforce for Unreal for Beginners, 2024-04-14](https://larstofus.com/2024/04/14/perforce-for-unreal-for-beginners/); [Epic docs: Perforce](https://dev.epicgames.com/documentation/unreal-engine/using-perforce-as-source-control-for-unreal-engine?lang=en-US)

### Gaps
- I did not check whether Unity CoreCLR/Code Reload shipped in a 6.x release by Sep 2026.
- No sources on Unity YAML SmartMerge pain or UE cook times this pass.

## 5. VFX tools: Niagara vs VFX Graph vs Shuriken

### Takeaway
Artists generally rate Niagara the most pleasant and most powerful. Unity's split into CPU Shuriken (physics access, mobile-safe, fast to iterate) and GPU VFX Graph (high counts, compute required, weak on mobile/URP historically) forces a per-project choice.

### Cited Findings
- On realtimevfx.com, artists say VFX Graph was "not ready for mobile use yet with the URP" and needs compute-capable devices. Shuriken has "access to the main physics system" (collisions) and iterates faster for stylized work. Beginners are told to "learn with Shuriken first" because VFX Graph is "overwhelming". (Posts from October; year not shown, likely around 2020.) — [Real Time VFX: Unity VFX Graph and Shuriken](https://realtimevfx.com/t/unity-vfx-graph-and-shuriken/15033)
- One VFX artist found Niagara smoother and more enjoyable than Shuriken or VFX Graph (snippet). — [80.lv: Creating a VFX Fire Pack for Unity & Unreal](https://80.lv/articles/creating-a-vfx-fire-pack-for-unity-unreal-engine)
- Frostbite (in-house) built its own graph-based GPU "Emitter Graph", including shader generation (GDC 2018). — [GDC Vault: Frostbite GPU Emitter Graph System](https://www.gdcvault.com/play/1025132/)

### Gaps
- There are no 2025–2026 sources on VFX Graph mobile/URP maturity, and no data on Niagara's debugger or performance for artists.

## 6. GDC / community themes and vendor statements

### Takeaway
GDC 2026 (Mar 9–13) kept a two-day Technical Artist Summit. Its topics point to TAs moving into compute and graphics-programming territory, e.g. Epic's "Shaders 501: Compute for Tech Artists". The TA Roundtable reached its 20th year. The Shaders 201/301 bootcamp series shows that shader teaching remains the backbone of TA education.

### Cited Findings
- GDC 2026 TA Summit, Mon–Tue. "Shaders 501: Compute for Tech Artists" was given by Epic's Matt Oztalay on Mar 10, 2026. — [GDC session page](https://schedule.gdconf.com/session/shaders-501-compute-for-tech-artists/915095); [TA Summit page](https://gdconf.com/technical-artist-summit/)
- "Technical Artists Roundtable (20th Year)", led by Jeff Hanna (NVIDIA), Mar 13, 2026. — [GDC session](https://schedule.gdconf.com/session/technical-artists-roundtable-20th-year-day-3/917316)
- Earlier bootcamp talks: Shaders 201 "Creating Art with Math" and Shaders 301; TA Summit "Bringing the World to Your Shaders". — [GDC Vault 201](https://www.gdcvault.com/play/1024282/Technical-Artist-Bootcamp-Shaders-201); [GDC Vault 301](https://www.gdcvault.com/play/1025538/Technical-Artist-Bootcamp-Shaders); [GDC Vault TA Summit](https://gdcvault.com/play/1028009/Technical-Artist-Summit-Bringing-the)
- Vendor acknowledgements: Unity on "shader variants explosion" (2024), and Epic on shader stutter and the ODSC permutation reduction. — see section 1 links

### Gaps
- I found no formal 2025–2026 "state of tech art" survey. The searches returned only salary aggregators. Roundtable content and notes were not retrieved.

## 7. In-house engines

### Takeaway
Public evidence is sparse and comes from presentations rather than complaints. In-house teams (Frostbite, Guerrilla) build bespoke graph tools and procedural systems for artists. TA pain inside these engines is rarely documented publicly.

### Cited Findings
- Frostbite: GPU Emitter Graph (GDC 2018) and FrameGraph rendering architecture (GDC 2017). The Terrain Procedural Framework is used by five EA titles (GDC 2023). — [GDC Vault Emitter Graph](https://www.gdcvault.com/play/1025132/); [GDC Vault FrameGraph](https://www.gdcvault.com/play/1024612/FrameGraph-Extensible-Rendering-Architecture-in); [Frostbite GDC 2023](https://www.ea.com/frostbite/news/frostbite-presents-at-gdc-2023)
- Guerrilla (Decima): Principal Artist Gilbert Sanders presented Horizon Zero Dawn shader, mesh, and render-pass implementation and workflow (GDC 2018 artist sessions). — [80.lv: GDC 2018 Sessions for Artists](https://80.lv/articles/gdc-2018-sessions-for-artists)

### Inferences
- The well-known rumored pain of Frostbite being hard to use for non-shooter genres is outside the TA-specific scope and was not sourced here.

### Gaps
- There is no public evidence in this pass on TA experience in Ubisoft Anvil/Snowdrop or on Decima's shader tooling UX. The Polycount and r/TechnicalArtist searches returned little that was usable.
