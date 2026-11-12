//! What a gamepad is and what a game can ask of it beyond its buttons:
//! its kind (for the right glyphs), battery and abilities ([`PadInfo`]),
//! its motion sensors ([`PadMotion`]), and the feedback a game sends back —
//! the light bar, the player LEDs, rumble and short haptics, a DualSense's
//! adaptive triggers ([`PadFeedback`]).
//!
//! All of it is the same on every platform and every backend: a game asks
//! [`PadFeedback`] and the shell's backend does what the pad can — Apple's
//! GameController on macOS and iOS does all of it; gilrs (Windows, Linux,
//! the browser) none of the output yet, so there it is a no-op. A game asks
//! what a pad can do with [`PadInfo::caps`]. docs/gamepads.md.

use crate::input::PadButton;
use serde::{Deserialize, Serialize};

/// What kind of pad is in a slot: the glyphs it wants, by family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PadKind {
    /// Not told yet (a script's pad), or a pad the backend cannot name.
    #[default]
    Generic,
    DualSense,
    /// DualSense Edge: a DualSense with back paddles.
    DualSenseEdge,
    DualShock4,
    Xbox,
    /// Nintendo Switch Pro Controller.
    SwitchPro,
    /// One or two Joy-Cons.
    JoyCon,
    /// An MFi pad (Backbone, SteelSeries Nimbus…): Xbox-like labels.
    Mfi,
    /// The pad drawn on a touch screen (`TouchLayout::gamepad`) or the
    /// page's: ✕ ○ □ △, as it draws them.
    Screen,
}

/// The family of glyphs a pad's buttons are drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PadGlyphs {
    /// ✕ ○ □ △, L1 R1 L2 R2, Options, Create.
    PlayStation,
    /// A B X Y, LB RB LT RT, Menu, View.
    Xbox,
    /// B A Y X by position (South is B), L R ZL ZR, + −.
    Nintendo,
    /// Positions only: a game's own neutral art.
    Generic,
}

impl PadKind {
    /// The glyph family to draw this pad's buttons with.
    pub fn glyphs(self) -> PadGlyphs {
        match self {
            PadKind::DualSense | PadKind::DualSenseEdge | PadKind::DualShock4 | PadKind::Screen => PadGlyphs::PlayStation,
            PadKind::Xbox | PadKind::Mfi => PadGlyphs::Xbox,
            PadKind::SwitchPro | PadKind::JoyCon => PadGlyphs::Nintendo,
            PadKind::Generic => PadGlyphs::Generic,
        }
    }

    /// Whether its face buttons are labelled the Nintendo way (A on the
    /// right): positions still say South for the bottom one.
    pub fn nintendo(self) -> bool {
        self.glyphs() == PadGlyphs::Nintendo
    }

    /// A kind from a name the platform gives: GameController's product
    /// category ("DualSense", "Xbox One", "Switch Pro Controller"…), or a
    /// device's name (gilrs, the browser's `Gamepad.id`).
    pub fn from_name(name: &str) -> PadKind {
        let n = name.to_ascii_lowercase();
        if n.contains("dualsense edge") || n.contains("0df2") {
            PadKind::DualSenseEdge
        } else if n.contains("dualsense") || n.contains("0ce6") {
            PadKind::DualSense
        } else if n.contains("dualshock") || n.contains("ps4") || n.contains("05c4") || n.contains("09cc") {
            PadKind::DualShock4
        } else if n.contains("xbox") || n.contains("xinput") || n.contains("045e") {
            PadKind::Xbox
        } else if n.contains("joy-con") || n.contains("joycon") {
            PadKind::JoyCon
        } else if n.contains("pro controller") || n.contains("switch") || n.contains("057e") {
            PadKind::SwitchPro
        } else if n.contains("mfi") {
            PadKind::Mfi
        } else {
            PadKind::Generic
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PadKind::Generic => "pad",
            PadKind::DualSense => "DualSense",
            PadKind::DualSenseEdge => "DualSense Edge",
            PadKind::DualShock4 => "DualShock 4",
            PadKind::Xbox => "Xbox",
            PadKind::SwitchPro => "Switch Pro",
            PadKind::JoyCon => "Joy-Con",
            PadKind::Mfi => "MFi",
            PadKind::Screen => "screen",
        }
    }
}

/// What a pad can do beyond buttons and sticks: what the backend found on
/// it. A game asks before offering a setting (no light-bar row for a pad
/// without one); asking for what a pad lacks is a no-op either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PadCaps {
    /// Triggers report how far they are pulled, not just pressed.
    pub analog_triggers: bool,
    /// A coloured light (DualSense, DualShock 4): [`PadFeedback::set_light`].
    pub light: bool,
    /// Player LEDs or a player number the pad shows (`playerIndex`).
    pub player_leds: bool,
    /// Rumble or haptics in the handles: [`PadFeedback::rumble`].
    pub rumble: bool,
    /// Haptics in the triggers themselves (Xbox impulse triggers).
    pub trigger_rumble: bool,
    /// Resistance in the triggers (DualSense): [`PadFeedback::set_trigger_effect`].
    pub adaptive_triggers: bool,
    /// A touchpad: [`PadButton::Touchpad`], `Finger1`/`Finger2` and their axes.
    pub touchpad: bool,
    /// Gyro and accelerometer: [`PadFeedback::set_motion`], then
    /// [`crate::input::PadState::motion`].
    pub motion: bool,
    /// The platform reports a battery.
    pub battery: bool,
    /// The microphone's mute button as [`PadButton::Mute`] (where the
    /// platform hands it to games at all).
    pub mute_button: bool,
    /// Back paddles: [`PadButton::Paddle1`]…`Paddle4`.
    pub paddles: u8,
}

/// Whether the pad charges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BatteryState {
    #[default]
    Unknown,
    Discharging,
    Charging,
    Full,
}

/// A pad's battery: 0..1 and whether it charges.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Battery {
    pub level: f32,
    pub state: BatteryState,
}

/// What is in a slot: its kind, its name, the backend reading it, what it
/// can do and its battery. Sent as [`crate::input::InputEvent::PadInfo`]
/// when a pad comes and when its battery changes; read with
/// [`crate::input::PadState::info`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PadInfo {
    pub kind: PadKind,
    /// The platform's name for it ("DualSense Wireless Controller").
    pub name: String,
    /// What reads it: "gamecontroller", "gilrs", "web", "screen", "script".
    pub backend: String,
    pub caps: PadCaps,
    pub battery: Option<Battery>,
}

impl PadInfo {
    /// A pad of a kind, named after it, read by `backend`.
    pub fn new(kind: PadKind, backend: &str) -> Self {
        Self { kind, name: kind.label().to_string(), backend: backend.to_string(), ..Self::default() }
    }

    /// One line for a person: "DualSense (gamecontroller) 80% charging".
    pub fn summary(&self) -> String {
        let mut s = format!("{} [{}] via {}", self.name, self.kind.label(), self.backend);
        if let Some(b) = self.battery {
            s += &format!(", battery {:.0}%", b.level * 100.0);
            match b.state {
                BatteryState::Charging => s += " charging",
                BatteryState::Full => s += " full",
                _ => {}
            }
        }
        s
    }
}

/// A pad's motion sensors, in the pad's frame: what GameController's
/// `GCMotion` reports. Off by default (they cost battery): switch them on
/// per pad with [`PadFeedback::set_motion`].
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PadMotion {
    /// Gravity, in g.
    pub gravity: [f32; 3],
    /// Acceleration the player gives it, gravity taken out, in g.
    pub acceleration: [f32; 3],
    /// Radians per second about x, y, z.
    pub rotation_rate: [f32; 3],
    /// Orientation as a quaternion (x, y, z, w), where the pad has one.
    pub attitude: Option<[f32; 4]>,
}

/// The digital press of an analog trigger: down at `press`, up again only
/// below `release` — so a finger resting at the threshold does not chatter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TriggerThreshold {
    pub press: f32,
    pub release: f32,
}

impl Default for TriggerThreshold {
    /// Half way down, back up below 40 %.
    fn default() -> Self {
        Self { press: 0.5, release: 0.4 }
    }
}

impl TriggerThreshold {
    /// Whether the trigger counts as pressed at `value`, given whether it
    /// did before.
    pub fn held(&self, was: bool, value: f32) -> bool {
        if was {
            value >= self.release.min(self.press)
        } else {
            value >= self.press
        }
    }
}

/// Which trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerSide {
    Left,
    Right,
}

/// A DualSense adaptive trigger's resistance (`GCDualSenseAdaptiveTrigger`).
/// Positions and strengths are 0..1 along the trigger's travel.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TriggerEffect {
    /// Free travel.
    #[default]
    Off,
    /// Even resistance from `start` to the end.
    Feedback { start: f32, strength: f32 },
    /// A bow or a gun: resistance from `start` that gives way at `end`.
    Weapon { start: f32, end: f32, strength: f32 },
    /// The trigger buzzes from `start`: `amplitude` 0..1, `frequency` 0..1
    /// of the pad's range.
    Vibration { start: f32, amplitude: f32, frequency: f32 },
    /// Resistance rising (or falling) from `start_strength` at `start` to
    /// `end_strength` at `end`.
    Slope { start: f32, end: f32, start_strength: f32, end_strength: f32 },
}

impl TriggerEffect {
    /// A soft click a short way in: a detent (a radial menu opening).
    pub fn detent() -> Self {
        TriggerEffect::Weapon { start: 0.15, end: 0.35, strength: 0.35 }
    }

    /// Clamped to the ranges the pad takes, NaN as zero.
    pub fn clamped(self) -> Self {
        let c = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            TriggerEffect::Off => TriggerEffect::Off,
            TriggerEffect::Feedback { start, strength } => TriggerEffect::Feedback { start: c(start), strength: c(strength) },
            TriggerEffect::Weapon { start, end, strength } => {
                let (start, end) = (c(start), c(end));
                TriggerEffect::Weapon { start: start.min(end), end: end.max(start), strength: c(strength) }
            }
            TriggerEffect::Vibration { start, amplitude, frequency } => TriggerEffect::Vibration { start: c(start), amplitude: c(amplitude), frequency: c(frequency) },
            TriggerEffect::Slope { start, end, start_strength, end_strength } => {
                let (start, end) = (c(start), c(end));
                TriggerEffect::Slope { start: start.min(end), end: end.max(start), start_strength: c(start_strength), end_strength: c(end_strength) }
            }
        }
    }
}

/// Where on the pad a haptic plays (`GCHapticsLocality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum HapticPlace {
    /// The pad's own choice: its handles.
    #[default]
    Handles,
    LeftHandle,
    RightHandle,
    /// Both triggers (Xbox impulse triggers).
    Triggers,
    LeftTrigger,
    RightTrigger,
}

/// A short haptic: a tap (`seconds` 0) or a buzz, `strength` and
/// `sharpness` 0..1 (sharp is crisp and high, dull is a thud).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Haptic {
    pub strength: f32,
    pub sharpness: f32,
    pub seconds: f32,
    pub place: HapticPlace,
}

impl Haptic {
    /// A single tap.
    pub fn tap(strength: f32) -> Self {
        Self { strength, sharpness: 0.6, seconds: 0.0, place: HapticPlace::Handles }
    }
    /// A buzz of a length.
    pub fn buzz(strength: f32, sharpness: f32, seconds: f32) -> Self {
        Self { strength, sharpness, seconds, place: HapticPlace::Handles }
    }
    pub fn at(mut self, place: HapticPlace) -> Self {
        self.place = place;
        self
    }
}

/// Classic two-motor rumble: `low` the heavy left motor, `high` the light
/// right one, 0..1, for `seconds`. A rumble replaces the one before it on
/// that pad; `rumble(slot, 0, 0, 0)` stops it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rumble {
    pub low: f32,
    pub high: f32,
    pub seconds: f32,
}

/// One thing sent to a pad once (not a state that holds).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PadPulse {
    Rumble(Rumble),
    Haptic(Haptic),
}

/// The players' colours by slot, for the light bar until a game says
/// otherwise: orange, blue, green, pink, then round again.
pub const PLAYER_COLOURS: [[f32; 3]; 4] = [[1.0, 0.45, 0.05], [0.1, 0.35, 1.0], [0.1, 0.8, 0.25], [0.9, 0.2, 0.6]];

/// What a game asks of its pads' outputs, by player slot: a state that
/// holds (light colour, player LEDs, trigger resistance, motion sensors) and
/// pulses sent once (rumble, haptics). The shell's backend applies it each
/// frame — only what changed, the light at most 30 times a second — so a
/// game may set a pulsing colour every frame. Slots with no pad, pads
/// without the feature and backends without output ignore it.
///
/// A game's tests can hold one and read back what was asked
/// ([`PadFeedback::light`], [`PadFeedback::pulses`]).
#[derive(Debug, Clone, PartialEq)]
pub struct PadFeedback {
    lights: Vec<Option<[f32; 3]>>,
    colours: Vec<[f32; 3]>,
    lights_on: bool,
    leds: Vec<Option<Option<u8>>>,
    triggers: Vec<[Option<TriggerEffect>; 2]>,
    triggers_on: bool,
    motion: Vec<bool>,
    vibration_on: bool,
    pulses: Vec<(usize, PadPulse)>,
}

impl Default for PadFeedback {
    fn default() -> Self {
        Self {
            lights: Vec::new(),
            colours: PLAYER_COLOURS.to_vec(),
            lights_on: true,
            leds: Vec::new(),
            triggers: Vec::new(),
            triggers_on: true,
            motion: Vec::new(),
            vibration_on: true,
            pulses: Vec::new(),
        }
    }
}

fn at<T: Clone + Default>(v: &mut Vec<T>, slot: usize) -> &mut T {
    if v.len() <= slot {
        v.resize(slot + 1, T::default());
    }
    &mut v[slot]
}

impl PadFeedback {
    pub fn new() -> Self {
        Self::default()
    }

    /// The light of a slot's pad, 0..1 RGB: held until set again or
    /// [`PadFeedback::clear_light`].
    pub fn set_light(&mut self, slot: usize, rgb: [f32; 3]) {
        *at(&mut self.lights, slot) = Some(rgb.map(|c| if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 }));
    }

    /// Back to the slot's player colour.
    pub fn clear_light(&mut self, slot: usize) {
        if let Some(l) = self.lights.get_mut(slot) {
            *l = None;
        }
    }

    /// The players' colours by slot: what a light shows when the game set
    /// none (by default [`PLAYER_COLOURS`]).
    pub fn set_player_colours(&mut self, colours: &[[f32; 3]]) {
        if !colours.is_empty() {
            self.colours = colours.to_vec();
        }
    }

    /// Lights on (the colours) or off (dark) — a player's setting.
    pub fn set_lights_enabled(&mut self, on: bool) {
        self.lights_on = on;
    }

    pub fn lights_enabled(&self) -> bool {
        self.lights_on
    }

    /// The slot's player colour.
    pub fn player_colour(&self, slot: usize) -> [f32; 3] {
        self.colours[slot % self.colours.len()]
    }

    /// What the slot's light shows now: the game's colour, else its player
    /// colour; dark with the lights off.
    pub fn light(&self, slot: usize) -> [f32; 3] {
        if !self.lights_on {
            return [0.0; 3];
        }
        self.lights.get(slot).copied().flatten().unwrap_or_else(|| self.player_colour(slot))
    }

    /// The number the pad shows on its player LEDs: `Some(0)` is player 1;
    /// `None` all off. Without this, the slot's own number.
    pub fn set_player_index(&mut self, slot: usize, index: Option<u8>) {
        *at(&mut self.leds, slot) = Some(index);
    }

    /// What the slot's player LEDs show.
    pub fn player_index(&self, slot: usize) -> Option<u8> {
        self.leds.get(slot).copied().flatten().unwrap_or(u8::try_from(slot).ok())
    }

    /// A DualSense trigger's resistance, held until set again.
    pub fn set_trigger_effect(&mut self, slot: usize, side: TriggerSide, effect: TriggerEffect) {
        at(&mut self.triggers, slot)[side as usize] = Some(effect.clamped());
    }

    /// What the trigger is asked to do now (`Off` with trigger effects
    /// switched off).
    pub fn trigger_effect(&self, slot: usize, side: TriggerSide) -> TriggerEffect {
        if !self.triggers_on {
            return TriggerEffect::Off;
        }
        self.triggers.get(slot).and_then(|t| t[side as usize]).unwrap_or_default()
    }

    /// Trigger effects on or off — a player's setting.
    pub fn set_triggers_enabled(&mut self, on: bool) {
        self.triggers_on = on;
    }

    pub fn triggers_enabled(&self) -> bool {
        self.triggers_on
    }

    /// The pad's gyro and accelerometer on or off (off by default: they
    /// cost battery).
    pub fn set_motion(&mut self, slot: usize, on: bool) {
        *at(&mut self.motion, slot) = on;
    }

    pub fn motion(&self, slot: usize) -> bool {
        self.motion.get(slot).copied().unwrap_or(false)
    }

    /// Rumble: `low` the heavy motor, `high` the light one, 0..1, for
    /// `seconds`. Replaces the slot's rumble before it.
    pub fn rumble(&mut self, slot: usize, low: f32, high: f32, seconds: f32) {
        if self.vibration_on {
            let c = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            self.pulses.push((slot, PadPulse::Rumble(Rumble { low: c(low), high: c(high), seconds: seconds.clamp(0.0, 10.0) })));
        }
    }

    /// A short haptic on the slot's pad.
    pub fn haptic(&mut self, slot: usize, haptic: Haptic) {
        if self.vibration_on {
            self.pulses.push((slot, PadPulse::Haptic(haptic)));
        }
    }

    /// Rumble and haptics on or off — a player's setting. Off stops what
    /// plays.
    pub fn set_vibration_enabled(&mut self, on: bool) {
        if self.vibration_on && !on {
            self.pulses.clear();
            for slot in 0..self.lights.len().max(self.leds.len()).max(4) {
                self.pulses.push((slot, PadPulse::Rumble(Rumble { low: 0.0, high: 0.0, seconds: 0.0 })));
            }
        }
        self.vibration_on = on;
    }

    pub fn vibration_enabled(&self) -> bool {
        self.vibration_on
    }

    /// Rumbles and haptics asked since the backend last took them.
    pub fn pulses(&self) -> &[(usize, PadPulse)] {
        &self.pulses
    }

    /// The backend takes the pulses to send.
    pub fn take_pulses(&mut self) -> Vec<(usize, PadPulse)> {
        std::mem::take(&mut self.pulses)
    }
}

/// A colour breathing between `rgb` and `rgb × (1 − depth)` once every
/// `period` seconds, at time `t`: a "your turn" pulse to set each frame.
pub fn pulse(rgb: [f32; 3], t: f32, period: f32, depth: f32) -> [f32; 3] {
    let phase = (t / period.max(1e-3)) * std::f32::consts::TAU;
    let k = 1.0 - depth.clamp(0.0, 1.0) * (0.5 - 0.5 * phase.cos());
    rgb.map(|c| c * k)
}

/// `to` fading back into `from` over `seconds` from `since` — a flash (red
/// on a hit) at time `t`.
pub fn flash(from: [f32; 3], to: [f32; 3], t: f32, since: f32, seconds: f32) -> [f32; 3] {
    let k = 1.0 - ((t - since) / seconds.max(1e-3)).clamp(0.0, 1.0);
    let k = if t < since { 0.0 } else { k * k };
    [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * k)
}

/// Buttons a pad may have beyond the common sixteen, for listings.
pub const EXTRA_BUTTONS: [PadButton; 10] = [
    PadButton::Home,
    PadButton::Touchpad,
    PadButton::Mute,
    PadButton::Share,
    PadButton::Paddle1,
    PadButton::Paddle2,
    PadButton::Paddle3,
    PadButton::Paddle4,
    PadButton::Finger1,
    PadButton::Finger2,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_platforms_names() {
        assert_eq!(PadKind::from_name("DualSense"), PadKind::DualSense);
        assert_eq!(PadKind::from_name("DualSense Wireless Controller"), PadKind::DualSense);
        assert_eq!(PadKind::from_name("Xbox One"), PadKind::Xbox);
        assert_eq!(PadKind::from_name("Switch Pro Controller"), PadKind::SwitchPro);
        assert_eq!(PadKind::from_name("054c-0ce6-Wireless Controller"), PadKind::DualSense);
        assert_eq!(PadKind::from_name("Some USB pad"), PadKind::Generic);
        assert_eq!(PadKind::SwitchPro.glyphs(), PadGlyphs::Nintendo);
        assert_eq!(PadKind::DualSense.glyphs(), PadGlyphs::PlayStation);
    }

    #[test]
    fn a_trigger_presses_at_its_threshold_and_lets_go_below_the_lower_one() {
        let t = TriggerThreshold::default();
        assert!(!t.held(false, 0.49));
        assert!(t.held(false, 0.5));
        assert!(t.held(true, 0.45), "inside the hysteresis it stays down");
        assert!(!t.held(true, 0.39));
    }

    #[test]
    fn a_light_is_the_players_colour_until_the_game_says_otherwise() {
        let mut f = PadFeedback::new();
        f.set_player_colours(&[[1.0, 0.5, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(f.light(1), [0.0, 0.0, 1.0]);
        f.set_light(1, [2.0, 0.0, f32::NAN]);
        assert_eq!(f.light(1), [1.0, 0.0, 0.0], "clamped");
        f.clear_light(1);
        assert_eq!(f.light(1), [0.0, 0.0, 1.0]);
        assert_eq!(f.light(2), [1.0, 0.5, 0.0], "colours go round");
        f.set_lights_enabled(false);
        assert_eq!(f.light(0), [0.0; 3]);
        assert_eq!(f.player_index(1), Some(1));
        f.set_player_index(1, None);
        assert_eq!(f.player_index(1), None);
    }

    #[test]
    fn vibration_off_drops_pulses_and_stops_rumble() {
        let mut f = PadFeedback::new();
        f.rumble(0, 0.5, 0.2, 0.3);
        assert_eq!(f.pulses().len(), 1);
        f.set_vibration_enabled(false);
        f.haptic(0, Haptic::tap(1.0));
        assert!(f.pulses().iter().all(|(_, p)| matches!(p, PadPulse::Rumble(r) if r.low == 0.0 && r.high == 0.0)));
        f.set_triggers_enabled(false);
        f.set_trigger_effect(0, TriggerSide::Right, TriggerEffect::detent());
        assert_eq!(f.trigger_effect(0, TriggerSide::Right), TriggerEffect::Off);
        f.set_triggers_enabled(true);
        assert_eq!(f.trigger_effect(0, TriggerSide::Right), TriggerEffect::detent());
    }

    #[test]
    fn pulse_and_flash_stay_in_range() {
        let c = [1.0, 0.5, 0.0];
        assert_eq!(pulse(c, 0.0, 1.0, 0.6), c);
        let low = pulse(c, 0.5, 1.0, 0.6);
        assert!((low[0] - 0.4).abs() < 1e-5);
        assert_eq!(flash(c, [1.0, 0.0, 0.0], 1.0, 1.0, 0.3), [1.0, 0.0, 0.0]);
        assert_eq!(flash(c, [1.0, 0.0, 0.0], 2.0, 1.0, 0.3), c);
    }
}
