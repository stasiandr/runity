//! `shader_gallery <shaders-dir> <out.png>` — every material shader in a
//! folder on a sphere of its own, in one picture: what a batch of shaders
//! written again looks like, before any scene uses them. Names in the
//! order drawn, left to right, top to bottom.

use scrap::glam::{Mat4, Vec3};
use scrap::render::{Camera, Draw, Frame, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let out = std::env::args().nth(2).unwrap();
    let gpu = Gpu::headless_blocking(false).unwrap();
    let target = OffscreenTarget::new(&gpu, 1600, 900);
    let mut renderer = Renderer::new(&gpu, &target);
    let sphere = renderer.upload_mesh_owned(&gpu, &builtin::sphere(0.45, 32, 16));
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "wgsl"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut draws = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let src = std::fs::read_to_string(format!("{dir}/{name}.wgsl")).unwrap();
        let id = scrap::asset::shader_id(name);
        if let Err(e) = renderer.set_material_shader(&gpu, id, &src) {
            eprintln!("{name}: {e}");
        }
        let (x, y) = ((i % 7) as f32 - 3.0, 1.0 - (i / 7) as f32);
        draws.push(Draw {
            mesh: sphere,
            transform: Mat4::from_translation(Vec3::new(x * 1.1, y * 1.1, 0.0)),
            texture: TextureHandle::WHITE,
            material: Material {
                shader: Some(id),
                ..Material::new(0.8, 0.8, 0.8)
            },
            pose: None,
        });
        println!("{i:2} {name}");
    }
    let frame = Frame {
        camera: Camera {
            position: Vec3::new(0.0, 0.0, 7.5),
            target: Vec3::ZERO,
            ..Camera::default()
        },
        draws,
        ..Frame::default()
    };
    renderer.render(&gpu, &target, &frame);
    let rgba = target.read_rgba(&gpu);
    image::save_buffer(&out, &rgba, 1600, 900, image::ColorType::Rgba8).unwrap();
}
