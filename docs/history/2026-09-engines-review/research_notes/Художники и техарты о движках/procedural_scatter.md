# Procedural placement / scatter / vegetation systems: engines and AAA in-house pipelines (state as of Sept 2026)

Scope: how scatter/vegetation placement works in Unreal, Unity, Godot and in published AAA pipelines (Horizon Zero Dawn, Ghost of Tsushima, Far Cry 5, Witcher 3, Tiny Glade); what developers praise and what hurts. Written for a small Rust engine that generates a valley with forest and grass (flat-shaded, low-poly, deterministic simulation).

## Unreal Engine: PCG framework, Landscape Grass, Foliage, Nanite Foliage, HISM

### Takeaway
PCG is Unreal's single strategic scatter tool. Epic declared it Production-Ready in UE 5.7 (Nov 2025). GPU execution appeared in 5.5 and is still experimental and HLSL-flavoured. Runtime generation works, but its CPU/game-thread overhead (graph scheduling, cache CRCs, component/actor creation) is the main complaint on consoles in 2026. Landscape Grass is still the older runtime path: CPU, async HISM builds. Nanite Foliage (5.7, experimental) changes how foliage is rendered, not where it is placed.

### Cited Findings
- **PCG status.** PCG is officially Production-Ready in UE 5.7, though Epic says it is still under active development. GPU and game-thread optimizations make it "nearly double the performance compared to UE 5.5". 5.7 also adds a PCG editor mode (spline drawing, paint and volume tools), standalone graph execution and FastGeo support — [Digital Production, Oct 2025](https://digitalproduction.com/2025/10/17/unreal-5-7-preview-pcg-grows-up-foliage-gets-fancy/); [AlternativeTo news, Nov 2025](https://alternativeto.net/news/2025/11/unreal-engine-5-7-adds-procedural-content-generation-nanite-foliage-and-metahuman-updates)
- **GPU execution.** GPU generation has existed since 5.5 and was heavily optimized in 5.6. It "is still experimental and requires a basic understanding of HLSL" — [80.lv tutorial on PCG GPU in 5.6](https://80.lv/articles/introduction-to-gpu-generation-with-unreal-engine-5-6-s-pcg); official doc: [Using PCG with GPU Processing (UE 5.8 docs)](https://dev.epicgames.com/documentation/unreal-engine/using-pcg-with-gpu-processing-in-unreal-engine)
- **5.6 runtime work.** 5.6 added fine-grained time slicing for compute graph dispatch and cut the game-thread cost of GPU graph dispatch and of runtime generation. That Epic had to optimize this shows it was a known pain point — [Tom Looman, UE 5.6 performance highlights](https://tomlooman.com/unreal-engine-5-6-performance-highlights/)
- **GPU grass in PCG.** "GPU Grass & Micro Scattering" in PCG GPU compute samples the Landscape RVT and grass maps directly on the GPU for runtime grass spawning. This effectively moves Unreal toward the HZD-style GPU-runtime model — [search summary of 5.7 notes via Digital Production / 80.lv](https://digitalproduction.com/2025/10/17/unreal-5-7-preview-pcg-grows-up-foliage-gets-fancy/)
- **Generation modes.** The docs describe separate PCG generation modes (on load, on demand, runtime) — [Using PCG Generation Modes (UE 5.8 docs)](https://dev.epicgames.com/documentation/unreal-engine/using-pcg-generation-modes-in-unreal-engine)
- **Runtime cost complaint on PS5 (Jan–Feb 2026).** A studio ran hierarchical runtime PCG with 16/32/64 m grid levels, one graph per cell and point clouds precomputed offline. It still consistently blew its 500 µs CPU budget, with ~1 ms spikes. Hot spots were `FPCGGraphExecutor::PrepareForExecute` and `IPCGElement::GetDependenciesCRC` (up to a third of the time).
  - Epic's Julien L'Heureux first suggested disabling the PCG cache (`pcg.cache.enabled 0`, 9 Jan 2026).
  - He later suggested (20 Feb 2026): trimming attributes before GPU upload, loading shared GPU data once, using FastGeo Interop to avoid component/actor overhead, and throttling the runtime scheduler. He promised more optimizations in 5.8.
  - [Epic forums: Runtime Cost in PCG Hierarchical Generation](https://forums.unrealengine.com/t/runtime-cost-in-pcg-hierarchical-generation/2708776)
- **Other runtime/partition bugs.** Partitioned PCG components did not generate on demand at runtime [Epic forums](https://forums.unrealengine.com/t/partitioned-pcg-component-does-not-generate-on-demand-and-other-bugs/2220564). Packaged builds with runtime PCG froze ("Not Responding") [Epic forums](https://forums.unrealengine.com/t/using-pcg-at-runtime-high-chance-it-gets-stuck-at-not-responding-on-packaged-build-how-to-fix/1943620). World Partition + hierarchy caused crashes and data loss [Epic forums](https://forums.unrealengine.com/t/critical-errors-with-pcg-world-partition-and-hierarchy-crashing-and-improper-detachment-with-data-loss/2311614).
- **Determinism and seeds.** PCG seeds come from the component seed plus per-node settings; the API exposes a `GetSeed` function [UE 5.8 docs: Get Seed](https://dev.epicgames.com/documentation/unreal-engine/BlueprintAPI/PCG/Random/GetSeed). Community troubleshooting lists three causes of non-deterministic output: unseeded or time-based random nodes, execution order, and cross-platform float differences. Its fix is to set every node's seed to "From Component" [Bugnet blog, community source](https://bugnet.io/blog/fix-unreal-pcg-determinism-different-results). (Secondary source; I found no Epic doc guaranteeing cross-platform bitwise determinism.)
- **Landscape Grass internals.** `ULandscapeSubsystem` ticks grass. Each grass variety per component subsection becomes its own HISM component, built asynchronously by `FAsyncGrassBuilder` (instance buffers + cluster tree).
  - Throttles: `grass.TickInterval` (default 1; Fortnite uses 10), `grass.MaxCreatePerFrame` (default 1 HISM/frame) and `grass.MaxAsyncTasks`. The throttles exist because HISM creation is a CPU/hitch cost.
  - Placement is a jittered grid or a Halton sequence.
  - [Spacerad.io source analysis, May 2023, updated Feb 2026](https://spacerad.io/posts/a-look-under-the-hood-at-unreal-engine-landscape-grass-en)
- **Grass scalability.** A grass type can opt in or out of `grass.DensityScale` (scalability). Epic's docs say to opt out when the grass matters for gameplay (e.g., stealth cover) — [LandscapeGrassType Python API docs 5.4](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/LandscapeGrassType?application_version=5.4)
- **Nanite Foliage (UE 5.7, experimental, off by default).** It has three parts:
  - **Assemblies:** micro-instancing, up to 65k part instances per assembly. The Witcher 4 demo's biggest tree went from 3.5 GB on disk to 29 MB, and streaming memory for one tree from ~36 MB to 2.7 MB.
  - **Voxels:** near-pixel-sized voxels at distance replace LOD meshes and keep the tree's volumetric look.
  - **Skinning:** bone animation replaces WPO; ~100k bones updated in ~0.1 ms.
  - Limitations: WPO, alpha masking and full-triangle leaves go against its design. The Dynamic Wind plugin supports only a global wind direction, has no player/object collision and imports only from JSON. Animation must be disabled at distance for Virtual Shadow Map cost.
  - [UE docs: Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
- **Nanite Foliage demos.** Epic claims "dense foliage at 60 fps on current gen hardware" [Digital Production](https://digitalproduction.com/2025/10/17/unreal-5-7-preview-pcg-grows-up-foliage-gets-fancy/). One community test went from 62 to 119 fps with 77,376 trees of 20M polys each [80.lv](https://80.lv/articles/get-a-glimpse-of-nanite-foliage-with-voxel-representation-in-ue5-7). Another scatters ~200 trillion polygons with PCG over a 16K landscape [80.lv](https://80.lv/articles/200-trillion-polygons-scene-with-nanite-foliage-voxelization-in-ue5-7).
- **Procedural Vegetation Editor (5.7, experimental).** A PCG-node-based plugin to "grow, sculpt, and vary Nanite-ready foliage in real time inside Unreal". It replaces round-trips to SpeedTree-style external tools — [Digital Production](https://digitalproduction.com/2025/10/17/unreal-5-7-preview-pcg-grows-up-foliage-gets-fancy/)
- **Rendering vs placement.** Nanite Foliage changes rendering but not placement logic, and HISM stays the standard substrate for instanced foliage — [StraySpark blog (low-authority commercial blog)](https://strayspark.studio/blog/ue57-nanite-foliage-vs-hism-vs-pcg-benchmark)

### Inferences
- Epic's trajectory from 2017 to 2026: offline baked foliage (Foliage mode / Procedural Foliage Spawner / HISM) → PCG graphs on CPU → GPU PCG with runtime grass. This is the same arc Guerrilla described for Horizon Zero Dawn in 2017: CPU bakes too slow → GPU → fully runtime.
- The persistent pain is not the placement math. It is the per-object engine overhead (actors, components, HISM trees, caches). A small custom engine avoids this by writing points straight into GPU instance buffers.
- Nanite Foliage targets photoreal alpha-free dense trees. It is irrelevant to a flat-shaded low-poly style, except for one idea: geometry instead of alpha cards avoids overdraw.

### Gaps
- No authoritative Epic statement found on the formal status (deprecated or not) of the legacy Procedural Foliage Spawner in 5.7/5.8.
- No first-party numbers on the HISM rebuild cost of Foliage mode.
- No Epic statement on cross-platform bitwise determinism of PCG.
- Not verified: Witcher 4's actual use of PCG in production, beyond the demo marketing.

## Unity: terrain details, third-party fillers, Unity 6

### Takeaway
Unity's built-in terrain detail system is simple: painted density plus Perlin noise, with GPU instancing only in "Instanced Mesh" mode (batches of ≤1023). It has a long history of forum complaints about FPS collapses. The ecosystem relied on paid assets (Vegetation Studio Pro, now deprecated; GPU Instancer; Nature Renderer). Unity 6's GPU Resident Drawer (BRG-based instancing of GameObjects) helps GameObject-placed vegetation, but it is not a scatter/rules system.

### Cited Findings
- **Detail render modes (Unity 6.x manual).**
  - Instanced Mesh (recommended; GPU instancing) — "Objects are rendered in batches of 1,023 or fewer", and there is no instanced light-probe or lightmap support.
  - Vertex Lit Mesh (combined meshes, no instancing), Grass Mesh, and Grass Texture (quads, optional billboards).
  - Placement randomness is Perlin noise with a noise seed and spread.
  - [Unity Manual: Grass and other details](https://docs.unity3d.com/Manual/terrain-Grass.html)
- **Long-running performance complaints.** Examples: "Terrible grass performance" [Unity forum 2018](https://forum.unity.com/threads/terrible-grass-performance.534405/); "Extreme lag with grass" [Unity Discussions 2023](https://discussions.unity.com/t/extreme-lag-with-grass/932907); "Improve grass rendering FPS" [Unity Discussions 2024](https://discussions.unity.com/t/improve-grass-rendering-fps/941150). Search snippets report drops from ~100–150 fps to ~30 fps once detail grass is added. These are user reports and hardware-dependent.
- **GPU Resident Drawer (Unity 6).** Uses the BatchRendererGroup API to draw GameObjects with GPU instancing automatically, cutting draw calls and CPU time — [Unity 6 manual: GPU Resident Drawer (URP)](https://docs.unity3d.com/6000.0/Documentation/Manual/urp/gpu-resident-drawer.html). A community demo used a 35k-object vegetation scene [Knights of U](https://theknightsofu.com/boost-performance-of-your-game-in-unity-6-with-gpu-resident-drawer/).
- **Vegetation Studio Pro.** Its integration is marked deprecated in a major shader vendor's docs [Staggart Stylized Grass Shader docs](https://staggart.xyz/unity/stylized-grass-shader/sgs-docs/?section=third-party-integrations). It was built on Jobs/Burst [Unity forum release thread](https://forum.unity.com/threads/released-vegetation-studio-pro.538451/).
- **Nature Renderer.** It takes over terrain-detail rendering. Without the shader integration, grass "will appear to render in front of the camera and with severely poor performance". GPU Instancer likewise hooks the terrain detail system — [Staggart docs](https://staggart.xyz/unity/stylized-grass-shader/sgs-docs/?section=third-party-integrations)

### Inferences
- In Unity the pattern is "built-in = authoring (paint density maps); third party = actually fast rendering (GPU culling + indirect instancing)". The lesson: the density-map authoring model is fine, and indirect GPU instancing with GPU culling is the minimum viable renderer.

### Gaps
- I did not find a recent (2025–2026) Unity staff forum post that explicitly acknowledges terrain-detail performance problems or announces a new vegetation/scatter system.
- Not verified whether Unity 6's GPU Resident Drawer applies to terrain details (vs GameObjects only).

## Godot: MultiMesh and community scatter addons

### Takeaway
Godot has MultiMesh/MultiMeshInstance3D as the instancing primitive but no built-in scatter or foliage-painting system. Users depend on community addons (ProtonScatter, Spatial Gardener, MultiMesh Scatter, and others). As of spring 2026 these addons often break on new Godot versions or perform poorly.

### Cited Findings
- **ProtonScatter** (Godot 4). A modifier stack in the inspector (Blender-like): some modifiers create points, others transform them. Shapes are Box, Sphere and Path — [GitHub HungryProton/scatter](https://github.com/HungryProton/scatter)
- **Spatial Gardener.** Paints plants and props on arbitrary 3D surfaces, "without having to use heightmap terrain or writing procedural placement algorithms" — [Godot Asset Library](https://godotengine.org/asset-library/asset/2037)
- **MultiMesh Scatter.** Random placement aligned to the surface normal, with collision layers and random scale/rotation — [Godot Asset Library](https://godotengine.org/asset-library/asset/1566)
- **Addon state on Godot 4.6 (Mar–Apr 2026).**
  - Spatial Gardener needed a patch for a "Logger" name conflict.
  - ProtonScatter had "pretty bad" performance for the poster.
  - Multimesh+ was error-prone.
  - Scatterbox was unmaintained, and its painting worked only after disabling threaded 3D physics (raycasts failed).
  - [Godot Forum: Mesh scatter/paint tools for Godot 4.6](https://forum.godotengine.org/t/mesh-scatter-paint-tools-for-godot-4-6/135963)

### Inferences
- The Godot experience shows the cost of treating scatter as an editor addon rather than an engine subsystem: it breaks across versions and depends on physics raycasts for placement. A procedural valley generator should place against its own heightfield, not physics raycasts.

### Gaps
- No official Godot roadmap statement on built-in scatter/foliage found.

## AAA in-house pipelines (published talks)

### Takeaway
The most transferable reference is Horizon Zero Dawn (GDC 2017). Its model:

- Artists paint 2D world-data maps and build node-graph "ecotope" logic.
- A compute shader evaluates density maps per tile.
- Ordered dithering with a precomputed blue-noise-like point pattern turns density into points; the pattern is scaled to each asset's footprint.
- Collision between same-footprint layers is solved by "layered dithering" (stacked density ranges).
- Everything is deterministic and locally stable, and generated at runtime in ~250 µs of GPU per frame.

Far Cry 5 (GDC 2018) is the baked, Houdini-based counterpart: a viability/priority/radius competition between species, and deterministic nightly bakes. Ghost of Tsushima (GDC 2021) is the reference for GPU-generated grass blades. Witcher 3 (GDC 2014) mixed prebaked trees with on-the-fly grass driven by terrain materials.

### Cited Findings

**Horizon Zero Dawn — Jaap van Muijden, GDC 2017** (all points from the slides and speaker notes: [Guerrilla publication page with PDF/PPTX](https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn), [GDC Vault](https://gdcvault.com/play/1024700/GPU-Based-Run-Time-Procedural)).
- **Goals.** "Quick iterations, Large variety, Believable look, Art Directable, Data driven, Deterministic, Locally stable". The art director wanted "to be able to freely move mountains, rivers and gameplay without the need to continually redress the world".
- **Why runtime.** They started with CPU offline bakes. On Killzone Shadowfall "the bake times were a big problem and iteration was slow". A GPU prototype was so fast that they went fully real-time, which "would not only remove bakes altogether, but ... reduce the amount of data we would have to store, and stream from disk".
- **Scale.** 500+ asset types and ~100,000 placed meshes around the player. The system grew to place effects, pickups (gameplay) and wildlife, and it drives sound and weather. It costs ~250 µs average busy GPU load. The nature assets were made by 3 people and the ecotopes by 1 person.
- **WorldData.** A collection of 2D maps, streamed in sections, all generated (World Machine/Houdini) and all paintable, at ~4 MB/km² (~32 bits/m²). Mostly BC7, at 0.5–4 m/px. Examples:
  - Placement_Trees 1 m (hand-painted after a World Machine bake)
  - Height_Terrain / Height_Objects / Height_Water 0.5 m
  - Erosion_Flow/Deposition, Terrain_Cavity, Water_Flow
  - Topo_Roads and Topo_Objects (generated from roads and hand-placed objects, so procedural content reacts to them)
- **Logic.** Node networks like Nuke/Substance produce one density map per asset layer. Example: Placement_Trees × (1-objects) × (1-water) × (1-roads). Graphs are shared between ecotopes. Artists encode extra meaning in a map: values near 0 give lush "edge trees" and values near 1 give tall branchless "inner trees".
- **Ecotope asset tree.** Each asset has a footprint (effective diameter): trees ~6 m, undergrowth ~1 m. Density propagates down the hierarchy, and "clearing" maps are applied through inverse nodes.
- **Compilation.** Graphs flatten into per-asset layers and compile to an intermediate form. That form becomes either a compute shader or input to a GPU interpreter used for step-debugging in-game. "Author driven merging" cuts layers around the player "from several thousands, to several hundreds".
- **Tiling.** Granularity scales with footprint: trees in 128×128 m blocks, grass in 32×32 m blocks. Density maps are always 64×64 px per block.
- **Discretization (GENERATE).** Ordered dithering, but with "a nicely sorted set of explicit positions, each with its own implicit threshold". The pattern is a disk packing whose thresholds are evenly spread in [0,1] and ordered to maximize distance between consecutive thresholds, then scaled so spacing = footprint.
  - This gives a guaranteed minimum distance between trees, which "guarantees proper navigation for the player, as well as the enemies".
  - One thread per sample: range test, threshold test, height sample, normal, then append via group-local memory. ~10 µs.
- **Acknowledged artifact.** At high density trees "line up" in visible patterns: "once you see it you cannot un-see it". They added a user-defined noise offset; multiple stencils or Wang tiling were planned but not needed.
- **PLACEMENT step.** Rotation, tilt and scale from an RNG keyed on pattern point index + tile ID, "so each placement has a fully deterministic ID". The only non-determinism was GENERATE's output order. ~7 µs.
- **Collision.** Read-back approaches break determinism and cause GPU flushes; that path "never made it into the game". Instead, layered dithering: layers with the same footprint stack their densities into [min,max] ranges, so each sample point selects at most one asset. It works as a probability distribution over assets.
  - Production ecotopes often stack >20 same-footprint layers. Density maps must be computed for non-placing layers too, so layers are ordered heuristically.
  - Collision across different footprints was not solved generally.
- **Scheduling.** The pipeline is instantiated 64× and parallelized spatially. They shipped with at most 4 emits per frame "to reduce memory load and prevent GPU spikes", with one batched copy to the CPU.
- **Conclusion slide.** "Unpolished areas in shippable quality".

**Ghost of Tsushima grass — Eric Wohllaib, GDC 2021 Advanced Graphics Summit** ([GDC Vault](https://gdcvault.com/play/1027033/Advanced-Graphics-Summit-Procedural-Grass), [YouTube](https://www.youtube.com/watch?v=Ibe1JBF5i5Y))
- Individual blades are generated on the GPU, each with its own procedural shape and animation. The goal was acres of grass within memory and performance limits — [GDC Vault abstract](https://gdcvault.com/play/1027033/Advanced-Graphics-Summit-Procedural-Grass)
- Details from a secondary write-up [tigerabrodi.blog](https://tigerabrodi.blog/grass-in-ghost-of-tsushima) (not first-party; verify against the talk):
  - Each blade is a cubic Bézier with tilt/bend/facing parameters, 15 vertices near and 7 far; beyond that grass becomes terrain texture.
  - A compute shader places blades on a jittered grid, samples terrain textures for grass type and height, does frustum/distance culling and samples a displacement buffer for character interaction.
  - Voronoi "clumps" drive height, colour and lean.
  - View-space thickening avoids paper-thin edge-on blades.
  - Wind is global Perlin noise plus per-blade phase-offset bobbing.
  - Instanced indexed draws with no vertex streams; vertices are derived from index and instance ID.
- Grass tiles are subdivided and streamed with a double-buffer strategy — [search summary citing the talk](https://gdcvault.com/play/1027033/Advanced-Graphics-Summit-Procedural-Grass). Related Sucker Punch talk on streaming: [Building and Loading Ghost of Tsushima, GDC 2021 PDF](https://media.gdcvault.com/GDC+2021/ghost_streaming_gdc2021.pdf)

**Far Cry 5 — Etienne Carrier, GDC 2018 / Houdini HIVE** ([GDC Vault](https://www.gdcvault.com/play/1025557/Procedural-World-Generation-of-Far); notes: [Christian Mills](https://christianjmills.com/posts/procedural-tools-far-cry-5-notes/); [80.lv](https://80.lv/articles/houdini-procedural-world-generation-of-far-cry-5))
- **Motivation.** The terrain changed constantly over 2.5 years of development, so hand-placed content "became incoherent with each terrain iteration" and repainting was tedious — [80.lv](https://80.lv/articles/houdini-procedural-world-generation-of-far-cry-5)
- **Inputs.** Artists paint sub-biomes (forest, grassland). The tools compute abiotic maps: occlusion, flow, slope, curvature, illumination, altitude, latitude/longitude, wind.
- **Species competition.** Each species has a viability score from its favoured attributes. The highest accumulated viability wins. Priority lets trees dominate within their radius while still allowing bushes closer. A viability radius excludes other species.
- **Size variation.** Size is tied to viability, and size variants (e.g. 50/40/30 m conifers) are assigned by viability range, which gives natural tapering at forest edges. A density ramp controls counts, both for overlap and for performance.
- **Integration with hand-made content.** Roads and splines procedurally clear vegetation; the biome tool automatically clears vegetation from power lines.
- **Determinism and baking.** Generation is deterministic, with "identical results with the same inputs, regardless of the build machine". Nightly build machines re-bake map sections. Bake scope can be the whole world, a local section or the visible frustum.
- **Outputs.** Entity point clouds, terrain texture IDs, heightmap, terrain colour, and a forest mask used by fog and other tools.
- **Lessons.** "Excessive automation can lead to issues, while excessive manual control can be time-consuming". Sometimes manual control is preferred (e.g. riverbed carving).
- All points above from [Christian Mills' notes](https://christianjmills.com/posts/procedural-tools-far-cry-5-notes/) unless linked otherwise.

**The Witcher 3 (REDengine 3) — Marcin Gollent, GDC 2014** ([slides PDF](https://media.gdcvault.com/GDC2014/Presentations/Gollent_Marcin_Landscape_Creation_and.pdf); [archive.org transcript](https://archive.org/stream/GDC2014Gollent/GDC2014-Gollent_djvu.txt))
- **Two paths.** Vegetation uses "on-the-fly distribution" plus an "offline vegetation generator". SpeedTree's Forest library handles culling and LOD: one shared culling grid for trees, and a separate grid per grass layer with its own cell size and draw distance.
- **Grass/debris.** "Vegetation brushes" (sets of types with densities and scales) are assigned to terrain materials, so grass follows the terrain paint automatically; ~10 types were auto-distributed.
- **Occurrence maps.** A prebaked occurrence bitmap per grass type (125 kB per map at 100 m² cells on a 10 km² world) lets empty cells be skipped.
- **Cost.** "Doesn't exceed the cost of 1ms" in the cooked game, but "hits the editor performance badly when going overboard" with auto-distributed types.
- **Acknowledged problem.** "Grass doesn't 'sit' on the terrain … takes a lot of artist's time to hide it … critical when populating procedurally".

**Tiny Glade (Pounce Light; Bevy/Rust, custom Vulkan renderer)**
- **Authored rules.** Anastasia Opara: "I create every single rule that goes into the system... we curate exactly the experience you will get out of it". Procedural tools are validated by "how many stories they can tell" — [80.lv interview](https://80.lv/articles/exclusive-tiny-glade-developers-discuss-bevy-proceduralism-publishers-cozy-games)
- **Engine.** They started on Bevy's standard rendering but moved to a modified Bevy with custom Vulkan integration because procedural and graphics needs exceeded Bevy at the time. Vulkan validation layers were a key reason to leave OpenGL. Rust "doesn't crash in your face" — [80.lv interview](https://80.lv/articles/exclusive-tiny-glade-developers-discuss-bevy-proceduralism-publishers-cozy-games)
- **Iteration.** "Some of Tiny Glade's features have gone through five or six iterations ... simple surface-level suggestions ... turn out to be not that simple when plugged into the crazy procedural machinery" — [80.lv interview](https://80.lv/articles/exclusive-tiny-glade-developers-discuss-bevy-proceduralism-publishers-cozy-games)
- **Background.** Anastasia worked on procedural tools and Tomasz on rendering. Both previously worked at Embark, EA and Creative Assembly — [Wikipedia: Tiny Glade](https://en.wikipedia.org/wiki/Tiny_Glade)

### Inferences
- **The dominant AAA pattern** is: paintable 2D maps → per-asset density graph → deterministic discretization with minimum spacing (footprint) → instance buffers. HZD runs this at runtime on the GPU; FC5 bakes it offline in Houdini; W3 mixes both. Every one of them keeps artist override maps (paint/clear) on top of procedural inputs. "Art-directable" is the recurring praise, and "monotonous/robotic" is the recurring fear.
- **For a deterministic Rust engine**, HZD's method fits almost directly on the CPU at valley scale:
  - Fixed per-footprint point patterns (Poisson-disk/blue noise with sorted thresholds) tiled in world space.
  - An RNG keyed by (tile ID, pattern index) for per-instance transforms.
  - Density = product/curves of heightfield-derived maps (slope, altitude, flow, curvature, distance to paths).
  - This is order-independent and "locally stable": changing one area does not reshuffle the rest.
  - Using integer or fixed-point threshold comparisons avoids cross-platform float divergence. That point is my inference; HZD did not discuss cross-platform issues.
- **Layered dithering** cleanly solves "trees vs rocks vs bushes of the same footprint" without read-backs. FC5-style viability/priority/radius is the alternative for mixed footprints, but it is sequential.
- **Gameplay collision.** HZD's footprint spacing gave guaranteed navigability between trees. Its Topo_Roads/Topo_Objects masks, and FC5's spline clearing, keep paths free of trees.

### Gaps
- Red Dead Redemption 2 and Assassin's Creed: I found no public talk with concrete vegetation placement details in this pass.
- Ghost of Tsushima numbers (blade counts, ms, memory) come from a secondary blog. The primary video was not transcribed.
- Tiny Glade: no public technical write-up found on how its grass, trees or ground cover are placed or rendered. The interview covers philosophy and engine, not the scatter algorithm.

## Common problems and acknowledged pain points (cross-cutting)

### Takeaway
The recurring problems, in order of how often developers raise them:

1. Artist control vs randomness, and visible patterns.
2. Iteration and bake time.
3. Per-instance engine overhead and hitches in runtime generation.
4. Grass not "sitting" on terrain / looking procedural.
5. Memory and streaming of baked instances.
6. Gameplay conflicts: paths, cover, navigation.
7. Determinism.

Overdraw from alpha cards and LOD popping are the rendering-side issues that Nanite Foliage and GoT's geometric blades try to eliminate.

### Cited Findings
- **Artist control and patterns.** HZD: dithering produced visible tree lines at high density, fixed with a noise offset [Guerrilla slides](https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn). "Historically, procedural systems have often looked monotonous, bland and robotic" [same notes]. FC5: too much automation vs too much manual control [Christian Mills notes](https://christianjmills.com/posts/procedural-tools-far-cry-5-notes/).
- **Iteration and bake time.** Killzone Shadowfall bake times were "a big problem" and drove HZD to runtime GPU [Guerrilla](https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn). FC5 needed nightly build-machine bakes [Christian Mills](https://christianjmills.com/posts/procedural-tools-far-cry-5-notes/). The W3 editor slowed down when "going overboard" [Gollent transcript](https://archive.org/stream/GDC2014Gollent/GDC2014-Gollent_djvu.txt).
- **Runtime overhead and hitches.** UE PCG runtime blew a 500 µs PS5 CPU budget [Epic forums 2026](https://forums.unrealengine.com/t/runtime-cost-in-pcg-hierarchical-generation/2708776). UE grass throttles HISM creation to 1 per frame by default [Spacerad.io](https://spacerad.io/posts/a-look-under-the-hood-at-unreal-engine-landscape-grass-en). HZD capped emits at 4 per frame "to prevent GPU spikes" [Guerrilla](https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn).
- **Grass not sitting on terrain.** A W3 slide: "conventional problem of grass standing out visually… critical when populating procedurally" [Gollent transcript](https://archive.org/stream/GDC2014Gollent/GDC2014-Gollent_djvu.txt).
- **Memory of baked instances vs maps.** HZD stores maps at ~4 MB/km² instead of instance lists [Guerrilla](https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn). In the Witcher 4 demo, Nanite Assemblies cut the biggest tree from 3.5 GB to 29 MB [UE docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage).
- **Gameplay.** HZD's footprint spacing guarantees navigation, and the Topo_Roads/Objects masks keep content off roads and objects [Guerrilla](https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn). UE grass density scaling must be turned off for gameplay-relevant grass so low-spec players don't lose cover [UE API docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/LandscapeGrassType?application_version=5.4). FC5 auto-clears vegetation along roads and power lines [Christian Mills](https://christianjmills.com/posts/procedural-tools-far-cry-5-notes/).
- **Determinism.** HZD: everything is deterministic except GENERATE's output order, fixed with a per-point ID [Guerrilla](https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn). FC5: identical results regardless of build machine [Christian Mills](https://christianjmills.com/posts/procedural-tools-far-cry-5-notes/). UE PCG: non-determinism from unseeded nodes and execution order is a common community troubleshooting topic [Bugnet](https://bugnet.io/blog/fix-unreal-pcg-determinism-different-results).
- **Overdraw, alpha and LOD.** Nanite Foliage explicitly treats alpha masking and WPO as incompatible with its design and replaces LOD meshes with voxels [UE docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage). GoT uses geometric blades with a smooth LOD transition to avoid popping [tigerabrodi.blog (secondary)](https://tigerabrodi.blog/grass-in-ghost-of-tsushima).
- **Addon fragility.** Godot scatter addons break across engine versions and depend on the physics-raycast threading setting [Godot Forum 2026](https://forum.godotengine.org/t/mesh-scatter-paint-tools-for-godot-4-6/135963).

### Inferences
- **A flat-shaded low-poly style sidesteps the biggest rendering pain**, alpha-card overdraw: use opaque geometric grass blades or tufts and low-poly trees. What remains is instance count, culling and LOD/fade. GoT-style procedural blade generation from index/instance ID, plus distance-based density thinning, matches the style well.
- **A deterministic simulation needs two things kept separate:**
  1. Gameplay-relevant placement (trees, rocks that block movement). Generate it deterministically on the CPU from the seed plus the heightfield, with integer or fixed-point comparisons, and use it for collision and navigation.
  2. Purely visual grass. It can be GPU-generated and non-authoritative.
  - HZD's density → dither → per-point-ID RNG pipeline gives local stability for free.
- **The small-engine answer to the "artist control" complaint** is paintable override masks (forest / clearing / path) layered multiplicatively on top of procedural maps. HZD, FC5 and W3 all converged on this.

### Gaps
- No quantitative public data found on overdraw costs of alpha-tested foliage in shipped titles (beyond general statements).
- No first-party postmortem found on LOD popping specifically for procedural vegetation.
