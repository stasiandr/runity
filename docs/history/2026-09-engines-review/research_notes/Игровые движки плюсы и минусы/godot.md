# Godot Engine (4.x): strengths, weaknesses, maintainers' admissions (as of Sept 2026)

Current version: Godot 4.7 (feature release late June 2026; 4.7.1 maintenance patch after it). Timeline: 4.5 (Sept 15, 2025), 4.6 (Jan 26, 2026), 4.7 (June 2026).

## What developers praise

### Takeaway
People praise Godot for being free and MIT-licensed, for its lightweight editor and fast iteration, for its node/scene composition model and for its strong 2D. It gained momentum after Unity's 2023 per-install fee fiasco. Adoption has grown very fast (7.1% of Steam releases in 2025 vs 0.9% in 2020; ~39% of GMTK Jam 2025 entries), and it now has a flagship commercial hit in Slay the Spire 2.

### Cited Findings
- **Open source, no lock-in.** Godot stays MIT-licensed with no NDAs or restricted tools. The Foundation cites this commitment to openness as the reason it won't ship official console ports — [Godot Console Support](https://godotengine.org/consoles/)
- **Being able to change the engine.** Mega Crit (Slay the Spire 2) said that because Godot is open source there are no "dead ends": the team can modify the engine when needed. They run their own fork, "MegaDot" — [STS2 Reddit AMA summary, Feb 2026](https://slaythespire2.gg/guides/mega-crit-ama-reddit-feb-2026)
- **Lightweight cross-platform builds.** Mega Crit said lightweight Mac/PC/Linux builds speed up iteration. Ed Lu named the VRAM resource tracker as a favorite feature. Casey Yano's one regret was TextMeshPro, though he added that "MSDF in Godot is great tho" — [STS2 AMA summary](https://slaythespire2.gg/guides/mega-crit-ama-reddit-feb-2026)
- **Node/scene architecture as a deliberate choice.** Linietsky (2021): "Godot does composition at a higher level than in a traditional ECS." It puts ease of use and scene reusability ahead of raw data-oriented performance — [Why isn't Godot an ECS-based game engine? (Feb 26, 2021)](https://godotengine.org/article/why-isnt-godot-ecs-based-game-engine/)
- **Growth after the Unity fee.** Mega Crit was ~2 years into development in Unity. It switched to Godot after the 2023 runtime-fee announcement, calling Unity's actions "a violation of trust", and stayed on Godot even after Unity backtracked — [Game Developer](https://www.gamedeveloper.com/business/slay-the-spire-devs-followed-through-on-abandoning-unity); [PC Gamer](https://www.pcgamer.com/games/card-games/slay-the-spire-2-ditched-unity-for-open-source-engine-godot-after-2-years-of-development/)
- **Slay the Spire 2 as a flagship.** It entered Early Access on March 5, 2026 and peaked at ~574.6K concurrent players on Steam, the biggest Steam launch of 2026 per the outlet. It holds ~97% positive reviews — [Outlook Respawn](https://respawn.outlookindia.com/gaming/gaming-news/slay-the-spire-2-hits-574k-players-crushing-aaa-games-on-steam); [KitGuru](https://www.kitguru.net/gaming/matthew-wilson/slay-the-spire-2-surpasses-marathon-with-over-500k-concurrent-players-on-steam/)
- **Other notable shipped titles (all 2D or stylized/small 3D).** Brotato, Dome Keeper, Cassette Beasts and Buckshot Roulette are all in the official showcase — [Godot Showcase](https://godotengine.org/showcase/). Revenue estimates: Brotato ~$10.7M, Buckshot Roulette ~$6.9M, Dome Keeper ~$6.1M, Cassette Beasts ~$4.1M. These are unofficial third-party estimates (Medium author, probably Gamalytic-style data), so treat them with caution — [Medium analysis 2025](https://alihan98ersoy.medium.com/most-successful-games-made-with-godot-engine-revenue-sales-analysis-2025-9b69af569585). GameDiscoverCo also says Brotato is reportedly the highest-revenue Godot title on Steam (before STS2) — [GameDiscoverCo, Mar 17, 2026](https://newsletter.gamediscover.co/p/hows-pc-game-engine-usage-changing)
- **Steam share (GameDiscoverCo, ~33K games analyzed).**
  - Godot's share of Steam releases went from 0.9% (2020) to 7.1% (2025), and it is 8.6% among unreleased/upcoming games.
  - Over the same period Unity went from ~50–51% to 49.4%, and Unreal from ~15% to ~20%.
  - Godot's growth "hasn't come at the expense of Unity or Unreal".
  - Source: [GameDiscoverCo](https://newsletter.gamediscover.co/p/hows-pc-game-engine-usage-changing)
- **SteamDB counts.** 618 Godot games in 2023–24, ~1,500 in 2024–25 and 2,864 in 2025–26 (~4.6x in two years) — [SteamDB Godot](https://steamdb.info/tech/Engine/Godot/), as summarized by [StraySpark](https://www.strayspark.studio/blog/godot-explosive-growth-2026). This is secondary aggregation, and the exact figures may shift.
- **Game jams.** Godot made up 39% of GMTK Jam 2025 entries (9,724 total), up from 13% in 2021 — [GameDiscoverCo](https://newsletter.gamediscover.co/p/hows-pc-game-engine-usage-changing)
- **Official growth article (Clay John, May 6, 2026).**
  - Each release gets roughly 2 million downloads from the website/GitHub.
  - Steam releases show "strong signs of exponential growth".
  - The community "has doubled in size in the last couple of years", but growth "appears to be slowing down in the last year".
  - Source: [Godot usage and engine growth](https://godotengine.org/article/godot-growth-stats-2026/)
- **Release cadence and contributor base.**
  - 4.5: ~2,500 commits from 400+ contributors — [80.lv](https://80.lv/articles/godot-engine-4-5-released)
  - 4.6: 2,001 commits from ~400 contributors — [Godot 4.6 release page](https://godotengine.org/releases/4.6/)
  - 4.7: 300+ contributors and 1,600+ merged PRs; the release page counts 700+ new Godot games on Steam in 2026 YTD — [Godot 4.7 release page](https://godotengine.org/releases/4.7/)
- **Main features in recent releases.**
  - 4.5: stencil buffer, a screen-reader accessibility layer (experimental), shader baker, script backtracing — [80.lv](https://80.lv/articles/godot-engine-4-5-released)
  - 4.6: new "Modern" editor theme, movable/floatable docks, Jolt as default 3D physics, SSR overhaul, LibGodot (embedding), new IK framework, Direct3D 12 as the Windows default — [Godot 4.6](https://godotengine.org/releases/4.6/)
  - 4.7: HDR output, AreaLight3D, a new Asset Store replacing the Asset Library, standalone Android export (GABE), day-one Android XR and Steam Frame support — [Godot 4.7](https://godotengine.org/releases/4.7/)

### Inferences
- Godot's commercial successes are overwhelmingly 2D or stylized, low-scope 3D. Even the biggest (STS2) is a 2D card game, which fits the reputation of "excellent for 2D/indie, unproven for AAA 3D".
- The drop from a 7.1% share of all games to 4.1% among >$500K-revenue games suggests Godot is used disproportionately by hobbyists and small projects, although STS2 will lift the top-end numbers.

### Gaps
- I couldn't access Casey Yano's full 2023 "On Evaluating Godot" post (HTTP 403): [caseyyano.com](https://caseyyano.com/on-evaluating-godot-b35ea86e8cf4). I have no verified list of his pros and cons.
- I found no official sales data for Brotato, Dome Keeper and the others. All revenue figures are third-party estimates.

## Top recurring complaints

### Takeaway
The persistent complaints are:
- 3D performance, pipeline and tooling are weaker than Unity/Unreal (no world streaming, post-processing and FBX/animation workflows, few big shipped 3D titles).
- There is no official console support; you have to pay third parties such as W4 Games.
- C# still can't export to web (or, per a community source, iOS), and the .NET build is separate.
- GDScript is slow for data-heavy tasks and has no namespaces.
- Each feature release brings regressions.
- The PR review backlog is a "meme".
- The 2024 "#Wokot" social-media controversy.

### Cited Findings
- **Maintainers' own list of weaknesses on the official Priorities page.**
  - Rendering: post-processing is "a weak spot in Godot's renderer", with worse performance and quality than they want.
  - Physics: the current system doesn't expose Jolt's features well; there are "numerous features that can't be implemented in Godot due to the current way the system works".
  - Animation: 2D skeletal animation has lagged behind 3D.
  - Editor: GDScript has none of the refactoring tools modern IDEs offer, and the editor treats GDExtension classes "like binary blobs".
  - Web: the "size of the engine is becoming a pain point", and the whole .pck has to be downloaded upfront.
  - GDScript: "lack of namespaces" is a main complaint, performance is "lackluster" for data-heavy tasks, and `class_name` has robustness "caveats".
  - .NET: no C# web export, a separate build download, and missing .NET-specific docs.
  - Platforms: window creation is slower in 4.x than in 3.x, and multi-monitor DPI needs work.
  - Source: [Godot Priorities](https://godotengine.org/priorities/)
- **Debugging and ray tracing.** Rendering lead Clay John said Godot's debugging/profiling tools are "relatively basic" and that hardware ray tracing is "a ways away" — [Godot Rendering Priorities, Sept 2024 (forum)](https://forum.godotengine.org/t/godot-rendering-priorities-september-2024/83939) (historical; still reflected on the Priorities page)
- **3D performance anecdotes (user level, low rigor).** Some devs report Unity running similar 3D projects 2–3x faster. Large-scene complaints include long loading times, no world streaming, and physics getting stuck — [itch.io 2025 engine showdown](https://itch.io/blog/1067028/game-engine-showdown-2025-unity-vs-godot-vs-unreal-which-should-you-choose); [Road to Vostok Steam discussion](https://steamcommunity.com/app/1963610/discussions/0/4629233756626052446)
- **Postmortem: Cosmic Trinity Games switched Godot → Unity (June 14, 2025).**
  - "FBX import felt inconsistent. Rigging and animation workflows took too many workarounds."
  - "We constantly fought with shadow bugs, broken colliders, and unpredictable lighting behaviour."
  - C#: "Debugging larger projects caused weird errors. Mobile export with .NET was unreliable."
  - "Adding a second dev slowed us down instead of speeding things up."
  - Source: [dev.to](https://dev.to/cosmictrinitygames/why-we-switched-from-godot-to-unity-5-hard-lessons-from-an-indie-studio-1ke4)
- **Consoles.** Consoles are "closed platforms that require special permissions, SDKs, and hardware", so the Foundation won't maintain official ports.
  - It lists 8 third-party providers: W4 Games, RAWRLAB, Lone Wolf Technology, Pineapple Works, mazette!, Tuanisapps, Seaven Studio, Sickhead. Pricing isn't public — [Godot Console Support](https://godotengine.org/consoles/)
  - W4 Games (founded by Linietsky and Verschelde) sells official middleware ports for Switch, Xbox Series and PS5, plus a native Switch 2 port in beta — [W4 Switch 2 beta](https://www.w4games.com/blog/w4-games-news-1/w4-consoles-beta-support-for-the-nintendo-switch-2-87); [Game Developer](https://www.gamedeveloper.com/console/w4-games-says-godot-console-porting-solutions-land-in-october)
  - STS2 had no console date at EA launch — [STS2 AMA](https://slaythespire2.gg/guides/mega-crit-ama-reddit-feb-2026)
- **C# web export.** Still unsupported in official 4.x (tracking issue [#70796](https://github.com/godotengine/godot/issues/70796)). A Web .NET prototype was shown at GodotCon Boston (May 2025), but there is no official release — [Godot Forum thread](https://forum.godotengine.org/t/is-there-an-update-on-exporting-c-projects-to-web/128821). There is a community fork — [ComplexRobot/godot-dotnet-web-export](https://github.com/ComplexRobot/godot-dotnet-web-export). The Priorities page confirms it's still missing — [Priorities](https://godotengine.org/priorities/)
- **Physics.** Jolt was an experimental option from 4.4 and became the default for new 3D projects in 4.6 (Jan 2026). Existing projects aren't migrated, and the notes warn that results will differ — [Godot 4.6](https://godotengine.org/releases/4.6/); [80.lv](https://80.lv/articles/godot-4-6-is-out). This is partly resolved: the Priorities page still says the physics API limits what Jolt can expose.
- **Regressions.**
  - 4.6.2 needed a second RC because the first one "required more critical bugfixes than usual" — [linuxcompatible](https://www.linuxcompatible.org/story/godot-462-rc-2-released/)
  - 4.7 RC3 fixed Jolt physics glitches, animation bugs and particle-buffer leaks — [linuxcompatible](https://www.linuxcompatible.org/story/godot-47-release-candidate-3-fixes-critical-regressions-before-final-launch/)
  - 4.7.1 shipped 78 fixes (rendering artifacts, editor crashes, Android touch) — [linuxcompatible](https://www.linuxcompatible.org/story/godot-471-released-quick-stability-patch-fixes-rendering-and-platform-bugs/)
  - These are secondary news sources.
- **Asset ecosystem.** The 4.7 "Asset Store" replaced the old Asset Library — [Godot 4.7](https://godotengine.org/releases/4.7/). Developers who left still cite Unity's "massive Asset Store" and built-in terrain/lightbaking tools as advantages — [itch.io search results / devlogs](https://itch.io/blog/926428/unity-to-godot-and-back)
- **Governance controversy: "#Wokot" (Sept 27, 2024).**
  - The official X account posted "Apparently game engines are woke now?… Show us your #Wokot games", then blocked many critics, including developers who had asked technical questions.
  - A fork (Redot) appeared and some donors reportedly pulled funding.
  - The Foundation board apologized: "we mistakenly blocked individuals who were not participating in the harassment… takes full responsibility."
  - Sources: [Know Your Meme](https://knowyourmeme.com/memes/events/godot-engine-user-blocking-controversy-wokot); [AUTOMATON (JP)](https://automaton-media.com/articles/newsjp/20241001-312695/); [GameFromScratch](https://gamefromscratch.com/the-godot-community-firestorm/) (page not loadable for me)
  - Historical. This hasn't dominated since, but it's still cited by critics.
- **Funding level.**
  - The Development Fund shows ~€35,699/month in recurring donations from 1,822 members (at fetch, Sept 2026). No staff count is published — [fund.godotengine.org](https://fund.godotengine.org/)
  - The Foundation admits "donations don't track with the increase in users" and that the project runs on "hundreds of contributors in their free time, as well as a handful of part or full-time developers" — [Godot growth stats 2026](https://godotengine.org/article/godot-growth-stats-2026/)

### Inferences
- ~€36K/month (~€430K/yr) is tiny compared with Unity/Epic engineering budgets. It likely explains the slow progress on big-ticket items (C# web, streaming, ray tracing, profiling tools) and the review bottleneck.
- The console problem is structural (license vs NDA), not technical. Shipping on console will keep costing extra money and depending on a third party.

### Gaps
- I found no rigorous, reproducible 3D benchmark comparing Godot 4.6/4.7 with Unity 6 or UE5. The performance claims are anecdotal.
- I didn't verify C# iOS export status in 4.6/4.7. The claim that iOS is also unsupported comes only from a search summary and may be outdated, since iOS .NET support reportedly arrived in 4.2+ as experimental. The report writer should not state it firmly.
- I have no verified list of large shipped 3D Godot games. Road to Vostok is a notable in-progress example.

## Maintainers' own admissions

### Takeaway
The maintainers are unusually candid:
- Linietsky concedes ECS-style designs perform better for games with tens of thousands of objects.
- The Priorities page lists concrete weaknesses: post-processing, physics API, GDScript performance/namespaces, C# web, web build size.
- The June 2026 contribution-policy post admits the reviewer shortage "was one that we successfully ignored" and that the PR backlog is "a meme".
- Rémi Verschelde publicly asked for more funding to pay maintainers.

### Cited Findings
- **Linietsky on ECS.** "some types of games will see a performance benefit when using ECS in the game logic side", namely those processing "dozens of thousands of objects". His argument is that "most games are generally just in the hundreds of objects at most". For heavy cases he recommends culling, direct Server APIs, compute/GPGPU, or ECS plugins such as Godex — [Why isn't Godot an ECS-based game engine?](https://godotengine.org/article/why-isnt-godot-ecs-based-game-engine/)
- **Contribution policy (June 30, 2026).**
  - "the number of qualified reviewers is small, reviewing PRs is demanding, and we can't keep up with everything coming in". The open PR backlog has become "a meme in the community".
  - New rules: autonomous AI agents / "vibe coding" lead to an auto-ban; substantial AI-generated code is banned; minor AI use must be disclosed; contributors with ≤3 merged PRs need maintainer approval before submitting features or big refactors.
  - Source: [Changes to our Contribution Policies](https://godotengine.org/article/contribution-policy-2026/)
  - Press quote from the post: "This reviewer shortage was already a problem, but it was one that we successfully ignored" — [Cybernews](https://cybernews.com/ai-news/open-source-godot-tightens-ai-backlog/); [Hackaday](https://hackaday.com/2026/07/03/godots-new-contributing-policy-adds-barriers-for-ai-slop/)
- **Rémi Verschelde on Bluesky (Feb 16, 2026).** "We find ourselves having to second guess every PR from new contributors, multiple times per day"; "I don't know how long we can keep it up"; "more funding so we can pay more maintainers to deal with the slop is the only viable solution" — [spilled.gg](https://spilled.gg/godot-engine-maintainers-code-submissions-ability-development/)
- **Priorities page admissions** (post-processing "weak spot", GDScript performance "lackluster", "lack of namespaces", engine size "a pain point", physics architecture limiting Jolt) — [Godot Priorities](https://godotengine.org/priorities/)
- **Rendering lead Clay John (2024).** Profiling/debug tools are "relatively basic", and the team wants to overhaul or replace most post-processing effects — [Rendering Priorities Sept 2024](https://forum.godotengine.org/t/godot-rendering-priorities-september-2024/83939)
- **Clay John on growth (May 2026).** Community growth is slowing, and donations are not keeping pace with users — [Godot growth stats](https://godotengine.org/article/godot-growth-stats-2026/)
- **The #Wokot apology (2024).** The Foundation board took "full responsibility" for the mistaken blocks — [Know Your Meme (quotes statement)](https://knowyourmeme.com/memes/events/godot-engine-user-blocking-controversy-wokot)
- **Console stance.** Official ports would contradict Godot's openness values, so the Foundation defers to third parties — [Godot Consoles](https://godotengine.org/consoles/)

### Inferences
- These admissions line up closely with community complaints (C#, GDScript performance, 3D post-processing, review backlog, funding). This transparency is itself often read as a governance strength.
- The 2026 AI-contribution restrictions may reduce low-quality PRs, but they also raise the barrier for new contributors, which could tighten the reviewer pipeline further in the short term.

### Gaps
- I didn't find a Sam Gondelman–specific post. I relied on Clay John, who is the rendering lead and a Foundation board member.
- I have no exact current count of open PRs or issues on godotengine/godot. The GitHub pages weren't checked numerically.
- I didn't collect Juan Linietsky's individual X/Twitter posts on performance priorities.
