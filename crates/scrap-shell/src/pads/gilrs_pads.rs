//! Pads through gilrs: Windows, Linux, the browser's Gamepad API, and a
//! Mac when `SCRAP_PADS=gilrs` asks. Buttons, sticks and analog triggers;
//! no light, rumble or trigger effects yet (a game's asks are no-ops).

use super::{Backend, Listed, PadSource};
use scrap_core::input::{InputEvent, PadAxis, PadButton, PadCaps, PadInfo, PadKind, PadSlots};

pub struct GilrsPads {
    gilrs: gilrs::Gilrs,
}

impl GilrsPads {
    pub fn new() -> Result<Self, String> {
        gilrs::Gilrs::new().map(|gilrs| Self { gilrs }).map_err(|e| e.to_string())
    }

    fn info(&self, id: gilrs::GamepadId) -> PadInfo {
        let pad = self.gilrs.gamepad(id);
        let name = pad.name().to_string();
        let backend = if cfg!(target_arch = "wasm32") { "web" } else { "gilrs" };
        PadInfo {
            kind: PadKind::from_name(&name),
            name,
            backend: backend.to_string(),
            caps: PadCaps { analog_triggers: true, ..PadCaps::default() },
            battery: None,
        }
    }
}

impl PadSource for GilrsPads {
    fn backend(&self) -> Backend {
        Backend::Gilrs
    }

    fn poll(&mut self, slots: &mut PadSlots, live: bool, out: &mut Vec<InputEvent>) -> bool {
        // Pads in before the loop began say nothing of it: they take
        // their slots first, in the platform's order.
        let present: Vec<gilrs::GamepadId> = self.gilrs.gamepads().map(|(id, _)| id).collect();
        for id in present {
            let device = usize::from(id) as u64;
            if slots.slot(device).is_none() {
                let slot = slots.connect(device);
                out.push(InputEvent::PadConnected(slot));
                out.push(InputEvent::PadInfo { pad: slot, info: self.info(id) });
            }
        }
        let mut pressed = false;
        while let Some(event) = self.gilrs.next_event() {
            let device = usize::from(event.id) as u64;
            let connecting = matches!(event.event, gilrs::EventType::Connected) && slots.slot(device).is_none();
            if !live && !matches!(event.event, gilrs::EventType::Connected | gilrs::EventType::Disconnected) {
                continue;
            }
            pressed |= matches!(event.event, gilrs::EventType::ButtonPressed(..));
            let said = pad_events(slots, device, event.event);
            if connecting {
                if let Some(InputEvent::PadConnected(slot)) = said.first() {
                    let slot = *slot;
                    out.extend(said);
                    out.push(InputEvent::PadInfo { pad: slot, info: self.info(event.id) });
                    continue;
                }
            }
            out.extend(said);
        }
        pressed
    }

    fn list(&self, slots: &PadSlots) -> Vec<Listed> {
        self.gilrs
            .gamepads()
            .filter_map(|(id, _)| {
                let slot = slots.slot(usize::from(id) as u64)?;
                Some(Listed { slot, info: self.info(id), elements: Vec::new() })
            })
            .collect()
    }
}

/// A gilrs button as ours, by where it is on the pad.
pub(crate) fn pad_button(b: gilrs::Button) -> Option<PadButton> {
    use gilrs::Button;
    Some(match b {
        Button::South => PadButton::South,
        Button::East => PadButton::East,
        Button::West => PadButton::West,
        Button::North => PadButton::North,
        // gilrs calls the bumpers triggers and the triggers "2".
        Button::LeftTrigger => PadButton::LeftBumper,
        Button::RightTrigger => PadButton::RightBumper,
        Button::LeftTrigger2 => PadButton::LeftTrigger,
        Button::RightTrigger2 => PadButton::RightTrigger,
        Button::Select => PadButton::Select,
        Button::Start => PadButton::Start,
        Button::Mode => PadButton::Home,
        Button::LeftThumb => PadButton::LeftStick,
        Button::RightThumb => PadButton::RightStick,
        Button::DPadUp => PadButton::DPadUp,
        Button::DPadDown => PadButton::DPadDown,
        Button::DPadLeft => PadButton::DPadLeft,
        Button::DPadRight => PadButton::DPadRight,
        _ => return None,
    })
}

/// A gilrs stick axis as ours; gilrs already has up and right positive.
pub(crate) fn pad_stick(a: gilrs::Axis) -> Option<PadAxis> {
    use gilrs::Axis;
    Some(match a {
        Axis::LeftStickX => PadAxis::LeftX,
        Axis::LeftStickY => PadAxis::LeftY,
        Axis::RightStickX => PadAxis::RightX,
        Axis::RightStickY => PadAxis::RightY,
        _ => return None,
    })
}

/// Turn one gilrs event into ours: triggers as both a value and a button.
pub(crate) fn translate_pad(event: gilrs::EventType) -> Option<InputEvent> {
    use gilrs::{Button, EventType};
    match event {
        EventType::ButtonPressed(b, _) => pad_button(b).map(InputEvent::PadDown),
        EventType::ButtonReleased(b, _) => pad_button(b).map(InputEvent::PadUp),
        EventType::ButtonChanged(Button::LeftTrigger2, value, _) => Some(InputEvent::PadMoved { axis: PadAxis::LeftTrigger, value }),
        EventType::ButtonChanged(Button::RightTrigger2, value, _) => Some(InputEvent::PadMoved { axis: PadAxis::RightTrigger, value }),
        EventType::AxisChanged(axis, value, _) => pad_stick(axis).map(|axis| InputEvent::PadMoved { axis, value }),
        _ => None,
    }
}

/// One gilrs event of a device as ours, by the player slot the device is
/// in ([`PadSlots`]): a pad coming or going, or its button or stick on its
/// slot — which the merged pad sees as well.
pub(crate) fn pad_events(slots: &mut PadSlots, device: u64, event: gilrs::EventType) -> Vec<InputEvent> {
    use gilrs::EventType;
    match event {
        EventType::Connected => vec![InputEvent::PadConnected(slots.connect(device))],
        EventType::Disconnected => slots.disconnect(device).map(InputEvent::PadDisconnected).into_iter().collect(),
        other => {
            let pad = slots.slot(device).unwrap_or_else(|| slots.connect(device));
            translate_pad(other).and_then(|e| on_slot(pad, e)).into_iter().collect()
        }
    }
}

/// A pad's event as one of slot `pad`'s.
pub(crate) fn on_slot(pad: usize, event: InputEvent) -> Option<InputEvent> {
    match event {
        InputEvent::PadDown(button) => Some(InputEvent::PadDownOn { pad, button }),
        InputEvent::PadUp(button) => Some(InputEvent::PadUpOn { pad, button }),
        InputEvent::PadMoved { axis, value } => Some(InputEvent::PadMovedOn { pad, axis, value }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gilrs::{Axis, Button, EventType};

    #[test]
    fn a_pad_is_read_by_position() {
        assert_eq!(pad_button(Button::South), Some(PadButton::South));
        assert_eq!(pad_button(Button::LeftTrigger), Some(PadButton::LeftBumper));
        assert_eq!(pad_button(Button::RightTrigger2), Some(PadButton::RightTrigger));
        assert_eq!(pad_stick(Axis::LeftStickY), Some(PadAxis::LeftY));
        assert_eq!(pad_stick(Axis::LeftZ), None);
        assert_eq!(translate_pad(EventType::Connected), None);
    }

    #[test]
    fn each_pad_reports_on_its_own_slot() {
        let mut slots = PadSlots::new();
        assert_eq!(pad_events(&mut slots, 7, EventType::Connected), [InputEvent::PadConnected(0)]);
        assert_eq!(pad_events(&mut slots, 3, EventType::Connected), [InputEvent::PadConnected(1)]);
        assert_eq!(on_slot(1, InputEvent::PadDown(PadButton::South)), Some(InputEvent::PadDownOn { pad: 1, button: PadButton::South }));
        assert_eq!(pad_events(&mut slots, 7, EventType::Disconnected), [InputEvent::PadDisconnected(0)]);
        assert_eq!(pad_events(&mut slots, 7, EventType::Disconnected), [], "gone already");
    }
}
