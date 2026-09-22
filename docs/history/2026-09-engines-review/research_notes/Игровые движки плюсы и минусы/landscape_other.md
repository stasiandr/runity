# Game engine landscape: market share, cross-engine surveys, other engines (as of Sept 2026)

Scope: data that cuts across all engines, plus short notes on the less common engines. Unity, Unreal, Godot, Bevy and in-house engines are covered in depth by other notes.

Source-quality warning for the writer: many "game engine statistics 2026" pages (levvvel, voxbooster, rec0ded88, shattered.io, tech-insider, strayspark, ziva) are SEO aggregators. They often restate VG Insights and GDC numbers inaccurately. For example, they call VGI's **units-sold** share "revenue" share. Wherever possible, the figures below come from primary sources (the VGI PDF, gdconf.com, gameworldobserver, official engine sites).

## 1. Market share data (Steam, GDC survey, game jams, dev surveys)

### Takeaway
Two different measures give two different leaders:
- **By number of Steam releases**, Unity leads: about 51% in 2024.
- **By units sold**, Unreal (31%) passed Unity (26%) in 2024. Custom AAA engines still took the largest single share (41%).

Among professionals, the GDC 2026 survey put Unreal first for the first time (42% vs Unity's 30%). Among hobbyists and jam entrants, Godot overtook Unity in GMTK Jam 2026 (47% vs 34%).

### Cited Findings
**VG Insights, "The Big Game Engine Report of 2025"**
- Covers Steam data for 2024. Methodology: VGI tags the engines of more than 13,000 Steam games. Only games with at least 1,000 lifetime units are included, and units are estimated (partly with the Boxleiter method). — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **Share of 2024 Steam releases:** Unity 51%, Unreal 28%, Godot 5%, GameMaker 4%, Ren'Py 2%, Other (mostly custom) 10%. — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **Share of all Steam units sold in 2024:** Unreal 31%, Unity 26%, Other/custom (Frostbite, RAGE, Creation Engine, etc.) 41%, Godot about 1%, GameMaker about 1%, Ren'Py about 0%. — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- Godot, RPG Maker, GameMaker and similar engines make up "just over 10% of all games released, but barely 2% of the actual units sold" (2024). — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **Engine mix by game size (units, 2024), Unity / Unreal / Godot+GameMaker / Other:**

  | Game size | Unity | Unreal | Godot+GM | Other |
  |---|---|---|---|---|
  | Tiny (<1k units) | 50% | 23% | 13% | 14% |
  | Small (1k–100k) | 48% | 27% | 6% | 19% |
  | Medium (100k–1M) | 35% | 32% | 5% | 29% |
  | Large (1M+) | 22% | 31% | 1% | 46% |

  — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **Long-run trend in releases:**
  - Custom engines fell from 71% of Steam releases (2012) to 13% (2024).
  - Unity rose from 9% (2012) to a peak of about 53% (2021), then 50% in 2023–2024.
  - Unreal rose from about 13–17% to 28% (2024).
  - Godot+GameMaker combined reached about 9% in 2024.

  — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **Units-sold trend:**
  - Custom engines went from 74% (2014) to 42% (2024), below half for the first time.
  - Unreal jumped from 19% (2023) to 31% (2024), driven by Black Myth: Wukong, Palworld, Tekken 8, STALKER 2 and others.
  - Unity was flat at about 26–27% from 2021 to 2024.

  — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **VGI forecast for 2030 (units):** Unreal about 40%, Unity about 28%, custom about 27%, Godot+GameMaker about 5%. This is an estimate, not a measurement. — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- UE5 made up 72% of Unreal games released in 2024. A new UE version takes 3–4 years to show up in shipped games. — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **Smaller public engines** (Godot, GameMaker, RPG Maker, Ren'Py): units grew about 144% from 2020 to 2024, and "over 2/3 of the growth since 2020 has been driven by Godot." — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **By genre (2024 units):** games that need high-end graphics (action RPG, soulslike, FPS, open-world survival) skew toward Unreal. Strategy, simulation, city builders, 4X and turn-based games skew toward Unity. — [VGI PDF](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **Mislabelled figure to avoid:** aggregators report "Unreal 31% of 2024 Steam revenue" — [voxbooster](https://voxbooster.com/blog/game-engine-statistics-2026/). The primary VGI chart is labelled "units sold", not revenue. Use "units sold".

**GDC State of the Game Industry 2026** (published Jan 2026, 2,300+ respondents)
- Unreal is the primary engine for 42% of respondents and Unity for 30%. This is the first time Unreal has led. — [GDC](https://gdconf.com/article/gdc-2026-state-of-the-game-industry-reveals-impact-of-layoffs-generative-ai-and-more/)
- Unreal is used by 59% of respondents at AA studios and 47% at AAA studios. 54% of developers at older indie studios still use Unity. Godot is used by 11% of developers at newer indie studios. — [GDC](https://gdconf.com/article/gdc-2026-state-of-the-game-industry-reveals-impact-of-layoffs-generative-ai-and-more/); report PDF mirror: [InvestGame](https://investgame.net/news/pdf/2026-01-29-dec052f4_d88e_48ce_9f83_a18ce2f2a6e5_541400_gdc26_pdf_soti_report/)
- A secondary summary attributes Unity's decline to the 2023 fee fallout, layoffs and a perception that Unity's tooling has stagnated. This is commentary, not survey data. — [Cinevva](https://app.cinevva.com/news/2026-03-13-gdc-2026-by-the-numbers.html)

**GMTK Game Jam** (large hobbyist/indie jam; the best leading indicator for newcomers)
- 2026: about 10,400 entries. Godot 47% (about 4,900 games), Unity 34% (about 3,600), GameMaker 5% (498), Unreal 3% (319). This is the first year Godot led, ending nine straight years of Unity leadership. — [Game World Observer, 29 Jul 2026](https://gameworldobserver.com/2026/07/29/unity-is-no-longer-the-leader-godot-has-become-the-most-popular-engine-at-gmtk-game-jam-2026)
- Godot's share by year: 2022 16%, 2023 19%, 2024 37%, 2025 39%, 2026 47%. — [Game World Observer](https://gameworldobserver.com/2026/07/29/unity-is-no-longer-the-leader-godot-has-become-the-most-popular-engine-at-gmtk-game-jam-2026)
- 2025 had 9,724 entries with Godot at about 39%. Global Game Jam 2025: Godot at about 20% of 4,108 participants surveyed. Both figures come from a secondary source. — [StraySpark](https://www.strayspark.studio/blog/godot-explosive-growth-2026)

**Godot on Steam (secondary figure, unverified)**
- Godot titles on Steam reportedly grew 618 (2023–24) → about 1,500 (2024–25) → 2,864 (2025–26), attributed to SteamDB. The claim that Godot is "8–10% of new Steam releases" is also secondary. I could not check either against SteamDB directly (steamdb.info/tech returned 403). — [search summary citing StraySpark/VoxBooster](https://voxbooster.com/blog/game-engine-statistics-2026/)

**General developer surveys**
- Stack Overflow 2024: Unity used by 5.9% of all developers and Unreal by 3.0%. These come via an aggregator; the primary is [survey.stackoverflow.co](https://survey.stackoverflow.co/2025/). — [levvvel](https://levvvel.com/statistics/game-engine-market-share/)

### Inferences
- The right framing for the report is a three-tier market:
  - Unity leads by release volume (indies, small and mid-size games).
  - Unreal leads among third-party engines by units sold and among professionals (AA/AAA).
  - Godot is taking the entry and hobbyist pipeline (jams) and is slowly converting that into Steam releases. Its commercial share is still tiny (about 1% of units in 2024).
- The biggest structural trend is not Unity vs Unreal. It is custom engines losing ground to Unreal, which VGI calls the "End of the Era of In-House Engines."
- Jam share leads Steam share by several years. Today's jam cohort is tomorrow's indie releases, so Godot's commercial share should keep rising, but slowly.

### Gaps
- I could not fetch SteamDB tech stats directly (403). There are no primary per-engine Steam counts for 2025–2026.
- I found no VGI or Sensor Tower report covering 2025 full-year engine shares, so the 2024 data is the latest primary.
- I found no GDC 2024 or 2025 engine-share figures for a year-over-year comparison. The 2026 article does not give them.
- I did not find Stack Overflow 2025 or JetBrains 2025 game-engine figures. itch.io publishes no official engine statistics; its "made with" tags exist but are unaggregated.

## 2. Migration after the Unity Runtime Fee (Sept 2023) and whether it stuck

### Takeaway
The fee was announced in September 2023 and fully cancelled on 12 Sept 2024, but the trust damage outlasted the fee itself. Godot was the main beneficiary among hobbyists and indies. In the professional segment, Unreal gained share. There is little hard data on how many shipped commercial projects actually switched engines.

### Cited Findings
- Unity cancelled the Runtime Fee on 12 Sept 2024, announced by new CEO Matt Bromberg, and returned to seat-based pricing.
  - The free-tier revenue threshold rose from $100K to $200K.
  - Pro went up about 8% and Enterprise about 25%.

  — [Unity blog](https://unity.com/blog/unity-is-canceling-the-runtime-fee); [AlternativeTo](https://alternativeto.net/news/2024/9/unity-entirely-scraps-controversial-runtime-fee-returns-to-seat-based-subscription-model)
- Godot "doubled its user base in a month" after the fee announcement, growth that previously took about a year.
  - Rémi Verschelde warned switchers not to "come to it in panic."
  - Re-Logic (Terraria) donated $100K.

  — [Game World Observer, Mar 2024](https://gameworldobserver.com/2024/03/27/godot-doubled-user-base-after-unity-controversy)
- In the Godot Community Poll 2025 (9,661 respondents), 5,521 (57.1%) had used Unity before Godot. Reported via a secondary source. — [Ziva](https://ziva.sh/blogs/godot-community-poll-2025)
- **Flagship switch:** Mega Crit dropped Unity in Sept 2023 and ported about two years of Slay the Spire II work to Godot.
  - Early access launched 5 Mar 2026.
  - It sold over 3M copies in the first week, with a 177K concurrent-player peak on day one and over 400K by day two.
  - This is the strongest commercial proof so far that a Godot title can be a large hit.

  — [Wikipedia: Slay the Spire II](https://en.wikipedia.org/wiki/Slay_the_Spire_II)
- GameMaker used the moment for its own pricing overhaul (Nov 2023). — [Game World Observer](https://gameworldobserver.com/2023/11/22/gamemaker-new-pricing-free-non-commercial-use-one-time-pro-license)
- **Evidence the shift stuck:**
  - GMTK Godot share kept rising: 19% (2023) → 37% (2024) → 39% (2025) → 47% (2026). — [Game World Observer](https://gameworldobserver.com/2026/07/29/unity-is-no-longer-the-leader-godot-has-become-the-most-popular-engine-at-gmtk-game-jam-2026)
  - Unreal took the top spot in the GDC 2026 survey. — [GDC](https://gdconf.com/article/gdc-2026-state-of-the-game-industry-reveals-impact-of-layoffs-generative-ai-and-more/)
  - On Steam, however, Unity's release share only slipped: about 53% (2021) to 50% (2024). — [VGI](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)

### Inferences
- The migration happened mainly at the start of new projects, not in running ones. Existing Unity projects mostly stayed, because the cost of porting is high; Mega Crit is the exception.
- The effect shows up first in jams and surveys, and only with a lag on Steam.
- The lasting lesson for developers is licensing risk: a vendor can change terms after the fact. That explains the popularity of open-source engines (Godot, Defold) and of one-time or royalty-free licences.

### Gaps
- There is no rigorous dataset of shipped commercial games that migrated away from Unity.
- There is no primary data on migration to GameMaker, Unreal or Defold specifically.
- I found no post-2024 Steam share data showing Unity's release share for 2025–2026.

## 3. Other engines: short notes (strengths, weaknesses, sentiment)

### Takeaway
Outside the big three, every engine fills a niche:
- **GameMaker:** 2D, commercial, one-time licence.
- **Defold:** lightweight 2D, mobile and web, free and source-available.
- **Cocos:** Chinese mobile and mini-games.
- **Flax:** a small commercial 3D alternative with royalties.
- **O3DE:** an open AAA-grade engine with minimal game adoption.
- **Frameworks** (MonoGame/FNA, Love2D, Raylib): for programmers who want control.

Together these engines account for about 10% of Steam releases but about 2% of units (VGI 2024).

### Cited Findings
**GameMaker**
- On 21 Nov 2023 it became free for non-commercial use on all non-console platforms.
- Subscriptions were replaced by a one-time $99.99 commercial licence (consoles require the higher Enterprise tier).
- Opera, which owns GameMaker, reported a threefold growth in active users since the acquisition.

— [GameMaker FAQ](https://gamemaker.io/en/help/articles/november-2023-pricing-terms-change-faq); [Game World Observer](https://gameworldobserver.com/2023/11/22/gamemaker-new-pricing-free-non-commercial-use-one-time-pro-license); [GamingOnLinux](https://www.gamingonlinux.com/2023/11/gamemaker-now-free-for-non-commercial-use-one-time-fee-for-indie-devs/)

- Share: 4% of 2024 Steam releases ([VGI](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)) and 5% of GMTK 2026 entries ([GWO](https://gameworldobserver.com/2026/07/29/unity-is-no-longer-the-leader-godot-has-become-the-most-popular-engine-at-gmtk-game-jam-2026)).

**Defold**
- Created at Avalanche Studios and acquired by King in 2014. Public launch at GDC in March 2016.
- In May 2020 King open-sourced it and transferred it to the Defold Foundation; Refold AB does most of the development.
- It has no up-front costs, licence fees or royalties, and the source is on GitHub.
- Strengths: cross-platform 2D and 3D, small builds, strong on web and mobile.

— [defold.com/about](https://defold.com/about/)

- A claim that Cocos2d-x and Defold "collectively power 19% of mobile games" comes from a low-quality aggregator. Do not use it as fact. — [asomobile](https://asomobile.net/en/blog/top-mobile-game-engines-for-ios-and-android-in-2025-trends-tools-and-market-outlook/)

**Cocos (Cocos2d-x / Cocos Creator)**
- Dominant in Chinese mobile and mini-game (WeChat and similar) development. The SCMP called it "the fast-food equivalent of Unreal Engine" (2020). — [SCMP](https://www.scmp.com/tech/tech-leaders-and-founders/article/3099271/chinese-game-engine-creator-cocos-fast-food)
- Raised a $50M Series B. — [Cocos](https://www.cocos.com/en/post/more-than-a-game-engine-cocos-completes-50-million-in-series-b-financing)

**Flax Engine**
- Free until revenue exceeds $250K per calendar quarter, then a 4% royalty on the excess. This is similar in spirit to Unreal's 5%. — [flaxengine.com/licensing](https://flaxengine.com/licensing/)

**O3DE (Open 3D Engine)**
- Amazon's former Lumberyard, donated to the Linux Foundation's Open 3D Foundation (2021) under the Apache 2.0 licence. — [Phoronix](https://www.phoronix.com/review/open-3d-engine); [Wikipedia](https://en.wikipedia.org/wiki/Open_3D_Engine)
- The 25.05.0 release (June 2025) emphasised robotics simulation, stability and performance of the Atom renderer, and multi-GPU support. Contributors include AWS, Huawei and others. — [o3de.org](https://o3de.org/o3de-25-05-0-release-june-18-2025/)
- The emphasis on robotics simulation in its own release notes points to a pivot away from games. I found no reliable count of commercial games shipped on O3DE (see Gaps).

**Ren'Py and RPG Maker**
- Ren'Py (visual novels) had about 2% of 2024 Steam releases and almost 0% of units. RPG Maker is part of the "small public engines" group. — [VGI](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)

**Custom-engine and "no engine" indie trend**
- Noel Berry (Celeste, Earthblade), "Making Video Games in 2025 (without an engine)", 18 May 2025.
  - Stack: C# with Native-AOT, SDL3 (which covers DirectX, Vulkan and Metal), his own Foster wrapper, FMOD, Dear ImGui for editors, plus LDtk, Tiled and Trenchbroom for levels. Develops on Linux.
  - Argument: big engines carry features he doesn't need and force workarounds. Modern open-source libraries make going engine-less easier and more fun.
  - Caveat: this suits small teams. Larger 3D projects benefit from established engines, and he notes some mid-sized studios are moving off proprietary engines to reduce platform risk.

  — [noelberry.ca](https://noelberry.ca/posts/making_games_in_2025/)
- Casey Yano (Mega Crit) left LibGDX because Java was "frequently broken by operating system updates" and lacked console support. Being tied to a framework has its own long-term maintenance cost. — [Wikipedia: StS II](https://en.wikipedia.org/wiki/Slay_the_Spire_II)

### Inferences
- The licensing lessons of 2023 moved "trust" (open source or a one-time fee) into a top-tier selection criterion. GameMaker, Defold, Godot and Flax all market their licensing terms explicitly.
- For a report in Russian: Defold (developed from Sweden) and Godot carry no licence or sanctions dependency on a US corporation. Cocos matters for China-facing mobile teams. This is analytical, not a sourced claim.

### Gaps (not verified in this session; the writer should verify or omit)
- **Stride** (formerly Xenko, MIT-licensed C#): adoption numbers not found.
- **Construct 3:** subscription, browser-based. Not researched.
- **Love2D and Raylib:** no market data.
- **MonoGame/FNA:**
  - Commonly reported: Stardew Valley moved from XNA to MonoGame (v1.5), and Celeste is built on XNA/FNA with the team's own "Monocle" framework.
  - Not re-verified here. See [monogame.net](https://monogame.net) and [fna-xna.github.io](https://fna-xna.github.io).
- **Jonathan Blow's Jai language and Sokoban engine, Handmade Network:** not researched this session.
- **Roblox and UEFN as platforms:** revenue-share and user numbers not collected. Check the Roblox 10-K/DevEx data and Epic's Fortnite creator economy posts.
- **O3DE shipped games:** no reliable list found. The "low adoption" label is an inference from the lack of evidence.

## 4. Cross-engine themes: what developers want, common pain points, engine vs no engine

### Takeaway
Across sources, the same priorities keep coming up:
- predictable, trustworthy licensing;
- a quick start and a proven feature set;
- console support;
- stability and control over the roadmap.

The engine-vs-no-engine debate comes down to control plus smaller scope, versus speed to a first build plus a talent pool.

### Cited Findings
- **VGI's pros of a public engine:** you can start on day one, the quality is proven (e.g. Nanite/Lumen), there is no in-house maintenance, and the talent pool and training material are large. — [VGI](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **VGI's cons:** market dominance lets vendors raise prices; heavy customisation is often still needed; switching is expensive; custom engines enable unique designs; and you lose control over the roadmap. — [VGI](https://app.sensortower.com/vgi/assets/reports/The_Big_Game_Engines_Report_of_2025.pdf)
- **Licensing trust:** the 2023 Unity fee caused mass migration interest and was fully reversed a year later. — [Unity](https://unity.com/blog/unity-is-canceling-the-runtime-fee); [GWO](https://gameworldobserver.com/2024/03/27/godot-doubled-user-base-after-unity-controversy)
- **Platform and tooling breakage:** LibGDX/Java broke with OS updates and lacked consoles, which pushed Mega Crit to switch. — [Wikipedia](https://en.wikipedia.org/wiki/Slay_the_Spire_II)
- **Console support** is a real constraint for open-source engines. GameMaker's free tier excludes consoles. — [GameMaker FAQ](https://gamemaker.io/en/help/articles/november-2023-pricing-terms-change-faq)
- **Engine-less advocates** (Noel Berry) stress iteration speed through tools they build themselves, a small scope, and independence from vendor decisions. They accept higher upfront cost and poorer suitability for large 3D projects. — [noelberry.ca](https://noelberry.ca/posts/making_games_in_2025/)

### Inferences
- The market is polarising:
  - AAA is consolidating onto Unreal, with the remaining custom engines.
  - Indies are splitting between Unity (incumbent) and Godot (trust and open source), with a small but vocal segment that builds its own engine on top of SDL-style libraries.

### Gaps
- I found no quantitative cross-engine survey on pain points (such as "updates break projects" or iteration time). Reddit r/gamedev and Hacker News threads were not fetched in this session.
