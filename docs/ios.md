# iOS — игра на scrap как приложение iPhone и iPad

Платформа по порядку DNA (macOS, Windows, веб, Linux, **iOS**, Android) —
теперь есть: та же игра, тот же код, нативно (Metal через wgpu, не веб в
обёртке). Доказательство — `examples/kitchen` и Not Divinity в Симуляторе
и на iPhone 15.

## Как собрать и запустить

```
scrap run   --platform ios-sim [--device "iPhone 17"] [--env ND_MUTE=1] [--console]
scrap build --platform ios-sim [--debug]          # build/ios-sim/<Title>.app
scrap build --platform ios [--device UDID]        # build/ios/<Title>.app, подписано
scrap run   --platform ios [--device UDID] [--release]   # собрать, поставить, запустить
```

`ios` — телефон (`aarch64-apple-ios`), `ios-sim` — Симулятор на Mac с Apple
silicon (`aarch64-apple-ios-sim`). Нужны Xcode (actool, codesign, simctl,
devicectl) и цели Rust: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`.

Что кладётся в `<Title>.app` — то же, что `scrap build` кладёт в папку
(`crates/scrap-cli/src/ios.rs`):

* исполняемый файл игры и `data/` рядом: `scrap.ron`, привязки, сцены,
  префабы, экраны, строки и **библиотека, сваренная для телефона** (ASTC
  4×4, zstd); плюс то, что игра читает сама и чего стандартная раскладка не
  знает — `scrap.ron` → `build: (ship: ["content/**/*.json", …])`;
* `Info.plist`: имя (`game.title`), идентификатор (`build.bundle_id`, иначе
  `dev.scrap.<имя>`), версия из `Cargo.toml` игры, номер сборки — число
  коммитов, ориентации (`build.orientation`: `landscape`, `portrait`, `any`),
  статус-бар скрыт, экран целиком (`UIRequiresFullScreen`), сцены UIKit с
  делегатом движка `ScrapSceneDelegate` (iOS 27 без сцен приложение
  останавливает), минимум iOS 16;
* иконка (`build.icon`, квадрат 1024 px) и цвет экрана запуска
  (`build.background`) — каталог ассетов, собранный `actool`.

Пример (`not-divinity/scrap.ron`):

```ron
build: (
    ship: ["config/*.ron", "content/**/*.json", "content/**/ui/**/*.png"],
    bundle_id: "com.example.game",
    icon: "web/icons/icon-1024.png",
    orientation: "landscape",
    background: "#0b0a0d",
),
```

### Подпись и установка

Симулятор — подпись ad hoc. Телефон — подпись для разработки, **без проекта
Xcode**: идентичность «Apple Development» из связки ключей и профиль, который
Xcode уже скачал (`~/Library/Developer/Xcode/UserData/Provisioning Profiles`):
сначала точный под `bundle_id`, иначе подстановочный команды (`TEAM.*`), в
котором есть сертификат идентичности и (если телефон назван) сам телефон.
Права — из профиля, его `*` заменено на идентификатор приложения; профиль
кладётся в `embedded.mobileprovision`. Выбрать руками: `--identity`,
`--profile FILE`. Если подходящего профиля нет, команда говорит, что сделать:
войти в Xcode с командой и один раз собрать любое приложение для этого
телефона с автоматической подписью. Сертификаты и аккаунт движок не трогает.

Установка — `xcrun devicectl device install app --device <UDID> <app>`:
по кабелю или по сети, если телефон спарен с этим Mac, разблокирован, в той
же Wi-Fi и на нём включён Developer Mode. `scrap run --platform ios` сначала
спрашивает `devicectl`, доступен ли телефон, и если нет — печатает готовую
команду установки.

## Что движок делает на iOS

* **Хост** — winit (UIKit): цикл событий не возвращает; окно кладётся в
  сцену UIKit делегатом движка (`scrap-shell/src/ios.rs`). Статус-бар скрыт,
  полоска «домой» гаснет, свайп от края сперва игре
  (`preferredScreenEdgesDeferringSystemGestures`).
* **Фон** (`Suspended`/`Resumed` winit = resign/become active): ничего не
  шагается и не рисуется (из фона iOS не даёт отправлять работу на GPU), пальцы
  отпущены, часы при возвращении начинают заново, игре сказано
  `Game::active(false|true)`. Слой Metal UIKit сохраняет — поверхность та же.
* **Экран**: рисуется весь экран в пикселях (iPhone 15 — 2556×1179 @3x);
  `Context::safe_area` — вставки челки, скруглений и полоски «домой».
  `WindowConfig::letterbox: Some(16/9)` — картинка игры нужного соотношения,
  самая большая в безопасной области, по центру, чёрное вокруг; игре
  говорится размер этого прямоугольника, её виды и оверлей сдвигаются туда,
  касания — тоже. Масштаб рендера считается от настоящих пикселей, как на
  Mac.
* **Данные**: `data/` внутри бандла; `GameSettings::load` направляет все
  чтения путей проекта (`env!("CARGO_MANIFEST_DIR")`, которого на телефоне
  нет) в `data/` (`scrap::files::route`), так что код, называющий файл от
  корня проекта, находит его и на телефоне. Файлы игрока —
  `Library/Application Support/<игра>` в контейнере приложения
  (`player_prefs::user_dir`).
* **Звук**: kira поверх cpal (Core Audio). Сессия — по умолчанию iOS
  (`SoloAmbient`: беззвучный переключатель глушит игру, чужая музыка
  останавливается); прерывания (звонок, Siri) cpal переживает сам — после
  конца прерывания поток запускается снова.
* **Касания** — `scrap::touch_pad`, на телефоне по умолчанию:
  * `TouchLayout::standard()` — для игры от первого лица: стик под левым
    большим пальцем, взгляд — протяжкой, кнопки; только пока указатель
    захвачен;
  * `TouchLayout::gamepad()` — **целый геймпад на стекле**, как `touch.js`
    Not Divinity: два стика, приходящих под пальцы, ✕ ○ □ △, L1 L2 R1 R2,
    Select и Start; это **отдельный геймпад со своим слотом игрока**
    (`PadConnected`, `PadDownOn`…), так что игра, рассаживающая игроков по
    геймпадам, сажает палец как геймпад и показывает его глифы; системы игры
    не меняются. Виден, пока касание — устройство в руках: клавиша или
    настоящий геймпад его убирают (слот отпускается, как вынутый геймпад),
    касание возвращает. Всё — в безопасной области. `SCRAP_TOUCH_PAD=pad`
    включает его на Mac.
* **Геймпады MFi, DualSense, Xbox, Switch Pro** — через GameController
  (docs/gamepads.md), как на macOS: слоты по порядку, кнопки по месту,
  свет, вибрация, адаптивные курки, тачпад, датчики, батарея. Нажатый
  настоящий геймпад убирает геймпад на стекле (тот отпускает свой слот),
  касание возвращает его. Пока приложение в фоне, геймпады молчат, хаптика
  остановлена.
* **Клавиатура на стекле**: `Context::show_keyboard(true)` — клавиатура
  UIKit; набранное приходит как с клавиатуры (`Text`, Backspace, `"\n"`).
* **Сокет управления** (`docs/world-control.md`) работает и на iOS: в
  Симуляторе процесс игры — процесс Mac, сокет кладётся по пути из
  `SCRAP_CONTROL` и `scrap world --socket ПУТЬ …` управляет игрой. Так
  проверяют сборку для Симулятора без касаний: `simctl` их не умеет.
* **Steam** на телефоне и в браузере — тот же API без SDK (`Steam::init`
  говорит «не на этой платформе»): игра со списком модулей, где есть `steam`,
  собирается везде.

## Проверка в Симуляторе

```
scrap run --platform ios-sim --env SCRAP_CONTROL=/tmp/game.sock
scrap world --socket /tmp/game.sock hello
scrap world --socket /tmp/game.sock shot path=$PWD/target/shots/ios.png
xcrun simctl io booted screenshot target/shots/ios_sim.png
```

Звук Симулятора идёт в динамики Mac: игре, которую гоняют автоматически,
давать её флаг тишины (у Not Divinity `ND_MUTE=1`).

## Чего ещё нет

* Своего `UIViewController` у движка нет (winit — удобный путь, как на
  десктопе; правило 1 docs/stack.md держится: движок гость на чужой
  поверхности).
* Распространение (TestFlight, App Store, ad hoc .ipa) — не сделано:
  только подпись для разработки и установка на свой телефон.
* Android — тот же код оболочки (`touch_pad`, фон), упаковки ещё нет.
