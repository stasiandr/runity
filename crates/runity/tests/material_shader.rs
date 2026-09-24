//! A material with its own shader: a `surface` function over the standard
//! one, set while the renderer runs, refused in words when broken, and
//! kept when the standard shader is reloaded.

use runity::glam::{Mat4, Vec3};
use runity::material::Shading;
use runity::render::{Camera, Draw, Frame, SkyMode, TextureHandle, SHADER};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 32;

const RED: &str = "
fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    o.albedo = vec3<f32>(1.0, 0.0, 0.0);
    return o;
}
";

#[test]
fn a_material_draws_with_its_own_surface_function() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(2.0));
    let id = runity::asset::shader_id("red");
    let frame = Frame {
        camera: Camera {
            position: Vec3::new(0.0, 0.0, 4.0),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        sky: runity::render::Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        clear_color: Vec3::ZERO,
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        draws: vec![Draw {
            mesh: cube,
            transform: Mat4::IDENTITY,
            texture: TextureHandle::WHITE,
            material: Material {
                shading: Shading::Unlit,
                shader: Some(id),
                ..Material::new(1.0, 1.0, 1.0)
            },
            pose: None,
        }],
        ..Frame::default()
    };
    let middle = |renderer: &mut Renderer| {
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let before = middle(&mut renderer);
    assert!(
        before[1] > 200,
        "not in yet: the standard shader, white: {before:?}"
    );
    renderer.set_material_shader(&gpu, id, RED).unwrap();
    let red = middle(&mut renderer);
    assert!(red[0] > 200 && red[1] < 40, "its own: red {red:?}");
    let broken = renderer.set_material_shader(
        &gpu,
        id,
        "fn surface(in: SurfaceIn, out: Surface) -> Surface { return oops; }",
    );
    let words = broken.expect_err("a typo is refused");
    assert!(words.contains("oops"), "{words}");
    assert_eq!(
        middle(&mut renderer),
        red,
        "and the last one that built keeps drawing"
    );
    renderer.reload_shader(&gpu, SHADER).unwrap();
    assert_eq!(
        middle(&mut renderer),
        red,
        "built again on a reloaded standard shader"
    );
}

#[test]
fn a_materials_own_numbers_reach_its_shader() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(2.0));
    let id = runity::asset::shader_id("tint");
    renderer
        .set_material_shader(
            &gpu,
            id,
            "// runity:params _Tint.r _Tint.g _Tint.b
fn surface(in: SurfaceIn, out: Surface) -> Surface {
    var o = out;
    o.albedo = in.params[0].xyz;
    return o;
}",
        )
        .unwrap();
    let draw = |params: [f32; 8]| Draw {
        mesh: cube,
        transform: Mat4::IDENTITY,
        texture: TextureHandle::WHITE,
        material: Material {
            shading: Shading::Unlit,
            shader: Some(id),
            params,
            ..Material::new(1.0, 1.0, 1.0)
        },
        pose: None,
    };
    let mut middle = |params| {
        let frame = Frame {
            camera: Camera {
                position: Vec3::new(0.0, 0.0, 4.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            sky: runity::render::Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            clear_color: Vec3::ZERO,
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            draws: vec![draw(params)],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let green = middle([0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let blue = middle([0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert!(green[1] > 200 && green[2] < 40, "{green:?}");
    assert!(
        blue[2] > 200 && blue[1] < 40,
        "one shader, two materials: {blue:?}"
    );
}

#[test]
fn a_marker_on_top_shows_through_a_wall() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let mut behind = |on_top: bool| {
        let wall = Draw {
            mesh: cube,
            transform: Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0))
                * Mat4::from_scale(Vec3::new(3.0, 3.0, 0.1)),
            texture: TextureHandle::WHITE,
            material: Material {
                shading: Shading::Unlit,
                ..Material::new(0.0, 0.0, 1.0)
            },
            pose: None,
        };
        let marker = Draw {
            mesh: cube,
            transform: Mat4::IDENTITY,
            texture: TextureHandle::WHITE,
            material: Material {
                shading: Shading::Unlit,
                surface: runity::material::SurfaceType::Transparent,
                alpha: 0.9,
                on_top,
                ..Material::new(1.0, 0.0, 0.0)
            },
            pose: None,
        };
        let frame = Frame {
            camera: Camera {
                position: Vec3::new(0.0, 0.0, 4.0),
                target: Vec3::ZERO,
                ..Camera::default()
            },
            sky: runity::render::Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            clear_color: Vec3::ZERO,
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            draws: vec![wall, marker],
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
    };
    let hidden = behind(false);
    assert!(
        hidden[2] > 200 && hidden[0] < 40,
        "behind the wall: the wall {hidden:?}"
    );
    let seen = behind(true);
    assert!(seen[0] > 180, "on top: through the wall {seen:?}");
}

/// One colour everywhere, as an imported texture asset would be.
fn texture(id: u128, rgba: [u8; 4]) -> Vec<u8> {
    use runity::asset::{AssetId, TextureAsset};
    let asset = TextureAsset {
        id: AssetId(id),
        name: format!("t{id}"),
        width: 4,
        height: 4,
        pixels: rgba.repeat(16),
        mips: Vec::new(),
        srgb: false,
    };
    runity::asset::to_bytes(&asset, runity::asset::TEXTURE).unwrap()
}

#[test]
fn a_materials_own_textures_reach_its_shader_in_the_slots_it_names() {
    use runity::asset::{AssetId, TextureAsset};
    let Ok(mut gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    // The shader reads `_Second` in its first slot and `_First` in its
    // second: the material's names go to the shader's slots by the
    // shader's order, not the material's. Its third slot it does not
    // name, and the material's `_Unused` it does not read.
    let reading = |slot: u32| {
        format!(
            "// runity:textures _Second _First
fn surface(in: SurfaceIn, out: Surface) -> Surface {{
    var o = out;
    o.albedo = texture_at(in, {slot}u, in.uv).rgb;
    return o;
}}"
        )
    };
    let textures = [
        texture(11, [0, 255, 0, 255]),
        texture(12, [0, 0, 255, 255]),
        texture(13, [255, 0, 0, 255]),
    ];
    let material = |id| Material {
        shading: Shading::Unlit,
        shader: Some(id),
        textures: runity::material::MaterialTextures::new([
            ("_First", AssetId(11)),
            ("_Second", AssetId(12)),
            ("_Unused", AssetId(13)),
        ]),
        ..Material::new(1.0, 1.0, 1.0)
    };
    // Both ways maps are bound: one array indexed by the instance's
    // handles, and a bind group a set of maps.
    let ways: &[bool] = if gpu.bindless { &[true, false] } else { &[false] };
    for &bindless in ways {
        gpu.bindless = bindless;
        let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
        let mut renderer = Renderer::new(&gpu, &target);
        for bytes in &textures {
            renderer.upload_texture(&gpu, runity::asset::view::<TextureAsset>(bytes).unwrap());
        }
        let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(2.0));
        let mut middle = |slot: u32| {
            let id = runity::asset::shader_id(&format!("slot{slot}"));
            renderer.set_material_shader(&gpu, id, &reading(slot)).unwrap();
            let frame = Frame {
                camera: Camera {
                    position: Vec3::new(0.0, 0.0, 4.0),
                    target: Vec3::ZERO,
                    ..Camera::default()
                },
                sky: runity::render::Sky {
                    mode: SkyMode::Color,
                    ..Default::default()
                },
                clear_color: Vec3::ZERO,
                post: runity::post::PostProcess::OFF,
                ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
                draws: vec![Draw {
                    mesh: cube,
                    transform: Mat4::IDENTITY,
                    texture: TextureHandle::WHITE,
                    material: material(id),
                    pose: None,
                }],
                ..Frame::default()
            };
            renderer.render(&gpu, &target, &frame);
            OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)
        };
        let first = middle(0);
        assert!(
            first[2] > 200 && first[0] < 40 && first[1] < 40,
            "slot 0 is `_Second`, blue (bindless {bindless}): {first:?}"
        );
        let second = middle(1);
        assert!(
            second[1] > 200 && second[0] < 40 && second[2] < 40,
            "slot 1 is `_First`, green (bindless {bindless}): {second:?}"
        );
        let unnamed = middle(2);
        assert!(
            unnamed.iter().take(3).all(|c| *c > 200),
            "a slot the shader does not name is white (bindless {bindless}): {unnamed:?}"
        );
    }
}

/// A cut-out's own shader (grass: a blade out of a card) discards where
/// the surface is not; the depth prepass, which does not run it, must not
/// lay the whole card — or what is behind the cut is hidden by nothing.
#[test]
fn what_an_own_shader_cuts_away_shows_what_is_behind() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let id = runity::asset::shader_id("cut_all");
    renderer
        .set_material_shader(
            &gpu,
            id,
            "fn surface(in: SurfaceIn, out: Surface) -> Surface {\n    if in.uv.x > -1.0 { discard; }\n    return out;\n}\n",
        )
        .unwrap();
    let draw = |transform: Mat4, material: Material| Draw {
        mesh: cube,
        transform,
        texture: TextureHandle::WHITE,
        material,
        pose: None,
    };
    let frame = Frame {
        camera: Camera {
            position: Vec3::new(0.0, 0.0, 4.0),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        sky: runity::render::Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        clear_color: Vec3::ZERO,
        // Occlusion reads the prepass, and TAA its depth: with them on,
        // the prepass runs and the scene starts from its depth.
        ambient_occlusion: runity::ssao::AmbientOcclusion {
            enabled: true,
            ..Default::default()
        },
        draws: vec![
            // The card in front, cut away everywhere.
            draw(
                Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0)),
                Material {
                    shading: Shading::Unlit,
                    shader: Some(id),
                    ..Material::new(1.0, 1.0, 1.0)
                },
            ),
            // Red behind it.
            draw(
                Mat4::from_scale(Vec3::splat(3.0)) * Mat4::from_translation(Vec3::new(0.0, 0.0, -1.0)),
                Material {
                    shading: Shading::Unlit,
                    ..Material::new(1.0, 0.0, 0.0)
                },
            ),
        ],
        ..Frame::default()
    };
    renderer.render(&gpu, &target, &frame);
    renderer.render(&gpu, &target, &frame);
    let middle = OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2);
    assert!(middle[0] > 200 && middle[1] < 40, "the red behind the cut: {middle:?}");
}
