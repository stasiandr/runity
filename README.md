# runity

Игровой движок на Rust **без единой сторонней зависимости**. Весь код собирается
из `std` — ни `winit`, ни `wgpu`, ни `glam`, ни `image`, ни `libc`.

Основная платформа — **macOS** (AppKit через Objective-C runtime); X11 и Win32
уже работают, Wayland и мобильные — потом.

Рендерер здесь — инструмент **отладки и headless-разработки**, а не способ
выжать кадры в секунду: он однопоточный и таким останется. Взамен всё, что
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

![spinning cube](docs/cube.png)

Кадр выше отрисован полностью на CPU и сохранён своим PNG-энкодером:
`RUNITY_HEADLESS=1 cargo run --release --example spinning_cube`.

## Что уже работает

| Слой | Крейт | Содержимое |
| --- | --- | --- |
| Математика | `runity-math` | `Vec2/3/4`, `Mat4` (колоночная, как в GLSL), `Quat`, проекции, `look_at`, инверсия матриц |
| Рендер | `runity-render` | программируемый софтварный растеризатор, буфер глубины, текстуры, меши, OBJ, debug-виды, golden-тесты, PNG (запись и чтение) + DEFLATE |
| Платформа | `runity-platform` | окно и ввод: **macOS через Objective-C runtime**, X11 напрямую по wire-протоколу, Win32 через `user32`/`gdi32`, headless-бэкенд |
| Движок | `runity-core` | ECS на поколениях, время с фиксированным шагом, состояние ввода, главный цикл, headless-прогон и запись кадров |
| Фасад | `runity` | реэкспорт + `prelude`, примеры |

Конвейер рендера — тот же, что в GPU, только на CPU:

```
Mesh → Shader::vertex → отсечение по ближней плоскости → деление на w →
viewport → отсечение задних граней → растеризация (top-left rule) →
перспективно-корректная интерполяция → тест глубины → Shader::fragment → блендинг
```

## Отладка

Один переключатель `engine.debug_view` меняет то, что показывает цикл, — код
игры об этом ничего не знает:

| Вид | Что показывает |
| --- | --- |
| `Shaded` | сцену как есть |
| `Wireframe` | те же draw-вызовы, но только рёбра треугольников — через шейдер и с тестом глубины, а не линиями поверх кадра |
| `Depth` | буфер глубины, нормированный по содержимому кадра |
| `Overdraw` | сколько раз переписан каждый пиксель |

Поверх кадра можно дорисовать `engine.draw_normals(...)`, `engine.draw_axes(...)`
и `engine.draw_wireframe(...)`.

В примере это клавиши 1–4 и N, а headless — переменная
`RUNITY_DEBUG_VIEW=wireframe|depth|overdraw`.

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
cargo run --release --example spinning_cube      # окно: macOS, X11 или Win32
cargo run --release --example hello_triangle

RUNITY_HEADLESS=1 cargo run --release --example spinning_cube   # без дисплея, пишет cube.png
cargo test --workspace                                          # 129 тестов, дисплей не нужен

# кросс-проверка бэкендов, которые нельзя собрать на текущей машине
cargo check --workspace --target aarch64-apple-darwin
cargo check --workspace --target x86_64-apple-darwin
cargo check --workspace --target x86_64-pc-windows-gnu
```

Управление в `spinning_cube`: стрелки или WASD — орбита камеры, Q/E — зум,
Escape — выход.

## Свой шейдер

Шейдер — это просто пара функций; растеризатор интерполирует всё, что вернёт
вершинная стадия:

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

## Чего ещё нет

Сейчас это твёрдая основа, а не законченный движок. Ближайшее по списку:
mip-mapping, отсечение по frustum, иерархия трансформов, текстовый оверлей для
статистики прямо в кадре, загрузка glTF, HiDPI на macOS.

Чего в списке нет и не будет: многопоточной и SIMD-растеризации. Рендерер нужен
для отладки и headless-прогона, а однопоточный проход проще читать, и он
детерминирован — а на детерминизме держатся golden-тесты.

## Лицензия

MIT OR Apache-2.0.
