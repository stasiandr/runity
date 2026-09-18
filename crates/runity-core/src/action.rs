//! Named actions instead of raw keys.
//!
//! Reading `Key::W` in game code works right up until three things are wanted
//! at once: rebindable controls, a second input device, and multiplayer. The
//! last one decides the design. What travels over the wire cannot be a
//! keystroke — the other machine has different bindings and may not even have
//! a keyboard — so it has to be the *intent*: "this player wanted to move
//! forward at 0.7 this tick". [`Intent`] is that, in a form small enough to
//! send every tick and exact enough to replay identically on both ends.
//!
//! The layers, in order:
//!
//! * [`Bindings`] — configuration. Which physical inputs mean which action.
//!   Saveable, so a player's keymap survives a restart.
//! * [`Actions`] — this frame's state, evaluated from [`Bindings`] and
//!   [`Input`](crate::input::Input).
//! * [`Intent`] — a compact, deterministic snapshot to send or record.

use crate::input::Input;
use runity_platform::{Key, MouseButton};
use runity_serialize::{Deserialize, Error, Reader, Result, Serialize, Writer};

/// A named action, identified by a hash of its name.
///
/// Hashing rather than interning keeps `Action` `Copy` and comparable without
/// a registry to consult, and keeps saved bindings readable across builds: the
/// id of `"jump"` is the same number in every version of the game, whereas an
/// index into a list changes the moment an action is inserted above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Action(pub u32);

impl Action {
    /// The action with this name. `const`, so actions can be constants.
    pub const fn named(name: &str) -> Self {
        // FNV-1a, 32-bit — the same construction the RNG uses for stream
        // names, narrowed because an action set is tens of entries, not
        // billions.
        let bytes = name.as_bytes();
        let mut hash: u32 = 0x811c_9dc5;
        let mut index = 0;
        while index < bytes.len() {
            hash ^= bytes[index] as u32;
            hash = hash.wrapping_mul(0x0100_0193);
            index += 1;
        }
        Self(hash)
    }
}

/// One physical input that can trigger an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Key(Key),
    Mouse(MouseButton),
}

/// Where a continuous value comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AxisBinding {
    /// Two keys pulling in opposite directions: the classic `A`/`D` pair.
    Keys {
        negative: Binding,
        positive: Binding,
    },
    /// Horizontal mouse movement this frame, in pixels times `scale`.
    MouseX { scale: f32 },
    /// Vertical mouse movement this frame, in pixels times `scale`.
    MouseY { scale: f32 },
    /// Wheel movement this frame, times `scale`.
    Scroll { scale: f32 },
}

/// Which physical inputs mean which actions.
///
/// An action may have several bindings and they are all live at once, which is
/// what makes "WASD or the arrow keys" a configuration rather than a special
/// case in game code.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bindings {
    buttons: Vec<(Action, Binding)>,
    axes: Vec<(Action, AxisBinding)>,
}

impl Bindings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a binding, keeping any the action already has.
    pub fn bind(&mut self, action: Action, binding: Binding) -> &mut Self {
        if !self.buttons.contains(&(action, binding)) {
            self.buttons.push((action, binding));
        }
        self
    }

    /// Add a key binding — the common case, spelled shorter.
    pub fn key(&mut self, action: Action, key: Key) -> &mut Self {
        self.bind(action, Binding::Key(key))
    }

    /// Add an axis source, keeping any the action already has.
    pub fn bind_axis(&mut self, action: Action, binding: AxisBinding) -> &mut Self {
        self.axes.push((action, binding));
        self
    }

    /// Add an axis driven by two keys.
    pub fn key_axis(&mut self, action: Action, negative: Key, positive: Key) -> &mut Self {
        self.bind_axis(
            action,
            AxisBinding::Keys {
                negative: Binding::Key(negative),
                positive: Binding::Key(positive),
            },
        )
    }

    /// Drop every binding of one action. Rebinding is this plus [`bind`].
    ///
    /// [`bind`]: Bindings::bind
    pub fn clear_action(&mut self, action: Action) -> &mut Self {
        self.buttons.retain(|(bound, _)| *bound != action);
        self.axes.retain(|(bound, _)| *bound != action);
        self
    }

    /// Replace every binding of one action with a single key.
    pub fn rebind(&mut self, action: Action, key: Key) -> &mut Self {
        self.clear_action(action).key(action, key)
    }

    /// The button bindings of one action, in the order they were added.
    pub fn bindings_for(&self, action: Action) -> impl Iterator<Item = Binding> + '_ {
        self.buttons
            .iter()
            .filter(move |(bound, _)| *bound == action)
            .map(|(_, binding)| *binding)
    }

    /// The axis sources of one action, in the order they were added.
    pub fn axes_for(&self, action: Action) -> impl Iterator<Item = AxisBinding> + '_ {
        self.axes
            .iter()
            .filter(move |(bound, _)| *bound == action)
            .map(|(_, binding)| *binding)
    }

    /// Every action some other action would also answer to.
    ///
    /// A settings screen needs this before it accepts a new key: silently
    /// binding jump to the key that already crouches is a bug report.
    pub fn conflicts(&self, binding: Binding) -> Vec<Action> {
        let mut found: Vec<Action> = self
            .buttons
            .iter()
            .filter(|(_, bound)| *bound == binding)
            .map(|(action, _)| *action)
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// This frame's actions, evaluated from bindings and raw input.
#[derive(Debug, Clone, Default)]
pub struct Actions {
    bindings: Bindings,
    down: Vec<Action>,
    was_down: Vec<Action>,
    values: Vec<(Action, f32)>,
}

impl Actions {
    pub fn new(bindings: Bindings) -> Self {
        Self {
            bindings,
            ..Self::default()
        }
    }

    pub fn bindings(&self) -> &Bindings {
        &self.bindings
    }

    /// Edit the bindings. The next [`update`](Actions::update) uses them.
    pub fn bindings_mut(&mut self) -> &mut Bindings {
        &mut self.bindings
    }

    /// Recompute from this frame's input. Call once per frame, after the
    /// events have been fed to `input`.
    pub fn update(&mut self, input: &Input) {
        std::mem::swap(&mut self.down, &mut self.was_down);
        self.down.clear();
        for (action, binding) in &self.bindings.buttons {
            // Several bindings of one action: held by any of them is held.
            if held(input, *binding) && !self.down.contains(action) {
                self.down.push(*action);
            }
        }
        self.down.sort_unstable();

        self.values.clear();
        for (action, binding) in &self.bindings.axes {
            let value = axis_value(input, *binding);
            match self.values.iter_mut().find(|(bound, _)| bound == action) {
                // Two sources for one axis add: a stick and the keys should
                // not fight over which one wins.
                Some((_, total)) => *total += value,
                None => self.values.push((*action, value)),
            }
        }
    }

    /// Whether the action is held right now.
    pub fn down(&self, action: Action) -> bool {
        self.down.binary_search(&action).is_ok()
    }

    /// Whether the action went down this frame.
    ///
    /// The edge is computed from the previous frame's *actions*, not from the
    /// keys: swapping two bindings between frames must not look like a press,
    /// and releasing one of two bound keys while the other is still held must
    /// not look like a release and a press.
    pub fn pressed(&self, action: Action) -> bool {
        self.down(action) && !self.was_down.contains(&action)
    }

    /// Whether the action came up this frame.
    pub fn released(&self, action: Action) -> bool {
        !self.down(action) && self.was_down.contains(&action)
    }

    /// The value of an axis this frame; `0.0` if it has no bindings.
    pub fn value(&self, action: Action) -> f32 {
        self.values
            .iter()
            .find(|(bound, _)| *bound == action)
            .map(|(_, value)| *value)
            .unwrap_or(0.0)
    }

    /// Pack this frame into the form that travels: see [`Intent`].
    pub fn intent(&self, set: &ActionSet) -> Intent {
        let mut intent = Intent::for_set(set);
        for (index, action) in set.buttons.iter().enumerate() {
            if self.down(*action) {
                intent.buttons |= 1 << index;
            }
        }
        for (index, action) in set.axes.iter().enumerate() {
            intent.axes[index] = quantize(self.value(*action));
        }
        intent
    }
}

/// Whether one physical input is held.
fn held(input: &Input, binding: Binding) -> bool {
    match binding {
        Binding::Key(key) => input.key_down(key),
        Binding::Mouse(button) => input.mouse_down(button),
    }
}

/// One axis source's contribution this frame.
fn axis_value(input: &Input, binding: AxisBinding) -> f32 {
    match binding {
        AxisBinding::Keys { negative, positive } => {
            held(input, positive) as i32 as f32 - held(input, negative) as i32 as f32
        }
        AxisBinding::MouseX { scale } => input.mouse_delta().x * scale,
        AxisBinding::MouseY { scale } => input.mouse_delta().y * scale,
        AxisBinding::Scroll { scale } => input.scroll() * scale,
    }
}

/// The actions that go over the wire, in a fixed order.
///
/// [`Intent`] is a bitfield and an array; both are indexed by position in this
/// set, so every machine in a session must build the same one. Building it
/// from a list in code — rather than from whatever the local bindings happen
/// to mention — is what makes that true.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActionSet {
    buttons: Vec<Action>,
    axes: Vec<Action>,
}

/// The most actions one [`Intent`] can carry.
pub const MAX_BUTTONS: usize = 64;

impl ActionSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a button action. Order is the wire order, so only ever append.
    ///
    /// # Panics
    ///
    /// If more than [`MAX_BUTTONS`] are added: the bitfield is a `u64`, and
    /// silently dropping the sixty-fifth action would be a desync found weeks
    /// later.
    pub fn button(&mut self, action: Action) -> &mut Self {
        assert!(
            self.buttons.len() < MAX_BUTTONS,
            "an intent carries at most {MAX_BUTTONS} button actions"
        );
        self.buttons.push(action);
        self
    }

    /// Add an axis action. Order is the wire order, so only ever append.
    pub fn axis(&mut self, action: Action) -> &mut Self {
        self.axes.push(action);
        self
    }

    /// Where an action sits in the wire order, if it is in the set.
    pub fn button_index(&self, action: Action) -> Option<usize> {
        self.buttons.iter().position(|bound| *bound == action)
    }

    /// Where an axis sits in the wire order, if it is in the set.
    pub fn axis_index(&self, action: Action) -> Option<usize> {
        self.axes.iter().position(|bound| *bound == action)
    }

    pub fn button_count(&self) -> usize {
        self.buttons.len()
    }

    pub fn axis_count(&self) -> usize {
        self.axes.len()
    }

    /// A number that differs when two machines disagree about the set.
    ///
    /// Worth exchanging during the handshake: two builds with different action
    /// lists produce intents that mean different things, and the failure is
    /// otherwise invisible — the other player simply does the wrong thing.
    pub fn fingerprint(&self) -> u32 {
        let mut hash: u32 = 0x811c_9dc5;
        for action in self.buttons.iter().chain(&self.axes) {
            for byte in action.0.to_le_bytes() {
                hash ^= byte as u32;
                hash = hash.wrapping_mul(0x0100_0193);
            }
        }
        hash
    }
}

/// What a player wanted, on one tick.
///
/// Buttons are a bitfield and axes are fixed point in thousandths, both
/// indexed by [`ActionSet`] order. Fixed point rather than `f32` because this
/// is an input to a simulation two machines must agree on: an axis that
/// arrives as `0.7000001` on one of them is a divergence, and quantizing at
/// the source means both sides step on exactly the number that was sent.
/// Thousandths are finer than a player can feel and match the milli-unit
/// convention the pathfinder already uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Intent {
    buttons: u64,
    axes: Vec<i16>,
}

/// The largest magnitude an axis can carry: `i16` in thousandths.
pub const AXIS_LIMIT: f32 = 32.767;

impl Intent {
    /// An empty intent shaped for `set`.
    pub fn for_set(set: &ActionSet) -> Self {
        Self {
            buttons: 0,
            axes: vec![0; set.axes.len()],
        }
    }

    /// Whether a button was held, by its index in the set.
    pub fn button(&self, index: usize) -> bool {
        index < MAX_BUTTONS && self.buttons & (1 << index) != 0
    }

    /// Set a button by index — for tests, replays and bots.
    pub fn set_button(&mut self, index: usize, down: bool) {
        if index >= MAX_BUTTONS {
            return;
        }
        if down {
            self.buttons |= 1 << index;
        } else {
            self.buttons &= !(1 << index);
        }
    }

    /// An axis value, by its index in the set.
    pub fn axis(&self, index: usize) -> f32 {
        self.axes.get(index).copied().unwrap_or(0) as f32 / 1000.0
    }

    /// Set an axis by index, quantizing and clamping as the wire would.
    pub fn set_axis(&mut self, index: usize, value: f32) {
        if index < self.axes.len() {
            self.axes[index] = quantize(value);
        }
    }

    /// Whether a named button was held, looked up through the set.
    pub fn down(&self, set: &ActionSet, action: Action) -> bool {
        set.button_index(action)
            .is_some_and(|index| self.button(index))
    }

    /// A named axis, looked up through the set.
    pub fn value(&self, set: &ActionSet, action: Action) -> f32 {
        set.axis_index(action).map_or(0.0, |index| self.axis(index))
    }

    /// Whether anything at all is being asked for.
    ///
    /// An idle player's intent need not be sent every tick; the receiver holds
    /// the last one until told otherwise.
    pub fn is_idle(&self) -> bool {
        self.buttons == 0 && self.axes.iter().all(|value| *value == 0)
    }
}

/// Clamp to the representable range and round to thousandths.
fn quantize(value: f32) -> i16 {
    if value.is_nan() {
        return 0;
    }
    let scaled = (value.clamp(-AXIS_LIMIT, AXIS_LIMIT) * 1000.0).round();
    scaled as i16
}

// ------------------------------------------------------------ serialization

impl Serialize for Intent {
    fn serialize(&self, writer: &mut Writer) {
        writer.varint(self.buttons);
        writer.varint(self.axes.len() as u64);
        for value in &self.axes {
            writer.signed(*value as i64);
        }
    }
}

impl Deserialize for Intent {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let buttons = reader.varint()?;
        let count = reader.varint()? as usize;
        // A length is not a promise: refuse to allocate on one that the rest
        // of the message cannot possibly back.
        if count > reader.remaining() {
            return Err(Error::LengthOutOfRange {
                position: reader.position(),
                length: count as u64,
                available: reader.remaining(),
            });
        }
        let mut axes = Vec::with_capacity(count);
        for _ in 0..count {
            let position = reader.position();
            let value = reader.signed()?;
            axes.push(i16::try_from(value).map_err(|_| Error::InvalidValue {
                position,
                what: "axis value out of range",
            })?);
        }
        Ok(Self { buttons, axes })
    }
}

impl Serialize for Action {
    fn serialize(&self, writer: &mut Writer) {
        writer.u32(self.0);
    }
}

impl Deserialize for Action {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        Ok(Self(reader.u32()?))
    }
}

/// Keys are written as stable codes, not as their position in the `Key` enum:
/// a saved keymap must survive a new key being added to the middle of it.
fn key_code(key: Key) -> u32 {
    match key {
        Key::A => 1,
        Key::B => 2,
        Key::C => 3,
        Key::D => 4,
        Key::E => 5,
        Key::F => 6,
        Key::G => 7,
        Key::H => 8,
        Key::I => 9,
        Key::J => 10,
        Key::K => 11,
        Key::L => 12,
        Key::M => 13,
        Key::N => 14,
        Key::O => 15,
        Key::P => 16,
        Key::Q => 17,
        Key::R => 18,
        Key::S => 19,
        Key::T => 20,
        Key::U => 21,
        Key::V => 22,
        Key::W => 23,
        Key::X => 24,
        Key::Y => 25,
        Key::Z => 26,
        Key::Num0 => 30,
        Key::Num1 => 31,
        Key::Num2 => 32,
        Key::Num3 => 33,
        Key::Num4 => 34,
        Key::Num5 => 35,
        Key::Num6 => 36,
        Key::Num7 => 37,
        Key::Num8 => 38,
        Key::Num9 => 39,
        Key::Escape => 40,
        Key::Space => 41,
        Key::Enter => 42,
        Key::Tab => 43,
        Key::Backspace => 44,
        Key::Left => 45,
        Key::Right => 46,
        Key::Up => 47,
        Key::Down => 48,
        Key::LeftShift => 49,
        Key::RightShift => 50,
        Key::LeftControl => 51,
        Key::RightControl => 52,
        Key::LeftAlt => 53,
        Key::RightAlt => 54,
        Key::LeftSuper => 55,
        Key::RightSuper => 56,
        Key::F1 => 60,
        Key::F2 => 61,
        Key::F3 => 62,
        Key::F4 => 63,
        Key::F5 => 64,
        Key::F6 => 65,
        Key::F7 => 66,
        Key::F8 => 67,
        Key::F9 => 68,
        Key::F10 => 69,
        Key::F11 => 70,
        Key::F12 => 71,
        // Unnamed keys keep their platform code, above everything named.
        Key::Unknown(code) => 0x1000_0000 | (code & 0x0fff_ffff),
    }
}

fn key_from_code(code: u32) -> Key {
    match code {
        1 => Key::A,
        2 => Key::B,
        3 => Key::C,
        4 => Key::D,
        5 => Key::E,
        6 => Key::F,
        7 => Key::G,
        8 => Key::H,
        9 => Key::I,
        10 => Key::J,
        11 => Key::K,
        12 => Key::L,
        13 => Key::M,
        14 => Key::N,
        15 => Key::O,
        16 => Key::P,
        17 => Key::Q,
        18 => Key::R,
        19 => Key::S,
        20 => Key::T,
        21 => Key::U,
        22 => Key::V,
        23 => Key::W,
        24 => Key::X,
        25 => Key::Y,
        26 => Key::Z,
        30 => Key::Num0,
        31 => Key::Num1,
        32 => Key::Num2,
        33 => Key::Num3,
        34 => Key::Num4,
        35 => Key::Num5,
        36 => Key::Num6,
        37 => Key::Num7,
        38 => Key::Num8,
        39 => Key::Num9,
        40 => Key::Escape,
        41 => Key::Space,
        42 => Key::Enter,
        43 => Key::Tab,
        44 => Key::Backspace,
        45 => Key::Left,
        46 => Key::Right,
        47 => Key::Up,
        48 => Key::Down,
        49 => Key::LeftShift,
        50 => Key::RightShift,
        51 => Key::LeftControl,
        52 => Key::RightControl,
        53 => Key::LeftAlt,
        54 => Key::RightAlt,
        55 => Key::LeftSuper,
        56 => Key::RightSuper,
        60 => Key::F1,
        61 => Key::F2,
        62 => Key::F3,
        63 => Key::F4,
        64 => Key::F5,
        65 => Key::F6,
        66 => Key::F7,
        67 => Key::F8,
        68 => Key::F9,
        69 => Key::F10,
        70 => Key::F11,
        71 => Key::F12,
        // A code this build does not know still round-trips, so loading a
        // keymap saved by a newer build and saving it again loses nothing.
        other => Key::Unknown(other & 0x0fff_ffff),
    }
}

impl Serialize for Binding {
    fn serialize(&self, writer: &mut Writer) {
        match self {
            Binding::Key(key) => {
                writer.u8(0);
                writer.varint(key_code(*key) as u64);
            }
            Binding::Mouse(button) => {
                writer.u8(1);
                let code = match button {
                    MouseButton::Left => 0u8,
                    MouseButton::Middle => 1,
                    MouseButton::Right => 2,
                    MouseButton::Other(other) => 3u8.saturating_add(*other),
                };
                writer.u8(code);
            }
        }
    }
}

impl Deserialize for Binding {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let position = reader.position();
        match reader.u8()? {
            0 => Ok(Binding::Key(key_from_code(reader.varint()? as u32))),
            1 => Ok(Binding::Mouse(match reader.u8()? {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                other => MouseButton::Other(other - 3),
            })),
            _ => Err(Error::InvalidValue {
                position,
                what: "binding kind",
            }),
        }
    }
}

impl Serialize for AxisBinding {
    fn serialize(&self, writer: &mut Writer) {
        match self {
            AxisBinding::Keys { negative, positive } => {
                writer.u8(0);
                writer.write(negative);
                writer.write(positive);
            }
            AxisBinding::MouseX { scale } => {
                writer.u8(1);
                writer.f32(*scale);
            }
            AxisBinding::MouseY { scale } => {
                writer.u8(2);
                writer.f32(*scale);
            }
            AxisBinding::Scroll { scale } => {
                writer.u8(3);
                writer.f32(*scale);
            }
        }
    }
}

impl Deserialize for AxisBinding {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let position = reader.position();
        match reader.u8()? {
            0 => Ok(AxisBinding::Keys {
                negative: reader.read()?,
                positive: reader.read()?,
            }),
            1 => Ok(AxisBinding::MouseX {
                scale: reader.f32()?,
            }),
            2 => Ok(AxisBinding::MouseY {
                scale: reader.f32()?,
            }),
            3 => Ok(AxisBinding::Scroll {
                scale: reader.f32()?,
            }),
            _ => Err(Error::InvalidValue {
                position,
                what: "axis binding kind",
            }),
        }
    }
}

impl Serialize for Bindings {
    fn serialize(&self, writer: &mut Writer) {
        writer.varint(self.buttons.len() as u64);
        for (action, binding) in &self.buttons {
            writer.write(action);
            writer.write(binding);
        }
        writer.varint(self.axes.len() as u64);
        for (action, binding) in &self.axes {
            writer.write(action);
            writer.write(binding);
        }
    }
}

impl Deserialize for Bindings {
    fn deserialize(reader: &mut Reader) -> Result<Self> {
        let mut bindings = Bindings::new();
        let count = reader.varint()? as usize;
        if count > reader.remaining() {
            return Err(Error::LengthOutOfRange {
                position: reader.position(),
                length: count as u64,
                available: reader.remaining(),
            });
        }
        for _ in 0..count {
            let action: Action = reader.read()?;
            let binding: Binding = reader.read()?;
            bindings.bind(action, binding);
        }
        let count = reader.varint()? as usize;
        if count > reader.remaining() {
            return Err(Error::LengthOutOfRange {
                position: reader.position(),
                length: count as u64,
                available: reader.remaining(),
            });
        }
        for _ in 0..count {
            let action: Action = reader.read()?;
            let binding: AxisBinding = reader.read()?;
            bindings.bind_axis(action, binding);
        }
        Ok(bindings)
    }
}
