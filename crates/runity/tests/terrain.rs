//! Terrain drawn finely round the camera: the rings of its grid meet
//! without a crack wherever the camera stands, and nothing of it is drawn
//! past the terrain's edge — by the vertex shader, or with
//! `RUNITY_MESH_SHADERS=1` by mesh shaders.

use runity::glam::{Mat4, Vec3};
use runity::material::Shading;
use runity::render::{Camera, Draw, Frame, Sky, SkyMode, TextureHandle};
use runity::terrain::{Dunes, Terrain, TerrainSurface};
use runity::{Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 160;

#[test]
fn the_fine_grid_has_no_cracks_and_stops_at_the_edge() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(&gpu, &target);
    // Run with RUNITY_MESH_SHADERS set, the same checks hold of the grid
    // made by mesh shaders — and it must have been made by them.
    assert_eq!(renderer.terrain_by_mesh_shaders(), gpu.mesh_shaders);
    let terrain = Terrain {
        size: 200.0,
        cells: 128,
        dunes: Dunes {
            height: 6.0,
            ..Dunes::default()
        },
    };
    let mesh = renderer.upload_mesh_owned(&gpu, &terrain.mesh());
    let sand = Material {
        shading: Shading::Sand,
        ..Material::new(0.8, 0.62, 0.42)
    };
    let red = Vec3::new(1.0, 0.0, 0.0);
    let shot = |renderer: &mut Renderer, camera: Camera| {
        let frame = Frame {
            sky: Sky {
                mode: SkyMode::Color,
                ..Default::default()
            },
            post: runity::post::PostProcess::OFF,
            ambient_occlusion: runity::ssao::AmbientOcclusion::OFF,
            camera,
            // Red behind it all: a crack shows red.
            clear_color: red,
            draws: vec![Draw {
                mesh,
                transform: Mat4::IDENTITY,
                texture: TextureHandle::WHITE,
                material: sand,
                pose: None,
            }],
            terrain: Some(TerrainSurface {
                mesh,
                placed: Mat4::IDENTITY,
                terrain,
            }),
            ..Frame::default()
        };
        renderer.render(&gpu, &target, &frame);
        target.read_rgba(&gpu)
    };
    let red_pixels = |pixels: &[u8]| {
        pixels
            .chunks(4)
            .filter(|p| p[0] > 200 && p[1] < 40 && p[2] < 40)
            .count()
    };
    // Looking straight down from where the rings are coarse and fine,
    // at several places the grid snaps differently: all ground.
    for (i, x) in [0.0f32, 3.3, 17.7, 41.2].iter().enumerate() {
        let high = 8.0 + i as f32 * 9.0;
        let pixels = shot(
            &mut renderer,
            Camera {
                position: Vec3::new(*x, high, 0.3),
                target: Vec3::new(*x, 0.0, 0.0),
                ..Camera::default()
            },
        );
        assert_eq!(
            red_pixels(&pixels),
            0,
            "a crack, looking down from {high} m at x {x}"
        );
    }
    // Across the ground at a slant, over several rings: no cracks either.
    let low = shot(
        &mut renderer,
        Camera {
            // Low and looking well down, so all of the view is ground.
            position: Vec3::new(0.0, 3.0, 0.0),
            target: Vec3::new(-2.5, 0.0, -3.0),
            ..Camera::default()
        },
    );
    assert_eq!(red_pixels(&low), 0, "a crack along the ground");
    // Past its edge there is nothing: looking at its corner from outside,
    // red shows round it.
    let edge = shot(
        &mut renderer,
        Camera {
            position: Vec3::new(130.0, 40.0, 130.0),
            target: Vec3::new(100.0, 0.0, 100.0),
            ..Camera::default()
        },
    );
    assert!(
        red_pixels(&edge) > (SIZE * SIZE / 5) as usize,
        "nothing past the edge: {}",
        red_pixels(&edge)
    );
}
