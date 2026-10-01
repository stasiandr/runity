# Редакторы клипов: что хвалят и за что ругают (2026-10)

Это контекст для [clips.md](../clips.md), а не правила. Источники —
форумы, devtalk, документация и школы анимации. Пометка «(общ.)» значит,
что пункт опирается на общую практику, а не на найденный источник.

## Unity Animation window

**Хвалят:**

* низкий порог входа;
* Record: крутишь объект в сцене — ставится ключ;
* ключ на любое сериализуемое свойство;
* Dopesheet и Curves в одном окне, события прямо на таймлайне.

**Ругают:**

* **Клипы из FBX только для чтения.** Приходится делать Ctrl+D, копия
  теряет настройки импорта и не переживает реимпорт.
  https://discussions.unity.com/t/how-to-add-keyframes-on-imported-read-only-animation/31923
* **Случайная запись.** Скраб с включённым Record портит анимацию. Просят
  отделить превью от записи и громче показывать, что запись идёт.
  https://discussions.unity.com/threads/animation-window-improvements.427397/
* **Тангенты.** Auto даёт перелёт между близкими ключами. Box-scale
  сбрасывает тип тангентов. Weighted-тангентов нет, меню тангентов есть не
  во всех видах.
  https://forum.unity.com/threads/scaling-key-positions-in-animation-window-results-in-unexpected-curves.434486/
* **Скорость.** Окно открывается 10–15 с на гуманоиде.
  https://discussions.unity.com/t/animation-window-is-so-slow-its-unusable/842960
* Нет onion skin, motion trail, pose library и слоёв в клипе. Флипы Euler.
  (общ.)

## Blender (Dope Sheet, Graph Editor, NLA, Pose Library)

**Хвалят:**

* **Слайдеры Graph Editor 4.0:** Blend to Neighbor, Breakdown, Ease,
  Push/Pull, Time Offset, Match Slope — с overshoot.
  https://developer.blender.org/docs/release_notes/4.0/animation_rigging/
* Фильтры Butterworth и Gaussian для мокапа.
* **Pose Library на Asset Shelf:** смешивание позы слайдером, применение
  отзеркаленной позы. https://code.blender.org/2021/05/pose-library-v2-0/
* Недеструктивные модификаторы кривых.

**Ругают:**

* **NLA непонятен:** push down, stash, tweak mode, fake user.
* **Slotted Actions 4.4:** ключи попадают «не туда», активный слот не
  виден. https://devtalk.blender.org/t/blender-4-4-slotted-actions-feedback/38906
* Кватернионы по умолчанию. Нет авто-кадрирования выбранной кривой.

## Unreal (Sequencer, Control Rig, Curve Editor)

**Хвалят:**

* в 5.5–5.6 tween-инструменты встроены в Curve Editor и вызываются
  хоткеями;
* motion trail правится во вьюпорте;
* Animation Mode;
* Layered и Additive Control Rig поверх мокапа без запекания.
  https://www.awn.com/news/unreal-engine-56-brings-exciting-new-features-animators

**Ругают:**

* **Многоступенчатая правка.** Нельзя просто открыть Sequence и двигать
  ключи: «положи в уровень → bake to Control Rig → правь → bake назад».
  https://boardgamers2409.itch.io/plague-wardens/devlog/799201/editing-animations-in-ue5-clayton-jetton
* **Потеря ключей.** Ключи пропадают при перезапуске. Перекомпиляция рига
  удаляет треки. https://forums.unrealengine.com/t/sequencer-problems/2232548
* Анимация в Sequencer и в PIE выглядит по-разному.

## Maya + Tween Machine, animBot, aTools

**Хвалят:**

* Graph Editor — эталон;
* stepped-блокинг, потом spline;
* buffer curves;
* weighted-тангенты;
* **Tween Machine / Tweener** — один слайдер «к левому ↔ к правому ключу»,
  его называют лучшим инструментом;
* **animBot**:
  * семейство Blend to Buffer / Default / Ease / Neighbors;
  * Nudge;
  * temp pivot;
  * motion trail;
  * selection sets.

  https://animbot.ca/tooltips/ ,
  https://blog.animschool.edu/2021/02/25/top-10-tools-and-plug-ins-essential-for-maya-animators/

**Ругают:** голую Maya без плагинов, флипы на 360° в мокапе.

## Must-have, по важности

1. Отклик на плотных данных: 60+ костей, ключ на каждом кадре.
2. Превью отдельно от записи, громкая индикация записи, ручной Key.
3. Правка импорта без потери связи с исходником.
4. Полные тангенты, тип которых сохраняется при сдвиге и масштабе.
5. Общее выделение в Dopesheet и Curves, box-tool, retime.
6. Численный ввод, frame selected, изоляция каналов.
7. Stepped ↔ spline одной кнопкой.
8. Undo, которое не теряет ключи.
9. Повороты без флипов (Euler filter или unroll).
10. Onion skin и motion trail.

## Ловушки

* Read-only импорт плюс дублирование (Unity).
* Record пишет ключи из скрытого окна (Unity).
* Масштаб ключей сбрасывает тангенты (Unity).
* Bake туда и обратно (Unreal).
* Пересборка рига молча удаляет треки (Unreal): привязывать треки к
  стабильным именам и не удалять ключи осиротевших каналов.
* Превью отличается от игры (Unreal).
* Данные живут, только если на них есть «пользователь» (Blender fake user).
* Неявное назначение клипа объекту (Blender Slotted Actions).

## Идеи сверх Unity, по приоритету

1. Слой правок поверх импорта, переживающий реимпорт (UE Additive Control
   Rig, animBot layers).
2. Tween-слайдер с overshoot (Tween Machine, animBot, Blender, UE 5.6).
3. Семейство blend-слайдеров (Blender 4.0, animBot).
4. Buffer curve (Maya, animBot).
5. Stepped ↔ spline одной кнопкой (Maya).
6. Pose library во вьюпорте, с зеркалом (Blender, Studio Library).
7. Motion trail во вьюпорте (UE 5.6, animBot).
8. Onion skin (bhGhost).
9. Фильтры мокапа: Butterworth/Gaussian, decimate, Euler filter.
10. Temp pivot / world-space control (animBot).
11. Дорожка клипов в духе NLA, но без fake user и stash.
12. Одна Action на несколько связанных сущностей с видимым назначением.
13. Nudge и retime без дробных кадров (animBot).
14. Selection sets, изоляция каналов, авто-кадрирование.
