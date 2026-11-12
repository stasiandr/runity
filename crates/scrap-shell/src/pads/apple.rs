//! The GameController framework under [`super::gc::GcPads`]: every
//! `GCController` connected, read into [`GcState`] once a frame, and its
//! outputs — `GCDeviceLight`, `playerIndex`, CoreHaptics engines from
//! `GCDeviceHaptics`, `GCDualSenseAdaptiveTrigger`, `GCMotion`.
//!
//! Discovery: `GCController.controllers()` every frame (a pad present at
//! launch is there on the first frame it is listed — no event to wait
//! for), plus observers of the connect and disconnect notifications, which
//! is what makes the framework start talking to the system's daemon
//! early. On macOS `shouldMonitorBackgroundEvents` is on: a game started
//! from a terminal is not always "the frontmost application" to the
//! framework, and a pad that reads nothing is the failure this backend
//! exists to fix — the engine itself says nothing while the window is in
//! the background (`GcPads::poll`'s `live`), unless the game asks.
//!
//! Haptics: an engine per place on the pad, made the first time it is
//! needed and started then (not at connection: most games never rumble),
//! stopped when the app leaves the foreground, dropped with its pad.

use super::gc::{Controllers, GcState, Seen};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{msg_send, sel, AnyThread};
use objc2_core_haptics::{
    CHHapticEngine, CHHapticEvent, CHHapticEventParameter, CHHapticEventParameterIDHapticIntensity, CHHapticEventParameterIDHapticSharpness,
    CHHapticEventTypeHapticContinuous, CHHapticEventTypeHapticTransient, CHHapticPattern, CHHapticPatternPlayer,
};
use objc2_foundation::{NSArray, NSDate, NSDefaultRunLoopMode, NSNotificationCenter, NSObjectProtocol, NSOperationQueue, NSRunLoop, NSString};
use objc2_game_controller::{
    GCColor, GCController, GCControllerButtonInput, GCControllerDidConnectNotification, GCControllerDidDisconnectNotification,
    GCControllerDirectionPad, GCControllerPlayerIndex, GCDevice, GCDeviceBatteryState, GCDualSenseGamepad, GCDualShockGamepad,
    GCExtendedGamepad, GCHapticsLocality, GCHapticsLocalityHandles, GCHapticsLocalityLeftHandle, GCHapticsLocalityLeftTrigger,
    GCHapticsLocalityRightHandle, GCHapticsLocalityRightTrigger, GCHapticsLocalityTriggers, GCXboxGamepad,
};
use scrap_core::input::{Battery, BatteryState, Haptic, HapticPlace, PadCaps, PadInfo, PadKind, PadMotion, Rumble, TriggerEffect, TriggerSide};

type Player = Retained<ProtocolObject<dyn CHHapticPatternPlayer>>;

/// One controller and what is kept for it.
struct Entry {
    id: u64,
    controller: Retained<GCController>,
    /// Kind, name and abilities: looked at once.
    info: PadInfo,
    /// The microphone button, when the profile names one.
    mute: Option<Retained<GCControllerButtonInput>>,
    motion_on: bool,
    /// Engines by place, started or not.
    engines: Vec<(HapticPlace, Retained<CHHapticEngine>, bool)>,
    /// The rumble's players, stopped by the next rumble.
    rumbling: Vec<Player>,
}

pub struct AppleControllers {
    entries: Vec<Entry>,
    next: u64,
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

impl AppleControllers {
    /// The framework, watched from now. `background` is for the record:
    /// on macOS background events are always monitored and the engine
    /// gates them itself.
    pub fn new(background: bool) -> Self {
        let _ = background;
        #[cfg(target_os = "macos")]
        unsafe {
            GCController::setShouldMonitorBackgroundEvents(true);
        }
        let mut observers = Vec::new();
        let centre = NSNotificationCenter::defaultCenter();
        let queue = NSOperationQueue::mainQueue();
        let block = block2::RcBlock::new(|_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {});
        for name in unsafe { [GCControllerDidConnectNotification, GCControllerDidDisconnectNotification] } {
            observers.push(unsafe { centre.addObserverForName_object_queue_usingBlock(Some(name), None, Some(&queue), &block) });
        }
        Self { entries: Vec::new(), next: 1, _observers: observers }
    }

    fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn entry_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }
}

/// What a controller is and can do: looked at when it comes.
fn describe(c: &GCController) -> (PadInfo, Option<Retained<GCControllerButtonInput>>) {
    unsafe {
        let category = c.productCategory().to_string();
        let vendor = c.vendorName().map(|n| n.to_string());
        let ext = c.extendedGamepad();
        let dualsense = ext.as_ref().is_some_and(|e| e.downcast_ref::<GCDualSenseGamepad>().is_some());
        let dualshock = ext.as_ref().is_some_and(|e| e.downcast_ref::<GCDualShockGamepad>().is_some());
        let xbox = ext.as_ref().and_then(|e| e.downcast_ref::<GCXboxGamepad>().map(objc2::Message::retain));
        let mut kind = PadKind::from_name(&category);
        if kind == PadKind::Generic {
            kind = vendor.as_deref().map_or(PadKind::Generic, PadKind::from_name);
        }
        if kind == PadKind::Generic {
            kind = if dualsense {
                PadKind::DualSense
            } else if dualshock {
                PadKind::DualShock4
            } else if xbox.is_some() {
                PadKind::Xbox
            } else if category.eq_ignore_ascii_case("MFi") {
                PadKind::Mfi
            } else {
                PadKind::Generic
            };
        }
        // The microphone's button: no property names it (GameController
        // up to the 26 SDKs has none); a profile that has it names it.
        let profile = c.physicalInputProfile();
        let buttons = profile.buttons();
        let mute = buttons.allKeys().iter().find_map(|k| {
            let n = k.to_string().to_ascii_lowercase();
            (n.contains("mute") || n.contains("microphone") || n.contains(" mic")).then(|| buttons.objectForKey(&k)).flatten()
        });
        let haptics = c.haptics();
        let has = |loc: &GCHapticsLocality| haptics.as_ref().is_some_and(|h| h.supportedLocalities().containsObject(loc));
        let paddles = xbox.as_ref().map_or(0, |x| {
            [x.paddleButton1(), x.paddleButton2(), x.paddleButton3(), x.paddleButton4()].iter().filter(|p| p.is_some()).count() as u8
        });
        let caps = PadCaps {
            analog_triggers: ext.as_ref().is_some_and(|e| e.leftTrigger().isAnalog()),
            light: c.light().is_some(),
            player_leds: matches!(kind, PadKind::DualSense | PadKind::DualSenseEdge | PadKind::SwitchPro | PadKind::JoyCon),
            rumble: haptics.is_some(),
            trigger_rumble: has(GCHapticsLocalityTriggers),
            adaptive_triggers: dualsense,
            touchpad: dualsense || dualshock,
            motion: c.motion().is_some(),
            battery: c.battery().is_some(),
            mute_button: mute.is_some(),
            paddles,
        };
        let name = vendor.unwrap_or_else(|| category.clone());
        (PadInfo { kind, name, backend: "gamecontroller".into(), caps, battery: None }, mute)
    }
}

fn pressed(b: &GCControllerButtonInput) -> bool {
    unsafe { b.isPressed() }
}

fn stick(d: &GCControllerDirectionPad) -> [f32; 2] {
    unsafe { [d.xAxis().value(), d.yAxis().value()] }
}

/// A touchpad finger: down while it reads off the centre (the framework
/// puts it back to 0, 0 when the finger lifts).
fn finger(d: &GCControllerDirectionPad) -> Option<[f32; 2]> {
    let at = stick(d);
    (at != [0.0, 0.0]).then_some(at)
}

/// What a controller's elements say now.
fn read(c: &GCController, mute: Option<&GCControllerButtonInput>) -> GcState {
    let mut s = GcState::default();
    unsafe {
        if let Some(e) = c.extendedGamepad() {
            read_extended(&e, &mut s);
        } else if let Some(m) = c.microGamepad() {
            s.a = pressed(&m.buttonA());
            s.x = pressed(&m.buttonX());
            s.menu = pressed(&m.buttonMenu());
            let d = m.dpad();
            s.dpad = [pressed(&d.up()), pressed(&d.down()), pressed(&d.left()), pressed(&d.right())];
        }
    }
    s.mute = mute.is_some_and(pressed);
    s
}

unsafe fn read_extended(e: &GCExtendedGamepad, s: &mut GcState) {
    unsafe {
        let a = e.buttonA();
        s.a = pressed(&a);
        s.a_label = a.localizedName().map(|n| n.to_string());
        s.b = pressed(&e.buttonB());
        s.x = pressed(&e.buttonX());
        s.y = pressed(&e.buttonY());
        s.left_shoulder = pressed(&e.leftShoulder());
        s.right_shoulder = pressed(&e.rightShoulder());
        s.left_trigger = e.leftTrigger().value();
        s.right_trigger = e.rightTrigger().value();
        s.left_thumb = e.leftThumbstickButton().is_some_and(|b| pressed(&b));
        s.right_thumb = e.rightThumbstickButton().is_some_and(|b| pressed(&b));
        let d = e.dpad();
        s.dpad = [pressed(&d.up()), pressed(&d.down()), pressed(&d.left()), pressed(&d.right())];
        s.left_stick = stick(&e.leftThumbstick());
        s.right_stick = stick(&e.rightThumbstick());
        s.menu = pressed(&e.buttonMenu());
        s.options = e.buttonOptions().is_some_and(|b| pressed(&b));
        s.home = e.buttonHome().is_some_and(|b| pressed(&b));
        if let Some(ds) = e.downcast_ref::<GCDualSenseGamepad>() {
            s.touchpad_button = pressed(&ds.touchpadButton());
            s.fingers = [finger(&ds.touchpadPrimary()), finger(&ds.touchpadSecondary())];
        } else if let Some(ds) = e.downcast_ref::<GCDualShockGamepad>() {
            s.touchpad_button = ds.touchpadButton().is_some_and(|b| pressed(&b));
            s.fingers = [ds.touchpadPrimary().and_then(|d| finger(&d)), ds.touchpadSecondary().and_then(|d| finger(&d))];
        } else if let Some(x) = e.downcast_ref::<GCXboxGamepad>() {
            s.share = x.buttonShare().is_some_and(|b| pressed(&b));
            for (i, p) in [x.paddleButton1(), x.paddleButton2(), x.paddleButton3(), x.paddleButton4()].into_iter().enumerate() {
                s.paddles[i] = p.is_some_and(|b| pressed(&b));
            }
        }
    }
}

fn battery(c: &GCController) -> Option<Battery> {
    unsafe {
        c.battery().map(|b| Battery {
            level: b.batteryLevel(),
            state: match b.batteryState() {
                GCDeviceBatteryState::Discharging => BatteryState::Discharging,
                GCDeviceBatteryState::Charging => BatteryState::Charging,
                GCDeviceBatteryState::Full => BatteryState::Full,
                _ => BatteryState::Unknown,
            },
        })
    }
}

fn motion(c: &GCController) -> Option<PadMotion> {
    unsafe {
        let m = c.motion()?;
        let v3 = |x: f64, y: f64, z: f64| [x as f32, y as f32, z as f32];
        let g = m.gravity();
        let a = m.userAcceleration();
        let r = m.rotationRate();
        let attitude = m.hasAttitude().then(|| {
            let q = m.attitude();
            [q.x as f32, q.y as f32, q.z as f32, q.w as f32]
        });
        Some(PadMotion { gravity: v3(g.x, g.y, g.z), acceleration: v3(a.x, a.y, a.z), rotation_rate: v3(r.x, r.y, r.z), attitude })
    }
}

fn locality(place: HapticPlace) -> &'static GCHapticsLocality {
    unsafe {
        match place {
            HapticPlace::Handles => GCHapticsLocalityHandles,
            HapticPlace::LeftHandle => GCHapticsLocalityLeftHandle,
            HapticPlace::RightHandle => GCHapticsLocalityRightHandle,
            HapticPlace::Triggers => GCHapticsLocalityTriggers,
            HapticPlace::LeftTrigger => GCHapticsLocalityLeftTrigger,
            HapticPlace::RightTrigger => GCHapticsLocalityRightTrigger,
        }
    }
}

impl Entry {
    /// The engine for a place on the pad, made and started the first time
    /// (or the handles', where the pad has no such place).
    fn engine(&mut self, place: HapticPlace) -> Option<Retained<CHHapticEngine>> {
        let haptics = unsafe { self.controller.haptics() }?;
        let supported = unsafe { haptics.supportedLocalities() };
        let place = if supported.containsObject(locality(place)) { place } else { HapticPlace::Handles };
        let i = match self.engines.iter().position(|(p, ..)| *p == place) {
            Some(i) => i,
            None => {
                // `createEngineWithLocality:` — the bindings have it on iOS
                // only; the method is there on macOS 11 and later too.
                let engine: Option<Retained<CHHapticEngine>> = unsafe { msg_send![&*haptics, createEngineWithLocality: locality(place)] };
                self.engines.push((place, engine?, false));
                self.engines.len() - 1
            }
        };
        let (_, engine, started) = &mut self.engines[i];
        if !*started {
            if let Err(e) = unsafe { engine.startAndReturnError() } {
                eprintln!("pad haptics: {}", e.localizedDescription());
                return None;
            }
            *started = true;
        }
        Some(engine.clone())
    }

    /// Play one event (a tap when `seconds` is 0, else a buzz) where it
    /// goes.
    fn play(&mut self, place: HapticPlace, strength: f32, sharpness: f32, seconds: f32) -> Option<Player> {
        let engine = self.engine(place)?;
        let attempt = |engine: &CHHapticEngine| -> Result<Player, Retained<objc2_foundation::NSError>> {
            unsafe {
                let intensity = CHHapticEventParameter::initWithParameterID_value(CHHapticEventParameter::alloc(), CHHapticEventParameterIDHapticIntensity, strength);
                let sharp = CHHapticEventParameter::initWithParameterID_value(CHHapticEventParameter::alloc(), CHHapticEventParameterIDHapticSharpness, sharpness);
                let params = NSArray::from_retained_slice(&[intensity, sharp]);
                let event = if seconds <= 0.0 {
                    CHHapticEvent::initWithEventType_parameters_relativeTime(CHHapticEvent::alloc(), CHHapticEventTypeHapticTransient, &params, 0.0)
                } else {
                    CHHapticEvent::initWithEventType_parameters_relativeTime_duration(CHHapticEvent::alloc(), CHHapticEventTypeHapticContinuous, &params, 0.0, f64::from(seconds))
                };
                let pattern = CHHapticPattern::initWithEvents_parameters_error(CHHapticPattern::alloc(), &NSArray::from_retained_slice(&[event]), &NSArray::new())?;
                let player = engine.createPlayerWithPattern_error(&pattern)?;
                player.startAtTime_error(0.0)?;
                Ok(player)
            }
        };
        match attempt(&engine) {
            Ok(p) => Some(p),
            Err(_) => {
                // The engine stopped by itself (the system's reset): once more.
                unsafe { engine.startAndReturnError() }.ok()?;
                attempt(&engine).ok()
            }
        }
    }

    fn stop_rumble(&mut self) {
        for p in self.rumbling.drain(..) {
            let _ = unsafe { p.stopAtTime_error(0.0) };
        }
    }

    fn stop_engines(&mut self) {
        self.stop_rumble();
        for (_, engine, started) in &mut self.engines {
            if *started {
                let none: *mut AnyObject = std::ptr::null_mut();
                let _: () = unsafe { msg_send![&**engine, stopWithCompletionHandler: none] };
                *started = false;
            }
        }
    }
}

impl Controllers for AppleControllers {
    fn scan(&mut self, out: &mut Vec<Seen>) {
        let list = unsafe { GCController::controllers() };
        let now: Vec<Retained<GCController>> = list.iter().collect();
        self.entries.retain(|e| now.iter().any(|c| Retained::as_ptr(c) == Retained::as_ptr(&e.controller)));
        for c in now {
            if !self.entries.iter().any(|e| Retained::as_ptr(&e.controller) == Retained::as_ptr(&c)) {
                let (info, mute) = describe(&c);
                let id = self.next;
                self.next += 1;
                self.entries.push(Entry { id, controller: c, info, mute, motion_on: false, engines: Vec::new(), rumbling: Vec::new() });
            }
        }
        for e in &self.entries {
            let c = &e.controller;
            let mut info = e.info.clone();
            info.battery = battery(c);
            out.push(Seen { id: e.id, info, state: read(c, e.mute.as_deref()), motion: if e.motion_on { motion(c) } else { None } });
        }
    }

    fn set_light(&mut self, id: u64, rgb: [f32; 3]) {
        let Some(e) = self.entry(id) else { return };
        unsafe {
            if let Some(light) = e.controller.light() {
                light.setColor(&GCColor::initWithRed_green_blue(GCColor::alloc(), rgb[0], rgb[1], rgb[2]));
            }
        }
    }

    fn set_player_index(&mut self, id: u64, index: Option<u8>) {
        let Some(e) = self.entry(id) else { return };
        let index = match index {
            Some(i) if i < 4 => GCControllerPlayerIndex(i as isize),
            _ => GCControllerPlayerIndex::IndexUnset,
        };
        unsafe { e.controller.setPlayerIndex(index) };
    }

    fn set_trigger(&mut self, id: u64, side: TriggerSide, effect: TriggerEffect) {
        let Some(e) = self.entry(id) else { return };
        unsafe {
            let Some(ext) = e.controller.extendedGamepad() else { return };
            let Some(ds) = ext.downcast_ref::<GCDualSenseGamepad>() else { return };
            let t = match side {
                TriggerSide::Left => ds.leftTrigger(),
                TriggerSide::Right => ds.rightTrigger(),
            };
            match effect.clamped() {
                TriggerEffect::Off => t.setModeOff(),
                TriggerEffect::Feedback { start, strength } => t.setModeFeedbackWithStartPosition_resistiveStrength(start, strength),
                TriggerEffect::Weapon { start, end, strength } => t.setModeWeaponWithStartPosition_endPosition_resistiveStrength(start, end, strength),
                TriggerEffect::Vibration { start, amplitude, frequency } => t.setModeVibrationWithStartPosition_amplitude_frequency(start, amplitude, frequency),
                TriggerEffect::Slope { start, end, start_strength, end_strength } => {
                    // iOS 17.4 / macOS 14.4 and later; before, an even pull.
                    if t.respondsToSelector(sel!(setModeSlopeFeedbackWithStartPosition:endPosition:startStrength:endStrength:)) {
                        t.setModeSlopeFeedbackWithStartPosition_endPosition_startStrength_endStrength(start, end, start_strength, end_strength)
                    } else {
                        t.setModeFeedbackWithStartPosition_resistiveStrength(start, (start_strength + end_strength) / 2.0)
                    }
                }
            }
        }
    }

    fn set_motion(&mut self, id: u64, on: bool) {
        let Some(e) = self.entry_mut(id) else { return };
        e.motion_on = on;
        unsafe {
            if let Some(m) = e.controller.motion() {
                if m.sensorsRequireManualActivation() {
                    m.setSensorsActive(on);
                }
            }
        }
    }

    fn rumble(&mut self, id: u64, r: Rumble) {
        let Some(e) = self.entry_mut(id) else { return };
        e.stop_rumble();
        if r.seconds <= 0.0 || (r.low <= 0.0 && r.high <= 0.0) {
            return;
        }
        // The heavy motor a dull left handle, the light one a sharp right:
        // where the pad has two handles; one engine for both elsewhere.
        let split = unsafe { e.controller.haptics() }.is_some_and(|h| unsafe { h.supportedLocalities().containsObject(GCHapticsLocalityLeftHandle) });
        if split {
            if r.low > 0.0 {
                if let Some(p) = e.play(HapticPlace::LeftHandle, r.low, 0.2, r.seconds) {
                e.rumbling.push(p);
            }
            }
            if r.high > 0.0 {
                if let Some(p) = e.play(HapticPlace::RightHandle, r.high, 0.8, r.seconds) {
                e.rumbling.push(p);
            }
            }
        } else {
            let strength = r.low.max(r.high);
            let sharpness = 0.2 + 0.6 * r.high / (r.low + r.high).max(1e-3);
            if let Some(p) = e.play(HapticPlace::Handles, strength, sharpness, r.seconds) {
                e.rumbling.push(p);
            }
        }
    }

    fn haptic(&mut self, id: u64, h: Haptic) {
        if let Some(e) = self.entry_mut(id) {
            let _ = e.play(h.place, h.strength.clamp(0.0, 1.0), h.sharpness.clamp(0.0, 1.0), h.seconds.clamp(0.0, 5.0));
        }
    }

    fn stop_haptics(&mut self) {
        for e in &mut self.entries {
            e.stop_engines();
        }
    }

    fn forget(&mut self, id: u64) {
        if let Some(e) = self.entry_mut(id) {
            e.stop_engines();
        }
        self.entries.retain(|e| e.id != id);
    }

    fn elements(&self, id: u64) -> Vec<String> {
        let Some(e) = self.entry(id) else { return Vec::new() };
        let mut names: Vec<String> = unsafe { e.controller.physicalInputProfile().elements().allKeys() }.iter().map(|k: Retained<NSString>| k.to_string()).collect();
        names.sort();
        names
    }

    fn pump(&mut self, seconds: f64) {
        unsafe {
            NSRunLoop::currentRunLoop().runMode_beforeDate(NSDefaultRunLoopMode, &NSDate::dateWithTimeIntervalSinceNow(seconds.max(0.0)));
        }
    }
}
