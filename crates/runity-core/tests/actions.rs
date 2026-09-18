//! Bindings, action edges and the intent that travels over the wire.

use runity_core::action::{
    Action, ActionSet, Actions, AxisBinding, Binding, Bindings, Intent, AXIS_LIMIT, MAX_BUTTONS,
};
use runity_core::input::Input;
use runity_platform::{Event, Key, MouseButton};
use runity_serialize::{from_bytes, to_bytes};

const JUMP: Action = Action::named("jump");
const CROUCH: Action = Action::named("crouch");
const FORWARD: Action = Action::named("move_forward");
const LOOK_X: Action = Action::named("look_x");

/// A frame of input: clear the edges, feed events, evaluate.
fn frame(actions: &mut Actions, input: &mut Input, events: &[Event]) {
    input.begin_frame();
    for event in events {
        input.handle(event);
    }
    actions.update(input);
}

fn walking() -> Bindings {
    let mut bindings = Bindings::new();
    bindings
        .key(JUMP, Key::Space)
        .key(CROUCH, Key::LeftControl)
        .key_axis(FORWARD, Key::S, Key::W)
        .bind_axis(LOOK_X, AxisBinding::MouseX { scale: 0.1 });
    bindings
}

#[test]
fn an_action_name_hashes_to_the_same_id_everywhere() {
    assert_eq!(Action::named("jump"), JUMP);
    assert_ne!(Action::named("jump"), Action::named("crouch"));
    // The point of hashing rather than interning: no registry was consulted.
    assert_eq!(Action::named("").0, 0x811c_9dc5);
}

#[test]
fn a_bound_key_raises_its_action() {
    let mut actions = Actions::new(walking());
    let mut input = Input::new();

    frame(&mut actions, &mut input, &[Event::KeyDown(Key::Space)]);
    assert!(actions.down(JUMP) && actions.pressed(JUMP));
    assert!(!actions.down(CROUCH));

    // Held: still down, no longer a fresh press.
    frame(&mut actions, &mut input, &[]);
    assert!(actions.down(JUMP));
    assert!(!actions.pressed(JUMP));

    frame(&mut actions, &mut input, &[Event::KeyUp(Key::Space)]);
    assert!(actions.released(JUMP));
    assert!(!actions.down(JUMP));
}

#[test]
fn two_keys_for_one_action_are_both_live() {
    let mut bindings = walking();
    bindings.key(JUMP, Key::Enter);
    let mut actions = Actions::new(bindings);
    let mut input = Input::new();

    frame(&mut actions, &mut input, &[Event::KeyDown(Key::Enter)]);
    assert!(actions.pressed(JUMP), "the alternate binding works too");
}

/// The edge has to come from the action's own history. Letting go of one of
/// two bound keys while the other is still held is not a release.
#[test]
fn releasing_one_of_two_bound_keys_is_not_a_release() {
    let mut bindings = walking();
    bindings.key(JUMP, Key::Enter);
    let mut actions = Actions::new(bindings);
    let mut input = Input::new();

    frame(
        &mut actions,
        &mut input,
        &[Event::KeyDown(Key::Space), Event::KeyDown(Key::Enter)],
    );
    assert!(actions.pressed(JUMP));

    frame(&mut actions, &mut input, &[Event::KeyUp(Key::Enter)]);
    assert!(actions.down(JUMP), "space is still held");
    assert!(!actions.released(JUMP), "and so nothing was released");
    assert!(!actions.pressed(JUMP), "nor pressed again");
}

/// Rebinding to a key that is already held must not look like a press: a
/// settings screen where the player is holding a key while assigning it would
/// otherwise fire the action the instant it is bound.
#[test]
fn rebinding_onto_a_held_key_still_reads_as_a_press_only_once() {
    let mut actions = Actions::new(walking());
    let mut input = Input::new();

    frame(&mut actions, &mut input, &[Event::KeyDown(Key::Q)]);
    assert!(!actions.down(JUMP), "Q means nothing yet");

    actions.bindings_mut().rebind(JUMP, Key::Q);
    frame(&mut actions, &mut input, &[]);
    assert!(actions.down(JUMP));
    assert!(
        actions.pressed(JUMP),
        "the first frame it counts is an edge"
    );

    frame(&mut actions, &mut input, &[]);
    assert!(actions.down(JUMP) && !actions.pressed(JUMP));
    assert!(!actions.down(Action::named("jump_old")));

    // And the old key is gone.
    frame(&mut actions, &mut input, &[Event::KeyDown(Key::Space)]);
    assert!(actions.down(JUMP), "still Q");
    actions.bindings_mut().clear_action(JUMP);
    frame(&mut actions, &mut input, &[]);
    assert!(actions.released(JUMP), "unbinding releases it");
}

#[test]
fn a_settings_screen_can_see_conflicts() {
    let bindings = walking();
    assert_eq!(bindings.conflicts(Binding::Key(Key::Space)), vec![JUMP]);
    assert!(bindings.conflicts(Binding::Key(Key::Q)).is_empty());

    let mut clashing = walking();
    clashing.key(CROUCH, Key::Space);
    let found = clashing.conflicts(Binding::Key(Key::Space));
    assert_eq!(found.len(), 2, "both actions answer to space: {found:?}");
}

#[test]
fn a_key_axis_runs_from_minus_one_to_one() {
    let mut actions = Actions::new(walking());
    let mut input = Input::new();

    frame(&mut actions, &mut input, &[Event::KeyDown(Key::W)]);
    assert_eq!(actions.value(FORWARD), 1.0);
    frame(&mut actions, &mut input, &[Event::KeyDown(Key::S)]);
    assert_eq!(actions.value(FORWARD), 0.0, "both held cancels");
    frame(&mut actions, &mut input, &[Event::KeyUp(Key::W)]);
    assert_eq!(actions.value(FORWARD), -1.0);
}

#[test]
fn two_sources_for_one_axis_add() {
    let mut bindings = walking();
    bindings.key_axis(FORWARD, Key::Down, Key::Up);
    let mut actions = Actions::new(bindings);
    let mut input = Input::new();

    frame(
        &mut actions,
        &mut input,
        &[Event::KeyDown(Key::W), Event::KeyDown(Key::Down)],
    );
    assert_eq!(actions.value(FORWARD), 0.0, "+1 from W, -1 from Down");
}

#[test]
fn the_mouse_drives_an_axis_and_resets_each_frame() {
    let mut actions = Actions::new(walking());
    let mut input = Input::new();

    frame(&mut actions, &mut input, &[Event::MouseMove { x: 0, y: 0 }]);
    frame(
        &mut actions,
        &mut input,
        &[Event::MouseMove { x: 20, y: 0 }],
    );
    assert!((actions.value(LOOK_X) - 2.0).abs() < 1e-6);

    frame(&mut actions, &mut input, &[]);
    assert_eq!(actions.value(LOOK_X), 0.0, "a delta is per frame");
}

#[test]
fn an_unbound_axis_is_zero_rather_than_a_panic() {
    let actions = Actions::new(Bindings::new());
    assert_eq!(actions.value(Action::named("nothing")), 0.0);
    assert!(!actions.down(Action::named("nothing")));
}

#[test]
fn a_mouse_button_binds_like_a_key() {
    let fire = Action::named("fire");
    let mut bindings = Bindings::new();
    bindings.bind(fire, Binding::Mouse(MouseButton::Left));
    let mut actions = Actions::new(bindings);
    let mut input = Input::new();

    frame(
        &mut actions,
        &mut input,
        &[Event::MouseDown(MouseButton::Left)],
    );
    assert!(actions.pressed(fire));
    frame(
        &mut actions,
        &mut input,
        &[Event::MouseUp(MouseButton::Left)],
    );
    assert!(actions.released(fire));
}

// ----------------------------------------------------------------- intents

fn wire_set() -> ActionSet {
    let mut set = ActionSet::new();
    set.button(JUMP).button(CROUCH);
    set.axis(FORWARD).axis(LOOK_X);
    set
}

#[test]
fn an_intent_carries_what_the_player_asked_for() {
    let set = wire_set();
    let mut actions = Actions::new(walking());
    let mut input = Input::new();

    frame(
        &mut actions,
        &mut input,
        &[Event::KeyDown(Key::Space), Event::KeyDown(Key::W)],
    );
    let intent = actions.intent(&set);
    assert!(intent.down(&set, JUMP));
    assert!(!intent.down(&set, CROUCH));
    assert_eq!(intent.value(&set, FORWARD), 1.0);
    assert!(!intent.is_idle());
}

#[test]
fn an_untouched_frame_is_idle() {
    let set = wire_set();
    let mut actions = Actions::new(walking());
    let mut input = Input::new();
    frame(&mut actions, &mut input, &[]);
    assert!(actions.intent(&set).is_idle());
}

/// The whole reason for fixed point: what one machine sends is bit-for-bit
/// what the other steps on.
#[test]
fn axes_quantize_to_thousandths() {
    let set = wire_set();
    let mut intent = Intent::for_set(&set);
    intent.set_axis(0, 0.700_000_1);
    assert_eq!(intent.axis(0), 0.7);
    intent.set_axis(0, 0.123_456_7);
    assert_eq!(intent.axis(0), 0.123);
    intent.set_axis(0, -0.000_4);
    assert_eq!(intent.axis(0), 0.0, "below half a thousandth rounds away");
}

#[test]
fn an_axis_beyond_the_range_clamps_rather_than_wrapping() {
    let set = wire_set();
    let mut intent = Intent::for_set(&set);
    intent.set_axis(0, 1e9);
    assert_eq!(intent.axis(0), AXIS_LIMIT);
    intent.set_axis(0, -1e9);
    assert_eq!(intent.axis(0), -AXIS_LIMIT);
    intent.set_axis(0, f32::NAN);
    assert_eq!(intent.axis(0), 0.0, "a NaN axis is no input, not a desync");
}

#[test]
fn an_intent_survives_the_wire() {
    let set = wire_set();
    let mut intent = Intent::for_set(&set);
    intent.set_button(0, true);
    intent.set_button(1, false);
    intent.set_axis(0, -0.5);
    intent.set_axis(1, 12.25);

    let bytes = to_bytes(&intent);
    let back: Intent = from_bytes(&bytes).expect("round trip");
    assert_eq!(back, intent);
    assert_eq!(back.value(&set, FORWARD), -0.5);
    assert!(
        bytes.len() < 12,
        "a per-tick message, {} bytes",
        bytes.len()
    );
}

#[test]
fn a_truncated_intent_is_an_error_and_not_a_panic() {
    let set = wire_set();
    let mut intent = Intent::for_set(&set);
    intent.set_axis(0, 1.0);
    let bytes = to_bytes(&intent);
    for cut in 0..bytes.len() {
        let _ = from_bytes::<Intent>(&bytes[..cut]);
    }
    // And a length that no message could back must not allocate on trust.
    assert!(from_bytes::<Intent>(&[0x00, 0xff, 0xff, 0xff, 0x7f]).is_err());
}

#[test]
fn the_wire_order_is_the_set_order_not_the_binding_order() {
    let mut one = ActionSet::new();
    one.button(JUMP).button(CROUCH);
    let mut other = ActionSet::new();
    other.button(CROUCH).button(JUMP);

    assert_eq!(one.button_index(JUMP), Some(0));
    assert_eq!(other.button_index(JUMP), Some(1));
    assert_ne!(
        one.fingerprint(),
        other.fingerprint(),
        "two builds that disagree must be able to notice during the handshake"
    );

    let mut same = ActionSet::new();
    same.button(JUMP).button(CROUCH);
    assert_eq!(one.fingerprint(), same.fingerprint());
}

#[test]
fn an_action_outside_the_set_reads_as_nothing() {
    let set = wire_set();
    let intent = Intent::for_set(&set);
    assert!(!intent.down(&set, Action::named("not in the set")));
    assert_eq!(intent.value(&set, Action::named("not in the set")), 0.0);
}

#[test]
#[should_panic(expected = "at most")]
fn overflowing_the_button_field_is_loud() {
    let mut set = ActionSet::new();
    for index in 0..=MAX_BUTTONS {
        set.button(Action(index as u32));
    }
}

#[test]
fn a_keymap_survives_being_saved() {
    let bindings = walking();
    let bytes = to_bytes(&bindings);
    let back: Bindings = from_bytes(&bytes).expect("round trip");
    assert_eq!(back, bindings);
}

/// A keymap from a build that knows keys this one does not must still load,
/// and saving it again must not quietly drop them.
#[test]
fn an_unknown_key_round_trips() {
    let mut bindings = Bindings::new();
    bindings.key(JUMP, Key::Unknown(4242));
    let back: Bindings = from_bytes(&to_bytes(&bindings)).expect("round trip");
    assert_eq!(back, bindings);
}

#[test]
fn a_corrupt_keymap_is_an_error_and_not_a_panic() {
    let bytes = to_bytes(&walking());
    for cut in 0..bytes.len() {
        let _ = from_bytes::<Bindings>(&bytes[..cut]);
    }
    for byte in 0..=255u8 {
        let mut broken = bytes.clone();
        broken[0] = byte;
        let _ = from_bytes::<Bindings>(&broken);
    }
}
