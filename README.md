# runity

Игровой движок на Rust **без единой сторонней зависимости**. Весь код собирается
из `std` — ни `winit`, ни `wgpu`, ни `glam`, ни `image`, ни `libc`.

Основная платформа — **macOS** (AppKit через Objective-C runtime); X11 и Win32
уже работают, Wayland и мобильные — потом.

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
| Рендер | `runity-render` | программируемый софтварный растеризатор, буфер глубины, текстуры, меши, загрузчик OBJ, PNG/PPM-энкодер |
| Платформа | `runity-platform` | окно и ввод: **macOS через Objective-C runtime**, X11 напрямую по wire-протоколу, Win32 через `user32`/`gdi32`, headless-бэкенд |
| Движок | `runity-core` | ECS на поколениях, время с фиксированным шагом, состояние ввода, главный цикл |
| Фасад | `runity` | реэкспорт + `prelude`, примеры |

Конвейер рендера — тот же, что в GPU, только на CPU:

```
Mesh → Shader::vertex → отсечение по ближней плоскости → деление на w →
viewport → отсечение задних граней → растеризация (top-left rule) →
перспективно-корректная интерполяция → тест глубины → Shader::fragment → блендинг
```

## Запуск

```bash
cargo run --release --example spinning_cube      # окно: macOS, X11 или Win32
cargo run --release --example hello_triangle

RUNITY_HEADLESS=1 cargo run --release --example spinning_cube   # без дисплея, пишет cube.png
cargo test --workspace                                          # 90+ тестов, дисплей не нужен

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
многопоточная растеризация по тайлам, mip-mapping, отсечение по frustum и
иерархия трансформов, HiDPI на macOS, загрузка glTF, звук. Wayland и мобильные
платформы — потом.

## Лицензия

MIT OR Apache-2.0.
