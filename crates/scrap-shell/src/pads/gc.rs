//! Pads the way Apple's GameController framework sees them, and all the
//! engine's rules over them — independent of the framework, so they are
//! tested on any machine with a fake ([`Controllers`]): the slot each
//! controller takes (present at start, hot-plugged, gone and back), its
//! buttons by position (Cross/A/B — whichever is at the bottom is South),
//! triggers as analog values and as buttons past a threshold, the window
//! in the background, and the game's [`PadFeedback`] sent only as it
//! changes (the light at most 30 times a second).
//!
//! The framework's side — reading `GCController`s into [`GcState`] and
//! writing their light, LEDs, haptics and triggers — is `apple`.
//!
//! Polled once a frame, on the main thread, from the window's loop: the
//! framework updates its elements on the main queue, which that loop
//! drains. A press and release both inside one frame is not seen (as
//! with any polling); frames are short next to a press.

use super::{Backend, Listed, PadSource};
use scrap_core::input::{
    Haptic, InputEvent, PadAxis, PadButton, PadFeedback, PadInfo, PadKind, PadMotion, PadPulse, PadSlots, Rumble, TriggerEffect,
    TriggerSide, TriggerThreshold,
};
use std::collections::HashSet;

/// One controller's elements this frame, by the framework's names
/// (`GCExtendedGamepad`, `GCDualSenseGamepad`, `GCXboxGamepad`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GcState {
    pub a: bool,
    pub b: bool,
    pub x: bool,
    pub y: bool,
    /// `buttonA`'s label ("A", "B", "Cross"): which way a Nintendo pad's
    /// face buttons were handed over.
    pub a_label: Option<String>,
    pub left_shoulder: bool,
    pub right_shoulder: bool,
    /// Triggers, 0..1.
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub left_thumb: bool,
    pub right_thumb: bool,
    /// Up, down, left, right.
    pub dpad: [bool; 4],
    /// Sticks, −1..1, up and right positive.
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    /// `buttonMenu`: Options on a DualSense, ≡ on an Xbox pad — Start.
    pub menu: bool,
    /// `buttonOptions`: Create on a DualSense, View on an Xbox pad — Select.
    pub options: bool,
    pub home: bool,
    /// Xbox Series' Share.
    pub share: bool,
    pub touchpad_button: bool,
    /// The microphone button, where the profile names one.
    pub mute: bool,
    pub paddles: [bool; 4],
    /// Touchpad fingers, −1..1 each way, while down.
    pub fingers: [Option<[f32; 2]>; 2],
}

/// A controller as the platform shows it this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    /// The platform's controller, as long as it is connected.
    pub id: u64,
    pub info: PadInfo,
    pub state: GcState,
    /// Its sensors, while switched on.
    pub motion: Option<PadMotion>,
}

/// The framework, as [`GcPads`] uses it: what is connected and what it
/// says, and the outputs. `apple::AppleControllers` is the real one;
/// tests have a fake that records what was asked.
pub trait Controllers {
    /// Every controller connected now, in the platform's order.
    fn scan(&mut self, out: &mut Vec<Seen>);
    fn set_light(&mut self, id: u64, rgb: [f32; 3]);
    /// Player LEDs: `Some(0)` player 1; `None` off.
    fn set_player_index(&mut self, id: u64, index: Option<u8>);
    fn set_trigger(&mut self, id: u64, side: TriggerSide, effect: TriggerEffect);
    fn set_motion(&mut self, id: u64, on: bool);
    fn rumble(&mut self, id: u64, rumble: Rumble);
    fn haptic(&mut self, id: u64, haptic: Haptic);
    /// Stop every haptic (the app went to the background).
    fn stop_haptics(&mut self);
    /// A controller went: drop what was kept for it.
    fn forget(&mut self, _id: u64) {}
    /// Every element it names (for `scrap pads`).
    fn elements(&self, _id: u64) -> Vec<String> {
        Vec::new()
    }
    fn pump(&mut self, _seconds: f64) {}
}

/// The engine's buttons and axes from a controller's elements: the face
/// buttons by position, triggers and fingers as axes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Raw {
    pub buttons: HashSet<PadButton>,
    pub axes: Vec<(PadAxis, f32)>,
}

impl Raw {
    fn axis(&self, axis: PadAxis) -> f32 {
        self.axes.iter().find(|(a, _)| *a == axis).map_or(0.0, |(_, v)| *v)
    }
}

/// Every axis a controller reports, in a fixed order.
const AXES: [PadAxis; 10] = [
    PadAxis::LeftX,
    PadAxis::LeftY,
    PadAxis::RightX,
    PadAxis::RightY,
    PadAxis::LeftTrigger,
    PadAxis::RightTrigger,
    PadAxis::Finger1X,
    PadAxis::Finger1Y,
    PadAxis::Finger2X,
    PadAxis::Finger2Y,
];

/// A controller's elements as the engine's buttons, by position: South is
/// the bottom face button on every pad — Cross, A on Xbox, B on Nintendo.
/// GameController hands a Nintendo pad over by position (its `buttonA` is
/// the bottom one, labelled B); if it ever hands it over by label instead
/// (`buttonA` labelled "A", on the right), the four are turned back.
pub fn by_position(kind: PadKind, s: &GcState) -> Raw {
    let labelled = kind.nintendo() && s.a_label.as_deref().is_some_and(|l| l.eq_ignore_ascii_case("a"));
    let (south, east, west, north) = if labelled { (s.b, s.a, s.y, s.x) } else { (s.a, s.b, s.x, s.y) };
    let mut buttons = HashSet::new();
    let mut put = |on: bool, b: PadButton| {
        if on {
            buttons.insert(b);
        }
    };
    put(south, PadButton::South);
    put(east, PadButton::East);
    put(west, PadButton::West);
    put(north, PadButton::North);
    put(s.left_shoulder, PadButton::LeftBumper);
    put(s.right_shoulder, PadButton::RightBumper);
    put(s.left_thumb, PadButton::LeftStick);
    put(s.right_thumb, PadButton::RightStick);
    put(s.dpad[0], PadButton::DPadUp);
    put(s.dpad[1], PadButton::DPadDown);
    put(s.dpad[2], PadButton::DPadLeft);
    put(s.dpad[3], PadButton::DPadRight);
    put(s.menu, PadButton::Start);
    put(s.options, PadButton::Select);
    put(s.home, PadButton::Home);
    put(s.share, PadButton::Share);
    put(s.touchpad_button, PadButton::Touchpad);
    put(s.mute, PadButton::Mute);
    for (on, b) in s.paddles.iter().zip([PadButton::Paddle1, PadButton::Paddle2, PadButton::Paddle3, PadButton::Paddle4]) {
        put(*on, b);
    }
    put(s.fingers[0].is_some(), PadButton::Finger1);
    put(s.fingers[1].is_some(), PadButton::Finger2);
    let finger = |i: usize, k: usize| s.fingers[i].map_or(0.0, |f| f[k]);
    let clean = |v: f32| if v.is_finite() { v } else { 0.0 };
    let axes = vec![
        (PadAxis::LeftX, clean(s.left_stick[0])),
        (PadAxis::LeftY, clean(s.left_stick[1])),
        (PadAxis::RightX, clean(s.right_stick[0])),
        (PadAxis::RightY, clean(s.right_stick[1])),
        (PadAxis::LeftTrigger, clean(s.left_trigger)),
        (PadAxis::RightTrigger, clean(s.right_trigger)),
        (PadAxis::Finger1X, finger(0, 0)),
        (PadAxis::Finger1Y, finger(0, 1)),
        (PadAxis::Finger2X, finger(1, 0)),
        (PadAxis::Finger2Y, finger(1, 1)),
    ];
    Raw { buttons, axes }
}

/// What was last sent to a controller's outputs.
#[derive(Debug, Clone, Default)]
struct Sent {
    light: Option<[u8; 3]>,
    light_at: f64,
    player: Option<Option<u8>>,
    triggers: [Option<TriggerEffect>; 2],
    motion: Option<bool>,
    /// The rumble playing, and until when.
    rumble: Option<(Rumble, f64)>,
}

/// A controller the backend knows.
struct Known {
    id: u64,
    info: PadInfo,
    raw: Raw,
    /// The triggers' buttons, by [`TriggerThreshold`].
    triggers: [bool; 2],
    motion: Option<PadMotion>,
    sent: Sent,
}

/// The light at most this often (seconds): a game may set it each frame.
pub const LIGHT_EVERY: f64 = 1.0 / 30.0;

/// Pads read through GameController ([`Controllers`]), by player slot.
pub struct GcPads<C: Controllers> {
    pub controllers: C,
    known: Vec<Known>,
    thresholds: TriggerThreshold,
    seen: Vec<Seen>,
}

impl<C: Controllers> GcPads<C> {
    pub fn new(controllers: C) -> Self {
        Self { controllers, known: Vec::new(), thresholds: TriggerThreshold::default(), seen: Vec::new() }
    }
}

/// The battery to the nearest 5 %: a new [`InputEvent::PadInfo`] when it
/// moves that far, not at every percent.
fn coarse(info: &PadInfo) -> PadInfo {
    let mut info = info.clone();
    if let Some(b) = info.battery.as_mut() {
        b.level = (b.level * 20.0).round() / 20.0;
    }
    info
}

impl<C: Controllers> PadSource for GcPads<C> {
    fn backend(&self) -> Backend {
        Backend::GameController
    }

    fn poll(&mut self, slots: &mut PadSlots, live: bool, out: &mut Vec<InputEvent>) -> bool {
        let mut seen = std::mem::take(&mut self.seen);
        seen.clear();
        self.controllers.scan(&mut seen);
        // Gone: the slot waits for it.
        let gone: Vec<u64> = self.known.iter().map(|k| k.id).filter(|id| !seen.iter().any(|s| s.id == *id)).collect();
        for id in gone {
            self.known.retain(|k| k.id != id);
            self.controllers.forget(id);
            if let Some(slot) = slots.disconnect(id) {
                out.push(InputEvent::PadDisconnected(slot));
            }
        }
        let mut pressed = false;
        for s in &seen {
            let info = coarse(&s.info);
            let i = match self.known.iter().position(|k| k.id == s.id) {
                Some(i) => {
                    if self.known[i].info != info {
                        if let Some(slot) = slots.slot(s.id) {
                            out.push(InputEvent::PadInfo { pad: slot, info: info.clone() });
                        }
                        self.known[i].info = info;
                    }
                    i
                }
                // New — present at start or just plugged in: a slot, and
                // what it is.
                None => {
                    let slot = slots.connect(s.id);
                    out.push(InputEvent::PadConnected(slot));
                    out.push(InputEvent::PadInfo { pad: slot, info: info.clone() });
                    self.known.push(Known { id: s.id, info, raw: Raw::default(), triggers: [false; 2], motion: None, sent: Sent::default() });
                    self.known.len() - 1
                }
            };
            let Some(slot) = slots.slot(s.id) else { continue };
            let k = &mut self.known[i];
            let raw = by_position(k.info.kind, &s.state);
            let mut buttons = raw.buttons.clone();
            for (n, (axis, button)) in [(PadAxis::LeftTrigger, PadButton::LeftTrigger), (PadAxis::RightTrigger, PadButton::RightTrigger)].into_iter().enumerate() {
                k.triggers[n] = self.thresholds.held(k.triggers[n], raw.axis(axis));
                if k.triggers[n] {
                    buttons.insert(button);
                }
            }
            let was = &k.raw.buttons;
            if live {
                for b in was.difference(&buttons) {
                    out.push(InputEvent::PadUpOn { pad: slot, button: *b });
                }
                for b in buttons.difference(was) {
                    out.push(InputEvent::PadDownOn { pad: slot, button: *b });
                    pressed = true;
                }
                for axis in AXES {
                    let v = raw.axis(axis);
                    if v != k.raw.axis(axis) {
                        out.push(InputEvent::PadMovedOn { pad: slot, axis, value: v });
                    }
                }
                if s.motion.is_some() && s.motion != k.motion {
                    out.push(InputEvent::PadMotion { pad: slot, motion: s.motion.unwrap_or_default() });
                }
            }
            k.raw = Raw { buttons, axes: raw.axes };
            k.motion = s.motion;
        }
        self.seen = seen;
        pressed
    }

    fn apply(&mut self, slots: &PadSlots, feedback: &mut PadFeedback, now: f64) {
        for k in &mut self.known {
            let Some(slot) = slots.slot(k.id) else { continue };
            let caps = k.info.caps;
            if caps.light {
                let rgb = feedback.light(slot);
                let q = rgb.map(|c| (c * 255.0).round() as u8);
                if k.sent.light != Some(q) && (k.sent.light.is_none() || now - k.sent.light_at >= LIGHT_EVERY) {
                    self.controllers.set_light(k.id, q.map(|c| f32::from(c) / 255.0));
                    k.sent.light = Some(q);
                    k.sent.light_at = now;
                }
            }
            let index = feedback.player_index(slot);
            if k.sent.player != Some(index) {
                self.controllers.set_player_index(k.id, index);
                k.sent.player = Some(index);
            }
            if caps.adaptive_triggers {
                for side in [TriggerSide::Left, TriggerSide::Right] {
                    let effect = feedback.trigger_effect(slot, side);
                    if k.sent.triggers[side as usize] != Some(effect) {
                        self.controllers.set_trigger(k.id, side, effect);
                        k.sent.triggers[side as usize] = Some(effect);
                    }
                }
            }
            if caps.motion {
                let on = feedback.motion(slot);
                if k.sent.motion != Some(on) {
                    self.controllers.set_motion(k.id, on);
                    k.sent.motion = Some(on);
                }
            }
        }
        let mut haptics_this_frame: Vec<(u64, u32)> = Vec::new();
        for (slot, pulse) in feedback.take_pulses() {
            let Some(id) = slots.device(slot) else { continue };
            let Some(k) = self.known.iter_mut().find(|k| k.id == id) else { continue };
            if !k.info.caps.rumble {
                continue;
            }
            match pulse {
                PadPulse::Rumble(r) => {
                    // The same rumble asked again while more than half of
                    // it is still to play: it plays on.
                    let until = now + f64::from(r.seconds);
                    let still = k.sent.rumble.is_some_and(|(p, end)| p.low == r.low && p.high == r.high && end - now >= f64::from(r.seconds) * 0.5);
                    let quiet = r.low == 0.0 && r.high == 0.0 && k.sent.rumble.is_none_or(|(_, end)| end <= now);
                    if !still && !quiet {
                        self.controllers.rumble(id, r);
                        k.sent.rumble = Some((r, until));
                    }
                }
                PadPulse::Haptic(h) => {
                    // A few a frame at most: a burst of hits is one feel.
                    let n = match haptics_this_frame.iter_mut().find(|(i, _)| *i == id) {
                        Some((_, n)) => {
                            *n += 1;
                            *n
                        }
                        None => {
                            haptics_this_frame.push((id, 1));
                            1
                        }
                    };
                    if n <= 2 {
                        self.controllers.haptic(id, h);
                    }
                }
            }
        }
    }

    fn suspend(&mut self) {
        self.controllers.stop_haptics();
        for k in &mut self.known {
            k.sent.rumble = None;
        }
    }

    fn pump(&mut self, seconds: f64) {
        self.controllers.pump(seconds);
    }

    fn set_thresholds(&mut self, thresholds: TriggerThreshold) {
        self.thresholds = thresholds;
    }

    fn list(&self, slots: &PadSlots) -> Vec<Listed> {
        self.known
            .iter()
            .filter_map(|k| Some(Listed { slot: slots.slot(k.id)?, info: k.info.clone(), elements: self.controllers.elements(k.id) }))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use scrap_core::input::{Battery, BatteryState, Input, PadCaps};

    /// What a test's controller asked of the outputs.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Asked {
        Light(u64, [f32; 3]),
        Player(u64, Option<u8>),
        Trigger(u64, TriggerSide, TriggerEffect),
        Motion(u64, bool),
        Rumble(u64, Rumble),
        Haptic(u64, Haptic),
        Stop,
    }

    /// Controllers a test plugs in, presses and unplugs.
    #[derive(Default)]
    pub struct Fake {
        pub connected: Vec<Seen>,
        pub asked: Vec<Asked>,
    }

    impl Controllers for Fake {
        fn scan(&mut self, out: &mut Vec<Seen>) {
            out.extend(self.connected.iter().cloned());
        }
        fn set_light(&mut self, id: u64, rgb: [f32; 3]) {
            self.asked.push(Asked::Light(id, rgb));
        }
        fn set_player_index(&mut self, id: u64, index: Option<u8>) {
            self.asked.push(Asked::Player(id, index));
        }
        fn set_trigger(&mut self, id: u64, side: TriggerSide, effect: TriggerEffect) {
            self.asked.push(Asked::Trigger(id, side, effect));
        }
        fn set_motion(&mut self, id: u64, on: bool) {
            self.asked.push(Asked::Motion(id, on));
        }
        fn rumble(&mut self, id: u64, rumble: Rumble) {
            self.asked.push(Asked::Rumble(id, rumble));
        }
        fn haptic(&mut self, id: u64, haptic: Haptic) {
            self.asked.push(Asked::Haptic(id, haptic));
        }
        fn stop_haptics(&mut self) {
            self.asked.push(Asked::Stop);
        }
    }

    pub fn dualsense(id: u64) -> Seen {
        let caps = PadCaps {
            analog_triggers: true,
            light: true,
            player_leds: true,
            rumble: true,
            adaptive_triggers: true,
            touchpad: true,
            motion: true,
            battery: true,
            ..PadCaps::default()
        };
        Seen {
            id,
            info: PadInfo {
                kind: PadKind::DualSense,
                name: "DualSense Wireless Controller".into(),
                backend: "gamecontroller".into(),
                caps,
                battery: Some(Battery { level: 0.8, state: BatteryState::Discharging }),
            },
            state: GcState { a_label: Some("Cross".into()), ..GcState::default() },
            motion: None,
        }
    }

    fn of_kind(id: u64, kind: PadKind, a_label: &str) -> Seen {
        let mut s = dualsense(id);
        s.info.kind = kind;
        s.info.caps = PadCaps { analog_triggers: true, rumble: true, ..PadCaps::default() };
        s.state.a_label = Some(a_label.into());
        s
    }

    fn poll(pads: &mut GcPads<Fake>, slots: &mut PadSlots, live: bool) -> Vec<InputEvent> {
        let mut out = Vec::new();
        pads.poll(slots, live, &mut out);
        out
    }

    fn set(pads: &mut GcPads<Fake>, id: u64, f: impl FnOnce(&mut GcState)) {
        f(&mut pads.controllers.connected.iter_mut().find(|s| s.id == id).unwrap().state);
    }

    #[test]
    fn pads_present_at_start_take_their_slots_on_the_first_frame() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(40), dualsense(41)], ..Fake::default() });
        let mut slots = PadSlots::new();
        let out = poll(&mut pads, &mut slots, true);
        assert_eq!(out[0], InputEvent::PadConnected(0));
        assert!(matches!(&out[1], InputEvent::PadInfo { pad: 0, info } if info.kind == PadKind::DualSense));
        assert_eq!(out[2], InputEvent::PadConnected(1));
        assert_eq!(out.len(), 4, "nothing pressed, nothing moved: {out:?}");
        assert_eq!(poll(&mut pads, &mut slots, true), [], "said once");
        let mut input = Input::new();
        out.iter().for_each(|e| input.handle(e));
        assert_eq!(input.pad(1).unwrap().kind(), PadKind::DualSense);
        assert_eq!(pads.list(&slots).len(), 2);
    }

    #[test]
    fn a_pad_unplugged_leaves_its_slot_waiting_and_a_new_one_takes_it() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1), dualsense(2)], ..Fake::default() });
        let mut slots = PadSlots::new();
        poll(&mut pads, &mut slots, true);
        set(&mut pads, 1, |s| s.a = true);
        poll(&mut pads, &mut slots, true);
        pads.controllers.connected.retain(|s| s.id != 1);
        assert_eq!(poll(&mut pads, &mut slots, true), [InputEvent::PadDisconnected(0)]);
        // Player 2 stays player 2; the pad back (as a new controller) is player 1 again.
        pads.controllers.connected.push(dualsense(3));
        let out = poll(&mut pads, &mut slots, true);
        assert_eq!(out[0], InputEvent::PadConnected(0));
        assert_eq!(slots.slot(2), Some(1));
        // A third, plugged in later: the next slot.
        pads.controllers.connected.push(dualsense(4));
        assert_eq!(poll(&mut pads, &mut slots, true)[0], InputEvent::PadConnected(2));
    }

    #[test]
    fn face_buttons_are_named_by_position_on_every_family() {
        let pressed = |kind, label: &str, f: fn(&mut GcState)| {
            let mut s = GcState { a_label: Some(label.into()), ..GcState::default() };
            f(&mut s);
            let raw = by_position(kind, &s);
            let mut b: Vec<_> = raw.buttons.into_iter().collect();
            b.sort_by_key(|b| format!("{b:?}"));
            b
        };
        // Cross / A / (positional) B: the bottom one, South.
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.a = true), [PadButton::South]);
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.b = true), [PadButton::East], "Circle");
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.x = true), [PadButton::West], "Square");
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.y = true), [PadButton::North], "Triangle");
        assert_eq!(pressed(PadKind::Xbox, "A", |s| s.a = true), [PadButton::South]);
        assert_eq!(pressed(PadKind::Xbox, "A", |s| s.y = true), [PadButton::North]);
        // Nintendo handed over by position: buttonA is the bottom one, labelled B.
        assert_eq!(pressed(PadKind::SwitchPro, "B", |s| s.a = true), [PadButton::South]);
        // Handed over by label: buttonA is the right one, labelled A — East.
        assert_eq!(pressed(PadKind::SwitchPro, "A", |s| s.a = true), [PadButton::East]);
        assert_eq!(pressed(PadKind::SwitchPro, "A", |s| s.b = true), [PadButton::South]);
        assert_eq!(pressed(PadKind::SwitchPro, "A", |s| s.x = true), [PadButton::North]);
        assert_eq!(pressed(PadKind::SwitchPro, "A", |s| s.y = true), [PadButton::West]);
        // The rest by what they are.
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.menu = true), [PadButton::Start], "Options");
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.options = true), [PadButton::Select], "Create");
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.touchpad_button = true), [PadButton::Touchpad]);
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.mute = true), [PadButton::Mute]);
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.left_shoulder = true), [PadButton::LeftBumper], "L1");
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.right_thumb = true), [PadButton::RightStick], "R3");
        assert_eq!(pressed(PadKind::Xbox, "A", |s| s.share = true), [PadButton::Share]);
        assert_eq!(pressed(PadKind::Xbox, "A", |s| s.paddles[2] = true), [PadButton::Paddle3]);
        assert_eq!(pressed(PadKind::DualSense, "Cross", |s| s.dpad = [false, true, false, false]), [PadButton::DPadDown]);
    }

    #[test]
    fn press_and_release_are_said_once_each_on_the_pads_own_slot() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1), dualsense(2)], ..Fake::default() });
        let mut slots = PadSlots::new();
        poll(&mut pads, &mut slots, true);
        set(&mut pads, 2, |s| s.a = true);
        assert_eq!(poll(&mut pads, &mut slots, true), [InputEvent::PadDownOn { pad: 1, button: PadButton::South }]);
        assert_eq!(poll(&mut pads, &mut slots, true), [], "held is not pressed again");
        set(&mut pads, 2, |s| s.a = false);
        assert_eq!(poll(&mut pads, &mut slots, true), [InputEvent::PadUpOn { pad: 1, button: PadButton::South }]);
    }

    #[test]
    fn triggers_are_analog_all_the_way_and_a_button_past_a_threshold_with_hysteresis() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1)], ..Fake::default() });
        let mut slots = PadSlots::new();
        let mut input = Input::new();
        let frame = |pads: &mut GcPads<Fake>, slots: &mut PadSlots, input: &mut Input| {
            input.begin_frame();
            let out = poll(pads, slots, true);
            out.iter().for_each(|e| input.handle(e));
            out
        };
        frame(&mut pads, &mut slots, &mut input);
        set(&mut pads, 1, |s| s.right_trigger = 0.3);
        assert_eq!(frame(&mut pads, &mut slots, &mut input), [InputEvent::PadMovedOn { pad: 0, axis: PadAxis::RightTrigger, value: 0.3 }]);
        assert_eq!(input.pad(0).unwrap().axis(PadAxis::RightTrigger), 0.3, "no dead zone, not quantised");
        assert!(!input.pad(0).unwrap().held(PadButton::RightTrigger));
        set(&mut pads, 1, |s| s.right_trigger = 0.62);
        frame(&mut pads, &mut slots, &mut input);
        assert!(input.pad(0).unwrap().pressed(PadButton::RightTrigger));
        set(&mut pads, 1, |s| s.right_trigger = 0.45);
        frame(&mut pads, &mut slots, &mut input);
        assert!(input.pad(0).unwrap().held(PadButton::RightTrigger), "inside the hysteresis");
        set(&mut pads, 1, |s| s.right_trigger = 0.2);
        frame(&mut pads, &mut slots, &mut input);
        assert!(input.pad(0).unwrap().released(PadButton::RightTrigger));
        // A threshold of the game's own.
        pads.set_thresholds(TriggerThreshold { press: 0.15, release: 0.1 });
        frame(&mut pads, &mut slots, &mut input);
        assert!(input.pad(0).unwrap().pressed(PadButton::RightTrigger));
    }

    #[test]
    fn sticks_keep_the_engines_dead_zone_and_up_is_positive() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1)], ..Fake::default() });
        let mut slots = PadSlots::new();
        let mut input = Input::new();
        poll(&mut pads, &mut slots, true).iter().for_each(|e| input.handle(e));
        set(&mut pads, 1, |s| s.left_stick = [0.1, 1.0]);
        poll(&mut pads, &mut slots, true).iter().for_each(|e| input.handle(e));
        let pad = input.pad(0).unwrap();
        assert_eq!(pad.axis(PadAxis::LeftX), 0.0, "rest drift inside the dead zone");
        assert_eq!(pad.axis(PadAxis::LeftY), 1.0, "pushed up: +1, as gilrs");
    }

    #[test]
    fn touchpad_fingers_are_buttons_and_places() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1)], ..Fake::default() });
        let mut slots = PadSlots::new();
        let mut input = Input::new();
        poll(&mut pads, &mut slots, true);
        set(&mut pads, 1, |s| s.fingers[0] = Some([0.1, -0.5]));
        poll(&mut pads, &mut slots, true).iter().for_each(|e| input.handle(e));
        let pad = input.pad(0).unwrap();
        assert!(pad.pressed(PadButton::Finger1) && !pad.held(PadButton::Finger2));
        assert_eq!((pad.axis(PadAxis::Finger1X), pad.axis(PadAxis::Finger1Y)), (0.1, -0.5), "no dead zone on a finger");
        set(&mut pads, 1, |s| s.fingers[0] = None);
        input.begin_frame();
        poll(&mut pads, &mut slots, true).iter().for_each(|e| input.handle(e));
        assert!(input.pad(0).unwrap().released(PadButton::Finger1));
    }

    #[test]
    fn in_the_background_pads_are_followed_but_say_nothing() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1)], ..Fake::default() });
        let mut slots = PadSlots::new();
        poll(&mut pads, &mut slots, false);
        assert_eq!(slots.slot(1), Some(0), "connecting is not input: it takes its slot");
        set(&mut pads, 1, |s| {
            s.a = true;
            s.left_stick = [1.0, 0.0];
        });
        assert_eq!(poll(&mut pads, &mut slots, false), []);
        // Back in front: held from before is not a fresh press.
        assert_eq!(poll(&mut pads, &mut slots, true), []);
        set(&mut pads, 1, |s| s.a = false);
        assert_eq!(poll(&mut pads, &mut slots, true), [InputEvent::PadUpOn { pad: 0, button: PadButton::South }]);
    }

    #[test]
    fn the_glass_and_a_real_pad_share_the_slots_without_reading_one_twice() {
        // The glass's pad took slot 0 first (a phone, the game started on touch).
        let mut slots = PadSlots::new();
        assert_eq!(slots.connect(crate::shell::PAGE_PAD), 0);
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(9)], ..Fake::default() });
        let out = poll(&mut pads, &mut slots, true);
        assert_eq!(out[0], InputEvent::PadConnected(1));
        // One backend: the same controller seen again is the same slot, said once.
        assert_eq!(poll(&mut pads, &mut slots, true), []);
        set(&mut pads, 9, |s| s.a = true);
        let mut out = Vec::new();
        assert!(pads.poll(&mut slots, true, &mut out), "a real pad's press hides the glass");
        assert_eq!(out, [InputEvent::PadDownOn { pad: 1, button: PadButton::South }]);
        // The feedback for the glass's slot goes nowhere.
        let mut f = PadFeedback::new();
        f.rumble(0, 1.0, 1.0, 1.0);
        pads.apply(&slots, &mut f, 0.0);
        assert!(!pads.controllers.asked.iter().any(|a| matches!(a, Asked::Rumble(..))));
    }

    #[test]
    fn feedback_is_sent_as_it_changes_and_the_light_no_faster_than_thirty_a_second() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1), dualsense(2)], ..Fake::default() });
        let mut slots = PadSlots::new();
        poll(&mut pads, &mut slots, true);
        let mut f = PadFeedback::new();
        f.set_player_colours(&[[1.0, 0.5, 0.0], [0.0, 0.0, 1.0]]);
        pads.apply(&slots, &mut f, 0.0);
        let asked = std::mem::take(&mut pads.controllers.asked);
        assert!(asked.contains(&Asked::Light(1, [1.0, 128.0 / 255.0, 0.0])), "player 1 orange: {asked:?}");
        assert!(asked.contains(&Asked::Light(2, [0.0, 0.0, 1.0])), "player 2 blue");
        assert!(asked.contains(&Asked::Player(1, Some(0))) && asked.contains(&Asked::Player(2, Some(1))), "LEDs by slot");
        assert!(asked.contains(&Asked::Trigger(1, TriggerSide::Right, TriggerEffect::Off)));
        assert!(asked.contains(&Asked::Motion(1, false)));
        // Nothing changed: nothing sent.
        pads.apply(&slots, &mut f, 0.016);
        assert_eq!(pads.controllers.asked, []);
        // A colour set every frame: at most one send per 1/30 s.
        for i in 0..60 {
            f.set_light(0, [i as f32 / 60.0, 0.0, 0.0]);
            pads.apply(&slots, &mut f, 1.0 + i as f64 / 240.0);
        }
        let lights = pads.controllers.asked.iter().filter(|a| matches!(a, Asked::Light(1, _))).count();
        assert!((7..=9).contains(&lights), "{lights} sends in a quarter second");
        pads.controllers.asked.clear();
        f.set_trigger_effect(1, TriggerSide::Right, TriggerEffect::detent());
        f.set_motion(0, true);
        pads.apply(&slots, &mut f, 2.0);
        assert!(pads.controllers.asked.contains(&Asked::Trigger(2, TriggerSide::Right, TriggerEffect::detent())));
        assert!(pads.controllers.asked.contains(&Asked::Motion(1, true)));
    }

    #[test]
    fn a_rumble_held_each_frame_is_sent_once_and_haptics_are_capped_per_frame() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1)], ..Fake::default() });
        let mut slots = PadSlots::new();
        poll(&mut pads, &mut slots, true);
        let mut f = PadFeedback::new();
        pads.apply(&slots, &mut f, 0.0);
        pads.controllers.asked.clear();
        for i in 0..10 {
            f.rumble(0, 0.4, 0.2, 0.5);
            pads.apply(&slots, &mut f, i as f64 * 0.016);
        }
        assert_eq!(pads.controllers.asked.iter().filter(|a| matches!(a, Asked::Rumble(..))).count(), 1, "{:?}", pads.controllers.asked);
        pads.controllers.asked.clear();
        for _ in 0..5 {
            f.haptic(0, Haptic::tap(1.0));
        }
        pads.apply(&slots, &mut f, 1.0);
        assert_eq!(pads.controllers.asked.len(), 2);
        assert!(f.pulses().is_empty(), "taken");
        pads.suspend();
        assert_eq!(pads.controllers.asked.last(), Some(&Asked::Stop));
    }

    #[test]
    fn a_pad_without_a_feature_is_not_asked_for_it() {
        let mut pads = GcPads::new(Fake { connected: vec![of_kind(5, PadKind::Xbox, "A")], ..Fake::default() });
        let mut slots = PadSlots::new();
        poll(&mut pads, &mut slots, true);
        let mut f = PadFeedback::new();
        f.set_light(0, [1.0, 0.0, 0.0]);
        f.set_trigger_effect(0, TriggerSide::Left, TriggerEffect::Feedback { start: 0.2, strength: 0.5 });
        f.set_motion(0, true);
        pads.apply(&slots, &mut f, 0.0);
        assert!(pads.controllers.asked.iter().all(|a| matches!(a, Asked::Player(..))), "{:?}", pads.controllers.asked);
    }

    #[test]
    fn motion_and_battery_reach_the_slots_state() {
        let mut pads = GcPads::new(Fake { connected: vec![dualsense(1)], ..Fake::default() });
        let mut slots = PadSlots::new();
        let mut input = Input::new();
        poll(&mut pads, &mut slots, true).iter().for_each(|e| input.handle(e));
        assert_eq!(input.pad(0).unwrap().info().unwrap().battery.unwrap().level, 0.8);
        pads.controllers.connected[0].motion = Some(PadMotion { gravity: [0.0, -1.0, 0.0], ..PadMotion::default() });
        pads.controllers.connected[0].info.battery = Some(Battery { level: 0.81, state: BatteryState::Discharging });
        let out = poll(&mut pads, &mut slots, true);
        assert_eq!(out.len(), 1, "a percent is no news: {out:?}");
        out.iter().for_each(|e| input.handle(e));
        assert_eq!(input.pad(0).unwrap().motion().unwrap().gravity, [0.0, -1.0, 0.0]);
        pads.controllers.connected[0].info.battery = Some(Battery { level: 0.6, state: BatteryState::Charging });
        let out = poll(&mut pads, &mut slots, true);
        out.iter().for_each(|e| input.handle(e));
        assert_eq!(input.pad(0).unwrap().info().unwrap().battery.unwrap().state, BatteryState::Charging);
    }
}
