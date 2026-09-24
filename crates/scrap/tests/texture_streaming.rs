//! Mip streaming: a 512² picture on a crate near the camera is all on the
//! GPU; walked away from, it gives up its finer levels, a level at a time;
//! walked back to, it has them again at once.

use scrap::asset::{AssetId, TextureAsset, TextureLevel};
use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

#[test]
fn a_texture_keeps_the_levels_the_screen_needs() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let mut mips = Vec::new();
    let mut side = 256u32;
    while side >= 1 {
        mips.push(TextureLevel {
            width: side,
            height: side,
            pixels: vec![200; (side * side * 4) as usize],
        });
        side /= 2;
    }
    let asset = TextureAsset {
        id: AssetId(77),
        name: "crate".into(),
        width: 512,
        height: 512,
        pixels: vec![200; 512 * 512 * 4],
        mips,
        srgb: true,
    };
    let bytes = scrap::asset::to_bytes(&asset, scrap::asset::TEXTURE).unwrap();
    let archived = scrap::asset::view::<TextureAsset>(&bytes).unwrap();
    let target = OffscreenTarget::new(&gpu, 256, 256);
    let mut renderer = Renderer::new(&gpu, &target);
    let texture = renderer.upload_texture(&gpu, archived);
    let cube = renderer.upload_mesh_owned(&gpu, &builtin::cube(1.0));
    let at = |distance: f32| Frame {
        camera: Camera {
            position: Vec3::new(0.0, 0.0, distance),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        draws: vec![Draw {
            mesh: cube,
            transform: Mat4::IDENTITY,
            texture,
            material: Material::new(1.0, 1.0, 1.0),
            pose: None,
        }],
        ..Frame::default()
    };
    let step = |renderer: &mut Renderer, frame: &Frame| {
        renderer.render(&gpu, &target, frame);
        renderer.stream_textures(&gpu, |id| (id == AssetId(77)).then_some(archived), 4);
        renderer.texture_residency()
    };
    let (near, full) = step(&mut renderer, &at(1.5));
    assert_eq!(near, full, "near: every level");
    // Far: coarser each frame, until what 256 pixels of screen show of it.
    let mut far = full;
    for _ in 0..8 {
        far = step(&mut renderer, &at(60.0)).0;
    }
    assert!(far * 20 < full, "far: {far} of {full} bytes");
    // Near again: all of it, on the next frame.
    let (back, _) = step(&mut renderer, &at(1.5));
    assert_eq!(back, full);
}
