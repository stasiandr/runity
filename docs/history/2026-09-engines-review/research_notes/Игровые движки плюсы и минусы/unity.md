# Unity: strengths, weaknesses, community sentiment and Unity's own admissions (as of Sept 2026)

## 1. What do developers consistently praise?

### Takeaway
Praise centres on the ecosystem rather than the core tech: the Asset Store, mobile and cross-platform deployment, 2D tooling, C# and the inspector workflow, a large hiring pool, and access to console/Apple platforms. Unity is still the default engine for established indie studios (54% of them), even though it lost the overall top spot to Unreal in 2026.

### Cited Findings
- GDC 2026 State of the Game Industry (2,300+ respondents): 54% of developers at older/established indie studios still use Unity as their primary engine. — [GDC](https://gdconf.com/article/gdc-2026-state-of-the-game-industry-reveals-impact-of-layoffs-generative-ai-and-more/)
- Commentary (secondary/blog source, treat as opinion): "The Asset Store is still the largest commercial marketplace for indie-usable components"; "Unity's mobile build pipeline, asset compression, and deployment tooling are still the strongest in the industry"; Unity's 2D tooling (sprites, Tilemap, 2D physics) is "competitive with Godot and ahead of Unreal"; "Unity developers remain easier to hire than Godot or Bevy developers." — [StraySpark blog, 2026](https://www.strayspark.studio/blog/unity-engine-2026-state-comeback-runtime-fee-aftermath)
- Hacker News commenters, even critical ones, credit Unity with solving real problems such as access to Apple and Nintendo platforms. They praise the inspector as a flexible immediate-mode UI and call the Asset Store "gold", while saying the core tech "feels less refined". — [HN thread "Unity's Mono problem"](https://news.ycombinator.com/item?id=46414819)
- Some Unity Discussions users welcomed Unity's 2026 decision to consolidate onto URP: "I feel this is the right direction" (KimmoFactor). — [Unity Discussions, Render Pipelines strategy for 2026](https://discussions.unity.com/t/render-pipelines-strategy-for-2026/1710004/print)
- Unity's 2025 "Production Verification" programme with studios (e.g. Kinetic Games, Ten Chambers) reportedly cut regression-fix time by 43% and reduced the backlog by 54%. — [Digital Production, Unite 2025 recap](https://digitalproduction.com/2025/11/26/unitys-2026-roadmap-coreclr-verified-packages-fewer-surprises/)
- Unity 6 (released 17 Oct 2024) introduced a predictable cadence: several "Update" releases a year plus one LTS a year, each LTS supported for 2 years (plus 1 more for Enterprise/Industry). Unity 6.0 LTS is supported to Oct 2026 and 6.3 LTS to Dec 2027. — [Unity releases/support](https://unity.com/releases/unity-6/support); [CG Channel](https://www.cgchannel.com/2024/10/unity-releases-unity-6/); [Unity 6.3 LTS blog](https://unity.com/blog/unity-6-3-lts-is-now-available)

### Inferences
- Unity's lasting advantage is its installed base: Asset Store, tutorials, hiring pool, mobile pipeline and console support. That base keeps mid-size indies and mobile studios on Unity despite the loss of trust.
- The mid-2020s "stability first" messaging is itself a response to complaints (see sections 2–3).

### Gaps
- I found no strong primary survey data (e.g. a Unity-specific developer-satisfaction survey) that quantifies praise. The specific praise points above come from a blog and HN comments. The learning-resources and C#-accessibility points are widely held but I did not source them directly beyond HN.

## 2. Top recurring complaints

### Takeaway
The complaints fall into four groups:
- **Trust and licensing:** the 2023 Runtime Fee, later cancelled.
- **Fragmentation and half-finished features:** URP/HDRP/Built-in, DOTS, Netcode, UI Toolkit vs UGUI.
- **An outdated scripting runtime:** Mono/Boehm GC and slow domain reloads, with the CoreCLR move promised since 2022.
- **Corporate instability:** repeated layoffs, including cuts to feature teams.

### Cited Findings
**Runtime Fee (historical, 2023–2024; resolved by cancellation)**
- 12 Sep 2023: Unity announced a per-install "Runtime Fee", which drew industry-wide backlash. — [Dexerto](https://www.dexerto.com/gaming/unity-boss-shares-updates-to-runtime-fee-amid-controversy-i-am-sorry-2306028/); [Kotaku](https://kotaku.com/unity-engine-runtime-fees-install-changes-devs-1850865615)
- 22 Sep 2023: Marc Whitten's open letter: "I am sorry. We should have spoken with more of you and we should have incorporated more of your feedback before announcing our new Runtime Fee policy." The revision kept Personal free with no fee, raised the Personal cap from $100K to $200K, dropped the mandatory splash screen, and exempted games under $1M trailing-12-month revenue. — [Dexerto](https://www.dexerto.com/gaming/unity-boss-shares-updates-to-runtime-fee-amid-controversy-i-am-sorry-2306028/); [The Register](https://www.theregister.com/software/2023/09/23/unity-apologizes-announces-revised-runtime-fee-criteria/325218); [GeekWire](https://www.geekwire.com/2023/unity-exec-marc-whitten-shares-apology-letter-following-backlash-to-controversial-licensing-policies/)
- 12 Sep 2024: CEO Matt Bromberg cancelled the Runtime Fee for games "effective immediately" and returned to seat-based subscriptions. He wrote: "We can't pursue that mission in conflict with our customers; at its heart, it must be a partnership built on trust." — [Unity blog](https://unity.com/blog/unity-is-canceling-the-runtime-fee)
- At the same time prices went up. Pro rose 8% to $2,200/seat/year and Enterprise rose 25% (companies above $25M revenue), both effective 1 Jan 2025. The Personal cap was raised to $200K. Unity also committed to review prices only once a year and to let users keep the ToS of the version they use. — [Unity blog](https://unity.com/blog/unity-is-canceling-the-runtime-fee); [CG Channel](https://www.cgchannel.com/2024/09/unity-scraps-controversial-runtime-fee-but-raises-prices/)

**Render pipeline fragmentation (ongoing; being addressed 2026)**
- Community complaints in response to Unity's 2026 strategy:
  - stonstad: "BIRP + DX11 is significantly faster than URP + DX12... I don't feel I have the performance budget to take a 10 FPS hit."
  - themeowrphy: "URP is never gonna be on par with HDRP."
  - Genebris called consolidation without feature parity "outrageous".
  - Hannah_Dawson noted that VRChat and legacy user-generated-content games cannot realistically migrate.
  - ferretnt: "If there is ever a forced migration of some other major system at the same time... we would seriously evaluate a broader engine migration."
  - Developers also reported that URP builds are slower than Built-in, and that shader authoring, GrabPass and multi-pass shaders lack parity with Built-in.
  - Source for all of the above: [Unity Discussions](https://discussions.unity.com/t/render-pipelines-strategy-for-2026/1710004/print)
- Outside commentary: "Unity has not shipped a single unified pipeline; URP and HDRP remain distinct." It also says there is no modern terrain replacement and that UGUI has not been fully displaced by UI Toolkit. — [StraySpark](https://www.strayspark.studio/blog/unity-engine-2026-state-comeback-runtime-fee-aftermath)
- GameFromScratch headline on the HDRP decision: "The Unity HDRP is Dead." — [GameFromScratch](https://gamefromscratch.com/the-unity-hdrp-is-dead/)

**Mono / GC / CoreCLR delays / domain reload (ongoing; fix scheduled for Unity 7.0 / 2027)**
- A developer blog post discussed on HN (late 2025) reported the same simulation code running about 3x faster on modern .NET than on Unity's Mono. Map generation took 80 s on Mono versus 10–30 s on .NET. — [HN](https://news.ycombinator.com/item?id=46414819)
- Recurring HN complaints:
  - Unity still uses the conservative Boehm GC.
  - IL2CPP produces "low-quality C++".
  - The Burst/HPC# restrictions are too tight.
  - Leadership feels "rudderless", with features "started but abandoned".
  - The CoreCLR migration has been delayed for years; xoofx led early work and later left.
  - The subscription model drives rushed quarterly releases.
  - Representative quote: "Unity's whole shtick is that they make something horrible, then improve upon it marginally." — [HN](https://news.ycombinator.com/item?id=46414819)
- Unity's plan to move from Mono to CoreCLR was already being discussed publicly in May 2022. — [HN 2022 thread](https://news.ycombinator.com/item?id=31425546)
- Compile and domain-reload times cost large projects hours per developer per week (secondary claim). — [Ocean View Games](https://oceanviewgames.co.uk/blog/posts/unity-path-to-coreclr)

**Networking (Netcode) (ongoing)**
- Dec 2025 Unity Discussions thread "More Agony with Netcode for GameObjects" (NGO 2.7.0):
  - The user reported inconsistent sync and raycasts, and outdated or conflicting documentation, and said NGO compares poorly to Photon.
  - They switched to Photon Fusion.
  - A Netcode team member (emmcmill) replied with debugging advice.
  - Source: [Unity Discussions](https://discussions.unity.com/t/more-agony-with-netcode-for-gameobjects/1700880)
- Users asked for a Netcode for Entities roadmap, saying: "The absence of a clear roadmap makes it very difficult to plan projects." — [Unity Discussions](https://discussions.unity.com/t/request-for-a-netcode-for-entities-roadmap/1586775)

**UI Toolkit vs UGUI**
- An indie devlog describes replacing UI Toolkit because maintenance became laborious and there were issues with visibility and animation. — [itch.io devlog](https://itch.io/devlog/943542/quick-update-of-behind-the-scenes-replacing-ui-toolkit.amp)
- Unity keeps extending UI Toolkit (world-space UI, custom shaders, vector graphics in 6.3), so the two UI systems still coexist. — [Digital Production](https://digitalproduction.com/2025/11/26/unitys-2026-roadmap-coreclr-verified-packages-fewer-surprises/)

**Abandoned features / layoffs affecting product**
- In February 2025, layoffs eliminated the whole Behavior package team. Notices came as 5am emails from a no-reply address, which staff described as "abrupt and impersonal". — [Game Developer](https://www.gamedeveloper.com/business/report-unity-continues-layoffs-with-abrupt-communications-and-5am-emails); [PocketGamer.biz](https://www.pocketgamer.biz/unity-lays-off-staff-and-axes-behavior-team/)
- Unity paused work on new animation and world-building workflows to focus on stability and CoreCLR (Unite 2025). — [Digital Production](https://digitalproduction.com/2025/11/26/unitys-2026-roadmap-coreclr-verified-packages-fewer-surprises/)

### Inferences
- The Runtime Fee is formally resolved, but the trust damage remains. Forum posters such as ferretnt explicitly tie future forced migrations to the possibility of leaving the engine.
- Most technical complaints share one root: parallel, half-finished systems (three render pipelines, GameObjects vs ECS, UGUI vs UI Toolkit, NGO vs Netcode for Entities). Unity's 2025–26 strategy is essentially to consolidate them.

### Gaps
- I did not fetch Reddit r/Unity3D or r/gamedev threads directly, so there are no quantified Reddit sentiment data.
- DOTS/ECS maturity complaints are known but I did not source them directly, beyond the HN criticism of Burst/HPC# and Unity's own ECS-for-all plan.
- I have no independent 2026 benchmark of editor performance.

## 3. What Unity has officially acknowledged (roadmaps, staff posts, keynotes)

### Takeaway
From 2025 to 2026 Unity has openly admitted to fragmentation, slow iteration and an outdated runtime. It has:
- deprecated Built-in (6.5) and frozen HDRP;
- made URP the only investment target;
- scheduled Mono's full removal and CoreCLR/.NET 10 for Unity 7.0 (beta Dec 2026, release Q1 2027);
- paused new features in favour of "stability before novelty".

It also admits that performance optimisation is being pushed back to 2027.

### Cited Findings
**Render Pipelines strategy for 2026 (Oliver Schnabel, Unity, 23 Feb 2026)** — source for this group: [Unity Discussions](https://discussions.unity.com/t/render-pipelines-strategy-for-2026/1710004/print); [unity.com topic page](https://unity.com/topics/render-pipelines-strategy-for-2026)
- Admission: "We've heard your feedback about the complexity that comes from fragmented options."
- **Built-in:** marked deprecated in 6.5 and "available at least up through Unity 6.7 LTS... but we strictly do not recommend it for any new title". Official coverage ends no earlier than 2028 (2029 for Enterprise/Industry).
- **HDRP:** maintenance mode. "No new features are planned for HDRP"; work goes to "stability, regressions and critical issues". The only platform expansion is Nintendo Switch 2 (preview in 6.5).
- **URP:** Unity is "focusing our efforts directly on advancing URP rather than introducing one large, disruptive engine wide migration". Planned additions:
  - physical light units and exposure;
  - dynamic sky;
  - real-time GI;
  - screen-space reflections;
  - on-tile post-processing.
- **Build times:** Unity acknowledged that shader compilation is a major contributor.
- **Upscaling:** a new framework (DLSS4, FSR3, console upscalers) comes in 6.6 for both pipelines.

**CoreCLR, Scripting and Serialization Update (Eric Dziurzynski, Unity, 16 Jun 2026)** — source for this group: [Unity Discussions](https://discussions.unity.com/t/coreclr-scripting-and-serialization-update-june-2026/1723299)
- **6.6:** Fast Enter Play Mode on by default, native Dictionary serialization, Burst integrated into core.
- **6.7 LTS:** the final Mono-based release, with an experimental CoreCLR desktop player and IL2CPP for iOS.
- **Unity 7.0:** full CoreCLR with ".NET 10 and C# 14". Mono is removed, and "traditional domain reloads go away entirely in Unity 7.0".
- `com.unity.serialization` is deprecated. Unity recommends System.Text.Json, Odin or MemoryPack instead.
- Admission: "We are explicitly delaying major performance optimization work until the subsequent LTS release following Unity 7.0 in 2027."
- Experimental builds "are not supported. Do not use them for active productions."
- **Timeline conflict:** earlier messaging (Unite Nov 2025, the "Path to CoreCLR, 2026" guide) said Mono would be dropped in Unity 6.8. — [Path to CoreCLR guide](https://discussions.unity.com/t/path-to-coreclr-2026-upgrade-guide/1714279); [Digital Production](https://digitalproduction.com/2025/11/26/unitys-2026-roadmap-coreclr-verified-packages-fewer-surprises/)
  - The June 2026 update names 6.7 LTS as the last Mono release and Unity 7.0 as the switch. The likely explanation is that the planned 6.8 was rebranded Unity 7, but I did not see this stated explicitly.

**Unite 2025 (Barcelona, Nov 2025)** — source for this group: [Digital Production](https://digitalproduction.com/2025/11/26/unitys-2026-roadmap-coreclr-verified-packages-fewer-surprises/)
- The theme was "stability before novelty".
- Unity paused new animation and world-building workflows.
- ECS becomes a core engine package in 6.4, with ECS components attachable to GameObjects instead of forcing a rewrite.
- Signed/verified packages arrive in 6.3.
- The hierarchy is rebuilt to handle "millions of objects".

**Unity 7 roadmap (Matt Bromberg, Unite Seoul, 21 Jul 2026)** — source for this group: [Unity news](https://unity.com/news/unity-7-roadmap-revealed-at-unite-seoul)
- Promises:
  - "near-instant Play Mode";
  - 90% faster shader builds;
  - a CLI and public API, plus a free MCP for coding agents;
  - Surface Cache GI (real-time global illumination);
  - AI-driven Vector ads and native direct-to-consumer IAP.
- Upgrade promise: "Unity 7 will not require a traditional upgrade from Unity 6 – no rebuilding, no new language to learn, nothing broken." This is an implicit admission that past upgrades were painful.
- Early beta in Dec 2026; full release in Q1 2027.

**Runtime Fee admissions**
- Whitten, 2023: "I am sorry…" — [Dexerto](https://www.dexerto.com/gaming/unity-boss-shares-updates-to-runtime-fee-amid-controversy-i-am-sorry-2306028/)
- Bromberg, 2024, on building a "partnership built on trust". — [Unity blog](https://unity.com/blog/unity-is-canceling-the-runtime-fee)

**Bromberg layoff memo (Feb 2025)**
- Unity is "currently stretched across too many products, creating complexity and limiting impact." — [PC Gamer](https://www.pcgamer.com/gaming-industry/2-years-into-unitys-long-downward-spiral-even-more-employees-are-being-laid-off-as-ceo-says-its-still-stretched-across-too-many-products/); [80.lv](https://80.lv/articles/exclusive-unity-ceo-s-internal-announcement-to-staff-amidst-the-layoffs)

### Inferences
- Unity now publicly concedes most of the community's technical criticisms: fragmentation, iteration speed, the Mono runtime and abandoned or half-finished systems. The fixes land mostly in the Unity 7 / 2027 window, so in 2026 many of them are still promises.
- Freezing HDRP is a strategic retreat from the high-end, AAA-looking segment. This fits the GDC data showing Unreal dominant at AA/AAA studios.

### Gaps
- I did not watch the Unite/GDC 2026 talk videos, such as ["Path to CoreCLR | GDC 2026"](https://www.youtube.com/watch?v=_t6xVfrmEWU), for verbatim quotes.
- I did not check whether Unity 6.5 and 6.6 actually shipped on schedule.

## 4. Migrations, notable titles, business context (layoffs, CEOs, pricing, market share)

### Takeaway
Unreal (42%) overtook Unity (30%) as the primary engine in the GDC 2026 survey. Godot is rising among new indies (11%). Mega Crit's *Slay the Spire 2* is the flagship Unity-to-Godot story.

Financially, Unity is recovering on ads (Vector), not on the engine. In Q2 2026, Create (engine) revenue was roughly flat at $158M, while Grow (ads) jumped to $389M.

Leadership changed after the Runtime Fee: Riccitiello left in Oct 2023 and Bromberg took over in 2024. There have been several rounds of layoffs since 2023, including 1,800 people (25% of staff) in Jan 2024.

### Cited Findings
**Market share**
- GDC 2026 SOTI:
  - Unreal is the primary engine for 42% of respondents and Unity for 30%. This is the first time Unreal has come out on top.
  - Unreal is used by 59% at AA studios and 47% at AAA.
  - Godot is used by 11% of newer indies.
  - Source: [GDC](https://gdconf.com/article/gdc-2026-state-of-the-game-industry-reveals-impact-of-layoffs-generative-ai-and-more/)

**Migrations**
- Mega Crit had built about two years of *Slay the Spire 2* in Unity. After the 2023 fee, it called Unity's actions "a violation of trust" and moved to Godot. The switch was confirmed at the April 2024 reveal. — [Game Developer](https://www.gamedeveloper.com/business/slay-the-spire-devs-followed-through-on-abandoning-unity); [PC Gamer](https://www.pcgamer.com/games/card-games/slay-the-spire-2-ditched-unity-for-open-source-engine-godot-after-2-years-of-development/)
- *Slay the Spire 2* launched on 5 Mar 2026 and reportedly sold 3M copies in its first week. PC Gamer called it "one of the year's biggest hits". — [Wikipedia](https://en.wikipedia.org/wiki/Slay_the_Spire_II); [PC Gamer](https://www.pcgamer.com/games/roguelike/slay-the-spire-2-is-one-of-the-years-biggest-hits-which-is-a-good-time-to-remember-it-abandoned-unity-because-of-the-dev-fee-debacle-that-is-how-badly-you-f-d-up/)
- Kinetic Games (Phasmophobia) and Ten Chambers are named as partners in Unity's Production Verification programme, i.e. studios that are still on Unity. — [Digital Production](https://digitalproduction.com/2025/11/26/unitys-2026-roadmap-coreclr-verified-packages-fewer-surprises/)

**CEO changes**
- John Riccitiello "retired" effective immediately on 9 Oct 2023, after the fee backlash. James Whitehurst became interim CEO. — [CNBC](https://www.cnbc.com/2023/10/09/unity-ceo-john-riccitiello-is-retiring-from-gaming-software-company-.html); [Unity IR PDF](https://s24.q4cdn.com/622300748/files/doc_news/Unity-Announces-Leadership-Transition-2023.pdf)
- Matt Bromberg joined as CEO in 2024. — [SEC DEF 14A FY2026](https://www.sec.gov/Archives/edgar/data/1810806/000181080626000021/unity-20260327.htm)

**Layoffs**
- Jan 2024: 1,800 jobs cut, about 25% of the workforce and Unity's largest round ever. — [CNBC](https://www.cnbc.com/2024/01/08/unity-software-to-lay-off-1800-employees-as-part-of-a-corporate-restructuring.html); [Game Informer](https://gameinformer.com/news/2024/01/10/unity-to-lay-off-1800-employees)
- Game Developer reports $205M spent in FY2024 on cutting about 25% of staff. — [Game Developer](https://www.gamedeveloper.com/business/report-unity-continues-layoffs-with-abrupt-communications-and-5am-emails)
- Feb 2025: further cuts hit Engine Product, CTO and Ads, including the Behavior team. — [Game Developer](https://www.gamedeveloper.com/business/report-unity-continues-layoffs-with-abrupt-communications-and-5am-emails); [PC Gamer](https://www.pcgamer.com/gaming-industry/2-years-into-unitys-long-downward-spiral-even-more-employees-are-being-laid-off-as-ceo-says-its-still-stretched-across-too-many-products/)
  - A law-firm blog calls this the 6th round of cuts. — [Samfiru Tumarkin](https://stlawyers.ca/blog-news/unity-technologies-completely-abrupt-job-cuts-2025/)
  - Unverified: search snippets claimed further layoffs in 2026, but I could not confirm this from a primary source. — [Mobilegamer.biz](https://mobilegamer.biz/more-layoffs-incoming-at-unity-amid-further-restructuring/)

**Financials (Q2 2026, reported 6 Aug 2026)** — source for this group: [Unity IR](https://investors.unity.com/news/news-details/2026/Unity-Reports-Second-Quarter-2026-Financial-Results/default.aspx); [Nasdaq](https://www.nasdaq.com/press-release/unity-reports-second-quarter-2026-financial-results-2026-08-06)
- Total revenue: $546M (vs $441M in Q2 2025), up 24%.
- Grow Solutions (ads/Vector): $389M (vs $287M).
- Create Solutions (engine): $158M (vs $154M).
- GAAP net loss: $23M. Adjusted EBITDA: $160M. Free cash flow: $202M.
- Bromberg: "This was arguably the best quarter in Unity's history as a public company."

**Pricing (current)**
- Personal is free up to $200K revenue. Pro is $2,200/seat/year. Enterprise rose 25% on 1 Jan 2025. — [Unity blog](https://unity.com/blog/unity-is-canceling-the-runtime-fee)

### Inferences
- Unity's business is increasingly an ad-tech company with an engine attached: engine revenue is flat, and growth comes from Vector. Developers who worry the engine is under-invested can point to this, and to the fact that layoffs repeatedly hit Engine Product teams.
- The drop to 30% primary-engine share is probably a combined effect of the fee fallout, layoffs and perceived tooling stagnation. GDC coverage makes this attribution, but it is commentary, not survey data.

### Gaps
- I have no hard data on how many studios actually migrated away from Unity, beyond anecdotes and the Slay the Spire 2 case.
- I did not collect a list of notable 2025–26 Unity-shipped titles beyond the partner studios above. Candidates to verify include Phasmophobia, Hollow Knight: Silksong (2025) and Genshin Impact.
- I did not find exact headcount figures for the 2025 and 2026 layoff rounds.
