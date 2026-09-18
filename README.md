# runity

Игровой движок на Rust **без единой сторонней зависимости**. Весь код собирается
из `std` — ни `winit`, ни `wgpu`, ни `glam`, ни `image`, ни `libc`.

Основная платформа — **macOS** (AppKit через Objective-C runtime); X11 и Win32
уже работают, Wayland и мобильные — потом.

Рендерер здесь — инструмент **отладки и headless-разработки**, а не способ
выжать кадры в секунду: он однопоточный и таким останется. Взамен он считает
физически корректный свет — PBR, IBL, тени, SSAO, отражения, bloom — и всё, что
рисуется, можно посмотреть послойно и проверить пиксель в пиксель без дисплея.

```
$ cargo tree -p runity            # в дереве только крейты этого репозитория
runity v0.1.0
├── runity-core v0.1.0
│   ├── runity-math v0.1.0
│   ├── runity-platform v0.1.0
│   └── runity-render v0.1.0
│       └── runity-math v0.1.0
├── runity-math v0.1.0
├── runity-platform v0.1.0
└── runity-render v0.1.0
```

![showcase](docs/showcase.png)

Кадр выше посчитан полностью на CPU, в один поток, и сохранён своим
PNG-энкодером: `RUNITY_HEADLESS=1 RUNITY_SUPERSAMPLE=2 cargo run --release
--example showcase`. Шары — шкала шероховатости: металл сзади, диэлектрик
спереди, слева зеркало, справа матовое.

## Что уже работает

| Слой | Крейт | Содержимое |
| --- | --- | --- |
| Математика | `runity-math` | `Vec2/3/4`, `Mat4` (колоночная, как в GLSL), `Quat`, проекции, `look_at`, инверсия матриц |
| Рендер | `runity-render` | отложенный растеризатор с PBR, IBL, тенями, SSAO, SSR, bloom и оптикой; мип-маппинг, debug-виды, golden-тесты, PNG (запись и чтение) + DEFLATE |
| Платформа | `runity-platform` | окно и ввод: **macOS через Objective-C runtime**, X11 напрямую по wire-протоколу, Win32 через `user32`/`gdi32`, headless-бэкенд |
| Движок | `runity-core` | ECS на поколениях, время с фиксированным шагом, состояние ввода, главный цикл, headless-прогон и запись кадров |
| Фасад | `runity` | реэкспорт + `prelude`, примеры |

Конвейер — тот же, что в GPU, только на CPU и в один поток:

```
геометрия  Mesh → vertex → отсечение по ближней плоскости → /w → viewport →
           отсечение задних граней → растеризация (top-left rule) →
           перспективно-корректная интерполяция → тест глубины → G-buffer
свет       SSAO → PBR (GGX + Smith + Шлик) с тенями и IBL → небо
экран      SSR → bloom → вигнетка/зерно/аберрации → суперсэмплинг →
           экспозиция → ACES → sRGB
```

Всё внутри — линейный HDR без верхней границы; в 8 бит кадр превращается ровно
один раз, в самом конце.

## Отладка

Один переключатель `engine.debug_view` меняет то, что показывает цикл, — код
игры об этом ничего не знает:

| Вид | Что показывает |
| --- | --- |
| `Shaded` | сцену как есть |
| `Wireframe` | те же draw-вызовы, но только рёбра треугольников — через шейдер и с тестом глубины, а не линиями поверх кадра |
| `Depth` | буфер глубины, нормированный по содержимому кадра |
| `Overdraw` | сколько раз переписан каждый пиксель |
| `Albedo` | цвет поверхности до освещения |
| `Normals` | нормали в мировом пространстве |
| `Material` | шероховатость в зелёном, металличность в синем |
| `Occlusion` | результат SSAO отдельно от всего остального |

Поверх кадра можно дорисовать `engine.draw_normals(...)`, `engine.draw_axes(...)`
и `engine.draw_wireframe(...)`.

В примерах это клавиши 1–8, а headless — переменная
`RUNITY_DEBUG_VIEW=wireframe|depth|overdraw|albedo|normals|material|occlusion`.
В `showcase` клавиши F1–F5 включают и выключают SSAO, отражения, bloom, оптику
и тени по отдельности — чтобы видеть, что именно каждый эффект даёт.

## Headless-разработка

Кадр можно получить без окна, цикла и дисплея:

```rust
let frame = headless::render(320, 240, |engine| {
    let shader = engine.lit_shader(Mat4::IDENTITY);
    engine.draw(&Mesh::cube(1.0), &shader);
});
save_png("frame.png", &frame)?;
```

Для анимации есть `headless::run` (N кадров с фиксированным шагом — прогон
детерминирован) и `headless::record` (то же, но с сохранением каждого кадра,
`headless::save_frames` разложит их по PNG).

Регрессии ловятся сравнением с эталоном:

```rust
golden::assert_matches("tests/golden/scene.png", &frame, Tolerance::new(4, 0.02));
```

Эталона нет — он создастся при первом запуске. Разошлось — рядом лягут
`scene.actual.png` и `scene.diff.png` с подсвеченной разницей, а в тексте
падения будет, сколько пикселей и насколько разъехались. Изменение осознанное —
`RUNITY_UPDATE_GOLDEN=1 cargo test`, и в ревью видно ровно то, что поменялось на
экране.

Чтобы прочитать эталон, нужен полноценный распаковщик: `runity-render` умеет
DEFLATE (stored, fixed и dynamic Huffman) и читает 8-битные PNG — greyscale,
RGB, палитру и варианты с альфой, со всеми пятью фильтрами строк.

## Запуск

```bash
cargo run --release --example showcase           # окно: macOS, X11 или Win32
cargo run --release --example spinning_cube
cargo run --release --example hello_triangle

RUNITY_HEADLESS=1 cargo run --release --example showcase         # без дисплея, пишет showcase.png
RUNITY_HEADLESS=1 RUNITY_SUPERSAMPLE=2 cargo run --release --example showcase
cargo run --release --example bench                             # сколько стоит каждый проход
cargo test --workspace                                          # 218 тестов, дисплей не нужен

# кросс-проверка бэкендов, которые нельзя собрать на текущей машине
cargo check --workspace --target aarch64-apple-darwin
cargo check --workspace --target x86_64-apple-darwin
cargo check --workspace --target x86_64-pc-windows-gnu
```

Управление в `spinning_cube`: стрелки или WASD — орбита камеры, Q/E — зум,
Escape — выход.

## Как это выглядит в коде

Обычный рендер — это материал и матрица:

```rust
engine.draw_pbr(
    &mesh,
    Mat4::from_translation(position),
    &Material::metal(Color::rgb(0.95, 0.78, 0.42), 0.15),
);
```

Материал — metal/roughness плюс карты (albedo, normal, metallic-roughness,
emissive), свет — направленный, точечный или конусный, окружение —
процедурное небо, из которого получаются и фон, и отражения, и рассеянный
свет, и солнце как источник:

```rust
engine.renderer.set_sky(Sky::new(SkyParams::golden_hour()));
engine.renderer.lights.push(Light::point(position, Color::rgb(1.0, 0.45, 0.18), 9.0, 7.0));
```

А если нужен свой шейдер — это по-прежнему просто пара функций, и растеризатор
интерполирует всё, что вернёт вершинная стадия:

```rust
struct Gradient { mvp: Mat4 }

impl Shader for Gradient {
    type Varying = Color;

    fn vertex(&self, v: &Vertex) -> VertexOutput<Color> {
        VertexOutput { clip_position: self.mvp.transform_point(v.position), varying: v.color }
    }

    fn fragment(&self, color: &Color) -> Option<Color> {
        Some(*color)          // None = discard
    }
}
```

## Правила по зависимостям

1. В `Cargo.toml` любого крейта разрешены только пути на соседние крейты
   воркспейса. Внешних зависимостей нет — включая dev-зависимости.
2. `unsafe` запрещён везде (`#![forbid(unsafe_code)]`), кроме Win32-бэкенда:
   там без FFI не обойтись, и каждый `unsafe`-блок сопровождается
   комментарием об инвариантах.
3. Всё, что нельзя проверить на машине без дисплея, должно быть проверяемо
   иначе: X11-бэкенд тестируется против поддельного X-сервера
   (`crates/runity-platform/tests/x11_wire.rs`), рендер — сравнением с
   независимо посчитанным пересечением луча с плоскостью
   (`crates/runity-render/tests/pipeline.rs`), а разбор событий Cocoa вынесен
   в модуль без единого вызова Objective-C, чтобы его тесты шли на любой
   машине (`crates/runity-platform/src/macos_keys.rs`).

Подробности о том, как вообще рисовать без зависимостей, — в
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Сколько это стоит

Один поток, кадр 640×360 со сценой уровня `showcase` (числа с машины, где это
писалось; `cargo run --release --example bench` посчитает для вашей):

| Что включено | Секунд на кадр |
| --- | --- |
| всё | 0.50 |
| без SSAO | 0.34 |
| без SSR | 0.38 |
| только геометрия и свет | 0.19 |
| всё, 2× суперсэмплинг | 2.41 |

Дороже всего SSAO и SSR, тени почти бесплатны. Для интерактивной работы эффекты
выключаются по одному; для стилла включается всё и добавляется суперсэмплинг.

## Чего ещё нет

Сейчас это твёрдая основа, а не законченный движок. Ближайшее по списку:
отсечение по frustum, иерархия трансформов, глубина резкости и motion blur,
каскады теней, площадные источники, загрузка glTF, текстовый оверлей со
статистикой прямо в кадре, HiDPI на macOS, сжимающий PNG-энкодер.

Чего в списке нет и не будет: многопоточной и SIMD-растеризации. Рендерер нужен
для отладки и headless-прогона, а однопоточный проход проще читать, и он
детерминирован — а на детерминизме держатся golden-тесты.

## Лицензия

MIT OR Apache-2.0.
