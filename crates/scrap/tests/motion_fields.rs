//! A clip moves any number of a thing, not only its place: its material's,
//! its light's, its game components' (`scrap::motion::access`).

use scrap::motion::{access, Property};
use scrap::world::{LightSource, Surface};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Fan {
    speed: f32,
    spins: u32,
}

#[test]
fn a_material_number_is_moved_on_its_thing_alone() {
    let mut world = hecs::World::new();
    let lamp = world.spawn((Surface(scrap::material::Material::default()),));
    let other = world.spawn((Surface(scrap::material::Material::default()),));
    let glow = Property::Material("emission.r".into());
    assert_eq!(access(&world, lamp, &glow, Some(4.0)), Some(0.0));
    assert_eq!(world.get::<&Surface>(lamp).unwrap().0.emission[0], 4.0);
    assert_eq!(world.get::<&Surface>(other).unwrap().0.emission[0], 0.0, "the other's is its own");
    // A colour as a person picks it: sRGB in, linear kept.
    access(&world, lamp, &Property::Material("base_color.g".into()), Some(0.5));
    let kept = world.get::<&Surface>(lamp).unwrap().0.base_color[1];
    assert!((kept - 0.214).abs() < 0.01, "{kept}");
    let back = access(&world, lamp, &Property::Material("base_color.g".into()), None).unwrap();
    assert!((back - 0.5).abs() < 1e-3, "{back}");
    assert_eq!(access(&world, lamp, &Property::Material("nothing".into()), Some(1.0)), None);
}

#[test]
fn a_lights_and_a_game_components_numbers_are_moved_by_their_paths() {
    let mut world = hecs::World::new();
    let light: scrap::scene::Light = scrap::ron::from_str("(color: (1.0, 0.6, 0.3), intensity: 2.0, range: 6.0)").unwrap();
    scrap::numbers::register::<Fan>("fan");
    let e = world.spawn((LightSource(light), Fan { speed: 1.0, spins: 3 }));
    let intensity = Property::Field("light.intensity".into());
    let before = access(&world, e, &intensity, Some(7.5)).expect("a light has an intensity");
    assert_ne!(before, 7.5);
    assert_eq!(access(&world, e, &intensity, None), Some(7.5));
    let speed = Property::Field("components.fan.speed".into());
    assert_eq!(access(&world, e, &speed, Some(2.5)), Some(1.0));
    assert_eq!(*world.get::<&Fan>(e).unwrap(), Fan { speed: 2.5, spins: 3 }, "the rest of it kept");
    assert_eq!(access(&world, e, &Property::Field("camera.fov_deg".into()), Some(1.0)), None, "no camera here");
}

#[test]
fn a_module_says_which_of_its_parts_clips_can_move() {
    // Kept where its module reads it every frame: moved.
    for part in ["light", "camera", "sound", "particles", "decal", "rope", "ocean", "smoke", "route"] {
        assert!(scrap::motion::movable_part(part), "{part}");
    }
    // A body or a spline is built once from its line: not.
    for part in ["physics", "spline", "model"] {
        assert!(!scrap::motion::movable_part(part), "{part}");
    }
}

#[test]
fn a_clip_lays_the_scenes_fog_over_what_the_scene_says() {
    use scrap::look::SceneLook;
    let scene: scrap::scene::Scene = scrap::ron::from_str("(fog: (color: (0.5, 0.6, 0.7), start: 10.0, end: 80.0))").unwrap();
    let mut world = hecs::World::new();
    let dusk = world.spawn(());
    assert!(scrap::world::scene_looked(&world, &scene).is_none(), "no clip, the scene as it is");
    let end = Property::Field("scene.fog.end".into());
    scrap::motion::set(&mut world, dusk, &end, 30.0);
    let looked = scrap::world::scene_looked(&world, &scene).unwrap();
    assert_eq!(looked.fog().end, 30.0);
    assert_eq!(looked.fog().color, [0.5, 0.6, 0.7], "the rest as the scene says");
    assert_eq!(scene.fog().end, 80.0, "the scene's own untouched");
    assert_eq!(access(&world, dusk, &end, None), Some(30.0));
}
