//! A mesh the game rewrites: drawn where its data says, and drawn again
//! where the new data says once it is changed.

use scrap::asset::Vertex;
use scrap::glam::{Mat4, Vec3};
use scrap::material::Shading;
use scrap::render::{Camera, FogSettings, Lighting, Sky, SkyMode};
use scrap::world::{build_frame, LiveMesh, Surface, WorldTransform};
use scrap::{Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 32;

/// A flat square a metre across, `side` metres to the right.
fn square(side: f32) -> (Vec<Vertex>, Vec<u32>) {
    let v = |x: f32, z: f32| Vertex {
        position: [x + side, 0.0, z],
        normal: [0.0, 1.0, 0.0],
        uv: [0.0, 0.0],
    };
    (
        vec![v(-0.5, -0.5), v(0.5, -0.5), v(0.5, 0.5), v(-0.5, 0.5)],
        vec![0, 2, 1, 0, 3, 2],
    )
}

#[test]
fn a_live_mesh_is_drawn_from_its_data_as_it_changes() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    let mut world = scrap::hecs::World::new();
    let (vertices, indices) = square(0.0);
    let water = world.spawn((
        WorldTransform(Mat4::IDENTITY),
        LiveMesh::new(vertices, indices),
        Surface(Material {
            shading: Shading::Unlit,
            render_face: scrap::material::RenderFace::Both,
            ..Material::new(1.0, 0.0, 0.0)
        }),
    ));
    let camera = Camera {
        position: Vec3::new(0.0, 3.0, 0.001),
        target: Vec3::ZERO,
        ..Camera::default()
    };
    let mut red_in_middle = |world: &scrap::hecs::World| {
        let mut frame = build_frame(world, camera, Lighting::default(), FogSettings::default());
        frame.sky = Sky {
            mode: SkyMode::Color,
            ..Default::default()
        };
        frame.clear_color = Vec3::ZERO;
        frame.post = scrap::post::PostProcess::OFF;
        frame.ambient_occlusion = scrap::ssao::AmbientOcclusion::OFF;
        renderer.render(&gpu, &target, &frame);
        OffscreenTarget::pixel(&target.read_rgba(&gpu), SIZE, SIZE / 2, SIZE / 2)[0] > 128
    };
    assert!(red_in_middle(&world), "drawn where its data says");
    assert!(red_in_middle(&world), "and again, unchanged");
    world
        .get::<&mut LiveMesh>(water)
        .unwrap()
        .move_vertices(|_, at| at + Vec3::new(3.0, 0.0, 0.0));
    assert!(!red_in_middle(&world), "moved aside: gone from the middle");
    let (vertices, indices) = square(0.0);
    world
        .get::<&mut LiveMesh>(water)
        .unwrap()
        .set(vertices, indices);
    assert!(red_in_middle(&world), "back");
}
