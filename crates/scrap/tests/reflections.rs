//! Reflection probes: a mirror-smooth metal floor beside a red wall shows
//! the wall in it with a probe around them, and the empty sky without.

use scrap::glam::{Mat4, Vec3};
use scrap::material::Shading;
use scrap::reflections::ReflectionProbe;
use scrap::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 64;

#[test]
fn a_metal_floor_reflects_the_wall_beside_it_with_a_probe() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let plane = renderer.upload_mesh_owned(&gpu, &builtin::plane(1.0, 1));
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let mirror = Material {
        metallic: 1.0,
        smoothness: 1.0,
        ..Material::new(0.95, 0.95, 0.95)
    };
    let red = Material {
        shading: Shading::Unlit,
        ..Material::new(1.0, 0.0, 0.0)
    };
    let draws = vec![
        Draw {
            mesh: plane,
            transform: Mat4::from_scale(Vec3::new(10.0, 1.0, 10.0)),
            texture: TextureHandle::WHITE,
            material: mirror,
            pose: None,
        },
        // A red wall along z, at x = 3.
        Draw {
            mesh: cube,
            transform: Mat4::from_translation(Vec3::new(3.0, 2.0, 0.0))
                * Mat4::from_scale(Vec3::new(0.2, 4.0, 10.0)),
            texture: TextureHandle::WHITE,
            material: red,
            pose: None,
        },
    ];
    let frame = |probes: Vec<ReflectionProbe>| Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: scrap::post::PostProcess::OFF,
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
        // Looking down at the floor, towards the wall: what the floor
        // reflects there is the wall's foot.
        camera: Camera {
            position: Vec3::new(0.0, 1.5, 0.0),
            target: Vec3::new(1.5, 0.0, 0.0),
            ..Camera::default()
        },
        lighting: Lighting {
            sun_intensity: 0.0,
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        draws: draws.clone(),
        reflection_probes: probes,
        ..Frame::default()
    };
    let red_at_middle = |renderer: &mut Renderer, probes| {
        renderer.render(&gpu, &target, &frame(probes));
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)[0]
    };
    let without = red_at_middle(&mut renderer, Vec::new());
    let probe = ReflectionProbe {
        position: Vec3::new(0.0, 1.0, 0.0),
        extents: Vec3::new(5.0, 3.0, 5.0),
        box_projection: true,
        blend_distance: 0.5,
    };
    let with = red_at_middle(&mut renderer, vec![probe]);
    assert!(
        without < 10,
        "no probe: the floor reflects the empty sky: {without}"
    );
    assert!(with > 80, "with a probe it reflects the wall: {with}");
    // Taken away, it is the sky again.
    let again = red_at_middle(&mut renderer, Vec::new());
    assert!(again < 10, "{again}");
}
