//! A material's maps, found by asset id: a normal map tilts the surface the
//! way its channels say — red towards +u, green towards the top of the
//! image — and a mask's alpha takes the shine away.

use runity::asset::{AssetId, AssetKind, TextureAsset};
use runity::glam::{Mat4, Vec3};
use runity::render::{Camera, Draw, Frame, Lighting, ShadowSettings, Sky, SkyMode, TextureHandle};
use runity::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 32;

/// One colour everywhere, as an imported texture asset would be.
fn texture(id: u128, rgba: [u8; 4], srgb: bool) -> Vec<u8> {
    let asset = TextureAsset {
        id: AssetId(id),
        name: format!("t{id}"),
        width: 4,
        height: 4,
        pixels: rgba.repeat(16),
        mips: Vec::new(),
        srgb,
    };
    runity::asset::to_bytes(&asset, AssetKind::Texture).unwrap()
}

/// How bright the middle of a floor is, lit from `sun` (the way the light
/// travels), with `material`.
fn brightness(gpu: &Gpu, sun: Vec3, material: Material, maps: &[Vec<u8>]) -> u32 {
    let target = OffscreenTarget::new(gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(gpu, &target);
    for bytes in maps {
        renderer.upload_texture(gpu, runity::asset::view::<TextureAsset>(bytes).unwrap());
    }
    let floor = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post: runity::post::PostProcess::OFF,
        ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
        camera: Camera {
            position: Vec3::new(0.0, 5.0, 0.01),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        lighting: Lighting {
            sun_direction: sun.normalize(),
            sun_intensity: 1.0,
            sky_color: Vec3::ZERO,
            ground_color: Vec3::ZERO,
            ..Lighting::default()
        },
        shadows: ShadowSettings::OFF,
        clear_color: Vec3::ZERO,
        draws: vec![Draw {
            mesh: floor,
            transform: Mat4::from_scale(Vec3::new(10.0, 1.0, 10.0)),
            texture: TextureHandle::WHITE,
            material,
            pose: None,
        }],
        ..Frame::default()
    };
    renderer.render(gpu, &target, &frame);
    let p = OffscreenTarget::pixel(&target.read_rgba(gpu), SIZE, SIZE / 2, SIZE / 2);
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

#[test]
fn a_normal_map_tilts_the_surface_the_way_its_channels_say() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let plain = Material::new(0.6, 0.6, 0.6);
    let with = |map: u128| Material {
        normal_map: Some(AssetId(map)),
        ..plain
    };
    // Red up: leaning towards +u, which on the plane is +x.
    let red = texture(1, [230, 128, 200, 255], false);
    // Green up: leaning towards the top of the image, v = 0, which is -z.
    let green = texture(2, [128, 230, 200, 255], false);
    // A low sun in +x, and one in -z.
    let from_x = Vec3::new(-1.0, -0.6, 0.0);
    let from_minus_z = Vec3::new(0.0, -0.6, 1.0);
    let flat_x = brightness(&gpu, from_x, plain, &[]);
    let tilted_x = brightness(&gpu, from_x, with(1), &[red]);
    assert!(
        tilted_x > flat_x + 20,
        "towards +x, facing the sun there: {flat_x} -> {tilted_x}"
    );
    let flat_z = brightness(&gpu, from_minus_z, plain, &[]);
    let tilted_z = brightness(&gpu, from_minus_z, with(2), &[green]);
    assert!(
        tilted_z > flat_z + 20,
        "towards -z, facing the sun there: {flat_z} -> {tilted_z}"
    );
    // A map the renderer never got draws as no map.
    assert_eq!(brightness(&gpu, from_x, with(99), &[]), flat_x);
}

#[test]
fn a_mask_takes_the_shine_away() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    // A mirror-smooth metal, lit so the camera sees the sun's highlight.
    let shiny = Material {
        metallic: 1.0,
        smoothness: 1.0,
        ..Material::new(0.9, 0.9, 0.9)
    };
    let dull = Material {
        mask_map: Some(AssetId(3)),
        ..shiny
    };
    let sun = Vec3::new(0.0, -1.0, 0.0);
    let mask = texture(3, [255, 255, 255, 0], false);
    let bright = brightness(&gpu, sun, shiny, &[]);
    let masked = brightness(&gpu, sun, dull, &[mask]);
    assert!(
        bright > masked + 30,
        "smoothness times the mask's alpha: {bright} -> {masked}"
    );
}
