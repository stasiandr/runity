//! A picture imported `filter: Nearest` is read nearest-texel: a 2 × 2
//! checker drawn 64 pixels wide is four flat squares, no colour between
//! them — what a voxel kit's palette needs, every block its own colour —
//! and it keeps no mips. The same picture blended runs its squares into
//! each other.

use scrap::asset::{TextureAsset, TextureFilter};
use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};
use scrap_import::{texture_from_image, ImportSettings};

const SHOT: u32 = 64;

/// A red and blue 2 × 2 checker, as a PNG in a fresh folder.
fn checker() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("scrap-texture-filter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("checker.png");
    let red = [220u8, 20, 20, 255];
    let blue = [20u8, 20, 220, 255];
    let pixels: Vec<u8> = [red, blue, blue, red].concat();
    image::save_buffer(&path, &pixels, 2, 2, image::ExtendedColorType::Rgba8).unwrap();
    path
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
        camera: Camera { position: Vec3::new(0.0, 1.0, 0.0001), target: Vec3::ZERO, fov_y_degrees: 53.13, ..Camera::default() },
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
    target.read_rgba(gpu)
}

/// Pixels with both red and blue in them: where squares ran together.
fn blended(pixels: &[u8]) -> usize {
    pixels.chunks(4).filter(|p| p[0] > 70 && p[2] > 70).count()
}

#[test]
fn a_nearest_picture_keeps_its_texels_apart_and_no_mips() {
    let path = checker();
    let source = path.to_string_lossy().into_owned();
    let nearest = texture_from_image(&path, &ImportSettings { filter: TextureFilter::Nearest, ..ImportSettings::for_source(&source) }).unwrap();
    let linear = texture_from_image(&path, &ImportSettings::for_source(&source)).unwrap();
    assert_eq!(nearest.filter, TextureFilter::Nearest);
    assert!(nearest.mips.is_empty(), "a mip would blend the texels it keeps apart");
    assert_eq!(linear.filter, TextureFilter::Linear);
    assert!(!linear.mips.is_empty());
    // The setting is written to the sidecar only when it is not the default.
    let written = ron::ser::to_string(&ImportSettings { filter: TextureFilter::Nearest, ..ImportSettings::default() }).unwrap();
    assert!(written.contains("Nearest") && !ron::ser::to_string(&ImportSettings::default()).unwrap().contains("filter"));
    let read: ImportSettings = ron::from_str("(source: \"a.png\", filter: Nearest)").unwrap();
    assert_eq!(read.filter, TextureFilter::Nearest);

    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let sharp = shot(&gpu, &nearest);
    let soft = shot(&gpu, &linear);
    assert!(sharp.chunks(4).any(|p| p[0] > 150) && sharp.chunks(4).any(|p| p[2] > 150), "both colours show");
    assert_eq!(blended(&sharp), 0, "nearest: four flat squares");
    assert!(blended(&soft) > 100, "blended: the squares run together ({})", blended(&soft));
}
