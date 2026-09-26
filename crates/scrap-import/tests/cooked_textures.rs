//! A texture cooked into GPU blocks draws the picture it was cooked from:
//! BC7 and ASTC 4x4, sampled as they are where the device can, and the same
//! within a block's error of RGBA8.

use scrap::asset::{AssetId, TextureAsset, TextureCoding, TextureLevel};
use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};
use scrap_import::cook::cook_texture;

const SIDE: u32 = 64;
const SHOT: u32 = 64;

/// Soft stripes and a gradient: what a block encoder gets right and wrong.
fn picture() -> TextureAsset {
    let texel = |x: u32, y: u32| {
        let r = (x * 255 / (SIDE - 1)) as u8;
        let g = (y * 255 / (SIDE - 1)) as u8;
        let b = if (x / 8 + y / 8) % 2 == 0 { 220 } else { 40 };
        [r, g, b, 255]
    };
    let level = |side: u32| -> Vec<u8> {
        let step = SIDE / side;
        (0..side * side).flat_map(|i| texel((i % side) * step, (i / side) * step)).collect()
    };
    let mut mips = Vec::new();
    let mut side = SIDE / 2;
    while side >= 1 {
        mips.push(TextureLevel { width: side, height: side, pixels: level(side) });
        side /= 2;
    }
    TextureAsset {
        id: AssetId(0xC00C),
        name: "stripes".into(),
        width: SIDE,
        height: SIDE,
        pixels: level(SIDE),
        mips,
        srgb: true,
        coding: TextureCoding::Rgba8,
    }
}

/// The texture, unlit, filling the shot.
fn shot(gpu: &Gpu, texture: &TextureAsset) -> Vec<u8> {
    let target = OffscreenTarget::new(gpu, SHOT, SHOT);
    let mut renderer = Renderer::new(gpu, &target);
    let bytes = scrap::asset::to_bytes(texture, scrap::asset::TEXTURE).unwrap();
    let handle = renderer.upload_texture(gpu, scrap::asset::view::<TextureAsset>(&bytes).unwrap());
    let plane = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let frame = Frame {
        post: scrap::post::PostProcess::OFF,
        ambient_occlusion: scrap::ssao::AmbientOcclusion::OFF,
        camera: Camera {
            position: Vec3::new(0.0, 1.0, 0.0001),
            target: Vec3::ZERO,
            fov_y_degrees: 53.13,
            ..Camera::default()
        },
        draws: vec![Draw {
            mesh: plane,
            transform: Mat4::from_scale(Vec3::splat(1.2)),
            texture: handle,
            material: Material { shading: scrap::material::Shading::Unlit, ..Material::new(1.0, 1.0, 1.0) },
            pose: None,
        }],
        ..Frame::default()
    };
    renderer.render(gpu, &target, &frame);
    assert_ne!(handle, TextureHandle::WHITE);
    target.read_rgba(gpu)
}

fn mean_difference(a: &[u8], b: &[u8]) -> f32 {
    let sum: u64 = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f32 / a.len() as f32
}

#[test]
fn a_cooked_texture_draws_the_picture_it_was_cooked_from() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let texture = picture();
    let plain = shot(&gpu, &texture);
    assert!(plain.chunks(4).any(|p| p[2] > 150) && plain.chunks(4).any(|p| p[2] < 80), "the stripes show");
    for coding in [TextureCoding::Bc7, TextureCoding::Astc4x4] {
        let cooked = cook_texture(&texture, coding).unwrap();
        let drawn = shot(&gpu, &cooked);
        let off = mean_difference(&plain, &drawn);
        assert!(off < 4.0, "{coding:?}: {off} off RGBA8 on average");
    }
}

#[test]
fn cooked_blocks_unpack_to_what_they_were_cooked_from() {
    let texture = picture();
    for coding in [TextureCoding::Bc7, TextureCoding::Astc4x4] {
        let cooked = cook_texture(&texture, coding).unwrap();
        let mut texels = vec![0u32; (SIDE * SIDE) as usize];
        let (w, h) = (SIDE as usize, SIDE as usize);
        match coding {
            TextureCoding::Bc7 => texture2ddecoder::decode_bc7(&cooked.pixels, w, h, &mut texels).unwrap(),
            _ => texture2ddecoder::decode_astc(&cooked.pixels, w, h, 4, 4, &mut texels).unwrap(),
        }
        let rgba: Vec<u8> = texels
            .iter()
            .flat_map(|t| {
                let [b, g, r, a] = t.to_le_bytes();
                [r, g, b, a]
            })
            .collect();
        let off = mean_difference(&texture.pixels, &rgba);
        assert!(off < 3.0, "{coding:?}: {off} off the source on average");
    }
}
