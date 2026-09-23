//! A shader edited while the renderer runs.
//!
//! DNA, postulate 1: shaders reload too, and a broken file does not take
//! the picture down. The edit here is the kind people make — the colour a
//! surface comes out — and the breakage is the kind they make: a typo
//! saved halfway through.

use runity::glam::{Mat4, Vec3};
use runity::render::{
    Camera, Draw, FogSettings, Frame, Lighting, ShadowSettings, TextureHandle, SHADER,
};
use runity::{builtin, Gpu, OffscreenTarget, Renderer};

const SIZE: u32 = 32;

fn centre(gpu: &Gpu, renderer: &mut Renderer, target: &OffscreenTarget) -> [u8; 4] {
    let mesh = renderer.upload_mesh_owned(gpu, &builtin::cube(2.0));
    let frame = Frame {
        // Counted in exact colours: no sky, no post-processing.
        sky: runity::render::Sky {
            mode: runity::render::SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        ray_tracing: Default::default(),
        camera: Camera {
            position: Vec3::new(0.0, 0.0, 4.0),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        lighting: Lighting::default(),
        fog: FogSettings {
            start: 100.0,
            end: 200.0,
            ..FogSettings::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        lights: Vec::new(),
        flares: Vec::new(),
        live_meshes: Vec::new(),
        texture_views: Vec::new(),
        ui_pictures: Vec::new(),
        reflection_probes: Vec::new(),
        decals: Vec::new(),
        volumetric_fog: Default::default(),
        wind: Default::default(),
        benders: Vec::new(),
        time: None,
        weather: Default::default(),
        screen_space_reflections: Default::default(),
        puffs: Vec::new(),
        draws: vec![Draw {
            mesh,
            transform: Mat4::IDENTITY,
            texture: TextureHandle::WHITE,
            material: runity::Material::new(0.5, 0.5, 0.5),
            pose: None,
        }],
        overlay_draws: Vec::new(),
        poses: Vec::new(),
    };
    renderer.render(gpu, target, &frame);
    OffscreenTarget::pixel(&target.read_rgba(gpu), SIZE, SIZE / 2, SIZE / 2)
}

#[test]
fn an_edited_shader_draws_the_next_frame_and_a_broken_one_does_not_draw_at_all() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let before = centre(&gpu, &mut renderer, &target);

    // Everything magenta: the edit someone makes to see that it took.
    let ret = "    return vec4<f32>(out, alpha);";
    assert!(SHADER.contains(ret), "the test knows where fs returns");
    let magenta = SHADER.replace(ret, "    return vec4<f32>(1.0, 0.0, 1.0, 1.0);");
    renderer.reload_shader(&gpu, &magenta).unwrap();
    let after = centre(&gpu, &mut renderer, &target);
    assert!(
        after[0] > 200 && after[1] < 40 && after[2] > 200,
        "{after:?} (was {before:?})"
    );

    // Half a line saved: said where, and the magenta shader keeps drawing.
    let broken = magenta.replace(
        "return vec4<f32>(1.0, 0.0, 1.0, 1.0);",
        "return vec4<f32>(1.0, 0.0,",
    );
    let err = renderer.reload_shader(&gpu, &broken).unwrap_err();
    assert!(err.contains(':'), "a place in the file: {err}");
    assert_eq!(centre(&gpu, &mut renderer, &target), after);

    // Compiles, but is not the renderer's shader: refused too.
    let err = renderer
        .reload_shader(
            &gpu,
            "@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }",
        )
        .unwrap_err();
    assert!(err.contains("does not fit"), "{err}");
    assert_eq!(centre(&gpu, &mut renderer, &target), after);
}
