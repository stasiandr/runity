//! Frustum culling: the picture must not change, only the cost.

use runity_math::{vec3, Mat4, Vec3};
use runity_render::{CameraView, Color, Framebuffer, Material, Mesh, Renderer, Sky, SkyParams};

/// A renderer looking down -Z from the origin.
fn scene(width: usize, height: usize) -> (Renderer, Framebuffer, CameraView) {
    let framebuffer = Framebuffer::new(width, height);
    let camera = CameraView::new(
        Mat4::look_at(Vec3::ZERO, vec3(0.0, 0.0, -1.0), Vec3::Y),
        Mat4::perspective(60f32.to_radians(), width as f32 / height as f32, 0.1, 100.0),
        Vec3::ZERO,
        0.1,
        100.0,
    );
    (Renderer::default(), framebuffer, camera)
}

fn render(cull: bool) -> (Framebuffer, usize) {
    let (mut renderer, mut framebuffer, camera) = scene(96, 64);
    renderer.settings.frustum_culling = cull;
    renderer.settings.shadows.enabled = false;
    renderer.settings.ssao.enabled = false;
    renderer.settings.ssr.enabled = false;
    renderer.settings.bloom.enabled = false;
    renderer.set_sky(Sky::new(SkyParams::default()));
    renderer.begin_frame(&mut framebuffer, camera, Color::BLACK);

    let cube = Mesh::cube(1.0);
    let material = Material::default();
    let mut culled = 0;
    // A grid of cubes, most of which are nowhere near the view.
    for x in -6..=6 {
        for z in -6..=6 {
            let model = Mat4::from_translation(vec3(x as f32 * 4.0, 0.0, z as f32 * 4.0));
            culled += renderer
                .draw(&mut framebuffer, &cube, model, &material)
                .meshes_culled;
        }
    }
    renderer.shade(&mut framebuffer);
    (framebuffer, culled)
}

#[test]
fn culling_removes_work_without_changing_the_picture() {
    let (with, culled) = render(true);
    let (without, none) = render(false);

    assert_eq!(none, 0, "culling off should skip nothing");
    assert!(
        culled > 100,
        "most of a 13x13 grid is off screen, culled {culled}"
    );

    // The frames must be identical: culling is an optimisation, and an
    // optimisation that changes the image is a bug.
    for y in 0..with.height() {
        for x in 0..with.width() {
            assert_eq!(
                with.get_pixel(x, y),
                without.get_pixel(x, y),
                "pixel ({x}, {y}) changed when culling was switched on"
            );
        }
    }
}

#[test]
fn what_is_in_view_is_never_culled() {
    let (mut renderer, mut framebuffer, camera) = scene(64, 64);
    renderer.begin_frame(&mut framebuffer, camera, Color::BLACK);
    let cube = Mesh::cube(1.0);

    // Dead ahead, off to the side but still in view, and partly off the edge.
    for position in [
        vec3(0.0, 0.0, -5.0),
        vec3(2.0, 0.0, -6.0),
        vec3(-3.4, 0.0, -6.0),
    ] {
        assert!(
            renderer.visible(&cube, Mat4::from_translation(position)),
            "{position:?} should be visible"
        );
    }

    for position in [
        vec3(0.0, 0.0, 5.0),
        vec3(60.0, 0.0, -6.0),
        vec3(0.0, 0.0, -400.0),
    ] {
        assert!(
            !renderer.visible(&cube, Mat4::from_translation(position)),
            "{position:?} should be culled"
        );
    }
}

#[test]
fn a_scaled_mesh_is_measured_at_its_real_size() {
    // A cube scaled up twenty times reaches into view from a position that
    // would be culled at its original size.
    let (mut renderer, mut framebuffer, camera) = scene(64, 64);
    renderer.begin_frame(&mut framebuffer, camera, Color::BLACK);
    let cube = Mesh::cube(1.0);

    let far_aside = Mat4::from_translation(vec3(14.0, 0.0, -8.0));
    assert!(!renderer.visible(&cube, far_aside));

    let huge = far_aside * Mat4::from_scale(Vec3::splat(20.0));
    assert!(
        renderer.visible(&cube, huge),
        "a big enough thing reaches into view"
    );
}

#[test]
fn a_shadow_caster_behind_the_camera_still_casts() {
    // The reason culling does not simply skip the draw: something out of
    // sight can still be between the sun and the frame.
    let (mut renderer, mut framebuffer, camera) = scene(64, 64);
    renderer.settings.frustum_culling = true;
    renderer.settings.shadows.enabled = true;
    renderer.settings.ssao.enabled = false;
    renderer.settings.ssr.enabled = false;
    renderer.set_sky(Sky::new(SkyParams {
        sun_direction: vec3(0.0, 1.0, 0.4).normalized(),
        ..SkyParams::default()
    }));
    renderer.begin_frame(&mut framebuffer, camera, Color::BLACK);

    let floor = Mesh::plane(40.0, 1);
    let blocker = Mesh::cube(4.0);
    let material = Material::default();

    // The floor is in view; the blocker is above and behind the camera,
    // between the sun and the floor.
    renderer.draw(
        &mut framebuffer,
        &floor,
        Mat4::from_translation(vec3(0.0, -2.0, -8.0)),
        &material,
    );
    let stats = renderer.draw(
        &mut framebuffer,
        &blocker,
        Mat4::from_translation(vec3(0.0, 6.0, 4.0)),
        &material,
    );
    assert_eq!(stats.meshes_culled, 1, "the blocker itself is not drawn");
    renderer.shade(&mut framebuffer);

    // Somewhere on the floor is darker than the brightest lit part of it.
    let mut brightest = 0.0f32;
    let mut darkest = f32::INFINITY;
    for y in framebuffer.height() / 2..framebuffer.height() {
        for x in 0..framebuffer.width() {
            let luminance = framebuffer.get_pixel(x, y).luminance();
            brightest = brightest.max(luminance);
            darkest = darkest.min(luminance);
        }
    }
    assert!(
        darkest < brightest * 0.8,
        "the culled blocker should still shadow the floor"
    );
}

#[test]
fn an_empty_mesh_is_culled_rather_than_drawn() {
    let (mut renderer, mut framebuffer, camera) = scene(32, 32);
    renderer.begin_frame(&mut framebuffer, camera, Color::BLACK);
    let empty = Mesh::new(Vec::new(), Vec::new());
    assert!(!renderer.visible(&empty, Mat4::IDENTITY));
}

#[test]
fn bounds_cover_every_vertex() {
    let sphere = Mesh::sphere(2.0, 16, 12);
    let (min, max) = sphere.bounds();
    for vertex in &sphere.vertices {
        assert!(vertex.position.x >= min.x - 1e-5 && vertex.position.x <= max.x + 1e-5);
        assert!(vertex.position.y >= min.y - 1e-5 && vertex.position.y <= max.y + 1e-5);
        assert!(vertex.position.z >= min.z - 1e-5 && vertex.position.z <= max.z + 1e-5);
    }

    let (centre, radius) = sphere.bounding_sphere();
    assert!(
        (radius - 2.0).abs() < 0.05,
        "a sphere of radius two: {radius}"
    );
    assert!(centre.length() < 1e-4, "centred on the origin");
    for vertex in &sphere.vertices {
        assert!((vertex.position - centre).length() <= radius + 1e-4);
    }
}
