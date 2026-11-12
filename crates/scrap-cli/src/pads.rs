//! `scrap pads`: the gamepads as the engine sees them — listed with their
//! slot, kind, backend and abilities, then every button and axis change
//! printed for a while, so a person with a pad in their hands checks the
//! hardware in one command. Outputs on request: `--light ff8000`,
//! `--rumble`, `--trigger weapon`, `--motion`. docs/gamepads.md.

use anyhow::{bail, Context, Result};
use scrap_shell::pads::{self, Backend};
use scrap::input::{
    Haptic, HapticPlace, InputEvent, PadAxis, PadFeedback, PadSlots, TriggerEffect, TriggerSide,
};
use std::time::{Duration, Instant};

const HELP: &str = "scrap pads [--seconds N] [--backend gamecontroller|gilrs] [--light RRGGBB] [--rumble] [--trigger off|feedback|weapon|vibration|slope|detent] [--motion]
  Lists the gamepads (slot, kind, backend, what each can do, its elements),
  then prints every button and axis change for N seconds (20; 0 lists only).
  --light     the light bar of every pad (default: the players' colours)
  --rumble    each pad rumbles: heavy motor, light motor, then three taps
  --trigger   that resistance on both triggers (a DualSense) while watching
  --motion    the gyro and accelerometer on, printed twice a second";

/// A rumble step: when, which slot, a rumble (low, high, seconds) or a tap.
type Step = (f64, usize, Option<(f32, f32, f32)>, Option<Haptic>);

struct Options {
    seconds: f64,
    backend: Backend,
    light: Option<[f32; 3]>,
    rumble: bool,
    trigger: Option<TriggerEffect>,
    motion: bool,
}

fn options(args: &[String]) -> Result<Options> {
    let mut o = Options { seconds: 20.0, backend: pads::chosen(), light: None, rumble: false, trigger: None, motion: false };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} wants a value"));
        match arg.as_str() {
            "--seconds" | "-s" => o.seconds = value()?.parse().context("--seconds wants a number")?,
            "--backend" => o.backend = pads::choose(Some(value()?), cfg!(any(target_os = "macos", target_os = "ios"))),
            "--light" => o.light = Some(hex(value()?)?),
            "--rumble" => o.rumble = true,
            "--motion" => o.motion = true,
            "--watch" => {}
            "--trigger" => {
                o.trigger = Some(match value()?.as_str() {
                    "off" => TriggerEffect::Off,
                    "feedback" => TriggerEffect::Feedback { start: 0.2, strength: 0.7 },
                    "weapon" => TriggerEffect::Weapon { start: 0.3, end: 0.6, strength: 0.9 },
                    "vibration" => TriggerEffect::Vibration { start: 0.1, amplitude: 0.8, frequency: 0.4 },
                    "slope" => TriggerEffect::Slope { start: 0.0, end: 1.0, start_strength: 0.1, end_strength: 1.0 },
                    "detent" => TriggerEffect::detent(),
                    other => bail!("no trigger effect {other}: off, feedback, weapon, vibration, slope, detent"),
                })
            }
            "--help" | "-h" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => bail!("unknown option {other}\n{HELP}"),
        }
    }
    Ok(o)
}

fn hex(text: &str) -> Result<[f32; 3]> {
    let t = text.trim_start_matches('#');
    if t.len() != 6 {
        bail!("--light wants RRGGBB, not {text}");
    }
    let n = u32::from_str_radix(t, 16).context("--light wants RRGGBB")?;
    Ok([(n >> 16) & 255, (n >> 8) & 255, n & 255].map(|c| c as f32 / 255.0))
}

pub fn run(args: &[String]) -> Result<std::process::ExitCode> {
    let o = options(args)?;
    let Some(mut source) = pads::open(o.backend, true) else {
        println!("no gamepad backend ({})", o.backend.name());
        return Ok(std::process::ExitCode::FAILURE);
    };
    println!("backend: {}", source.backend().name());
    let mut slots = PadSlots::new();
    let mut feedback = PadFeedback::new();
    let mut said = Vec::new();
    let began = Instant::now();
    // The framework finds paired pads a moment after it is first asked.
    while began.elapsed() < Duration::from_millis(1500) {
        source.pump(0.05);
        source.poll(&mut slots, true, &mut said);
        if !source.list(&slots).is_empty() && began.elapsed() > Duration::from_millis(400) {
            break;
        }
    }
    let listed = source.list(&slots);
    if listed.is_empty() {
        println!("no controllers. Paired and switched on? (Inside a sandbox the system may not hand this process any.)");
    }
    for l in &listed {
        let i = &l.info;
        let c = i.caps;
        let mut can: Vec<String> = Vec::new();
        for (on, what) in [
            (c.analog_triggers, "analog triggers"),
            (c.light, "light"),
            (c.player_leds, "player LEDs"),
            (c.rumble, "rumble"),
            (c.trigger_rumble, "trigger rumble"),
            (c.adaptive_triggers, "adaptive triggers"),
            (c.touchpad, "touchpad"),
            (c.motion, "motion"),
            (c.battery, "battery"),
        ] {
            if on {
                can.push(what.into());
            }
        }
        can.push(if c.mute_button { "mic button".into() } else { "no mic button exposed".into() });
        if c.paddles > 0 {
            can.push(format!("{} paddles", c.paddles));
        }
        println!("slot {}: {} — kind {:?}, glyphs {:?}, backend {}", l.slot, i.name, i.kind, i.kind.glyphs(), i.backend);
        if let Some(b) = i.battery {
            println!("  battery {:.0}% {:?}", b.level * 100.0, b.state);
        }
        println!("  can: {}", can.join(", "));
        if !l.elements.is_empty() {
            println!("  elements: {}", l.elements.join(", "));
        }
    }
    let slots_on: Vec<usize> = listed.iter().map(|l| l.slot).collect();
    for &slot in &slots_on {
        if let Some(rgb) = o.light {
            feedback.set_light(slot, rgb);
        }
        if let Some(effect) = o.trigger {
            feedback.set_trigger_effect(slot, TriggerSide::Left, effect);
            feedback.set_trigger_effect(slot, TriggerSide::Right, effect);
        }
        feedback.set_motion(slot, o.motion);
    }
    source.apply(&slots, &mut feedback, 0.0);
    // The rumble's steps, at times from now.
    let mut rumbles: Vec<Step> = Vec::new();
    if o.rumble {
        for &slot in &slots_on {
            rumbles.push((0.0, slot, Some((1.0, 0.0, 0.6)), None));
            rumbles.push((0.9, slot, Some((0.0, 1.0, 0.6)), None));
            for k in 0..3 {
                rumbles.push((1.8 + k as f64 * 0.25, slot, None, Some(Haptic::tap(1.0).at(if k == 1 { HapticPlace::RightHandle } else { HapticPlace::LeftHandle }))));
            }
        }
        println!("rumble: heavy motor, light motor, three taps on each pad");
    }
    if o.seconds > 0.0 {
        println!("watching for {:.0} s — press buttons, pull triggers, touch the touchpad (Ctrl-C to stop)", o.seconds);
    }
    let watch = Instant::now();
    let mut last_axis: std::collections::HashMap<(usize, PadAxis), f32> = Default::default();
    let mut last_motion = Instant::now();
    while watch.elapsed().as_secs_f64() < o.seconds {
        source.pump(0.008);
        said.clear();
        source.poll(&mut slots, true, &mut said);
        let t = watch.elapsed().as_secs_f64();
        rumbles.retain(|(at, slot, r, h)| {
            if *at > t {
                return true;
            }
            if let Some((low, high, s)) = r {
                feedback.rumble(*slot, *low, *high, *s);
            }
            if let Some(h) = h {
                feedback.haptic(*slot, *h);
            }
            false
        });
        source.apply(&slots, &mut feedback, t + 1.0);
        for e in &said {
            match e {
                InputEvent::PadDownOn { pad, button } => println!("{t:6.2}  pad {pad}  {button:?} down"),
                InputEvent::PadUpOn { pad, button } => println!("{t:6.2}  pad {pad}  {button:?} up"),
                InputEvent::PadMovedOn { pad, axis, value } => {
                    let before = last_axis.get(&(*pad, *axis)).copied().unwrap_or(0.0);
                    let edge = (*value == 0.0) != (before == 0.0) || value.abs() >= 0.999;
                    if (value - before).abs() >= 0.05 || edge {
                        last_axis.insert((*pad, *axis), *value);
                        println!("{t:6.2}  pad {pad}  {axis:?} {value:+.2}");
                    }
                }
                InputEvent::PadMotion { pad, motion } if last_motion.elapsed() > Duration::from_millis(500) => {
                    last_motion = Instant::now();
                    let g = motion.gravity;
                    let r = motion.rotation_rate;
                    println!("{t:6.2}  pad {pad}  gravity {:+.2} {:+.2} {:+.2}  rotation {:+.2} {:+.2} {:+.2}", g[0], g[1], g[2], r[0], r[1], r[2]);
                }
                InputEvent::PadConnected(pad) => println!("{t:6.2}  pad {pad}  connected"),
                InputEvent::PadDisconnected(pad) => println!("{t:6.2}  pad {pad}  disconnected"),
                InputEvent::PadInfo { pad, info } => println!("{t:6.2}  pad {pad}  {}", info.summary()),
                _ => {}
            }
        }
    }
    // Leave the pads as found: triggers free, sensors off, the players' colours.
    let mut rest = PadFeedback::new();
    for &slot in &slots_on {
        rest.set_trigger_effect(slot, TriggerSide::Left, TriggerEffect::Off);
        rest.set_trigger_effect(slot, TriggerSide::Right, TriggerEffect::Off);
        rest.set_motion(slot, false);
        rest.rumble(slot, 0.0, 0.0, 0.0);
    }
    source.apply(&slots, &mut rest, 1e6);
    source.pump(0.05);
    Ok(if listed.is_empty() { std::process::ExitCode::FAILURE } else { std::process::ExitCode::SUCCESS })
}
