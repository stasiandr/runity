//! The scene pass starts from the prepass's depth when it is drawn one
//! sample a pixel (TAA on): what is solid is shaded where its depth is
//! equal, with nothing discarded. A card cut out by its alpha still shows
//! the floor through its holes, as it does multisampled with no prepass.

use scrap::glam::{Mat4, Vec3};
use scrap::material::RenderFace;
use scrap::render::{Camera, Draw, Frame, Lighting, Sky, SkyMode, TextureHandle};
use scrap::{builtin, Gpu, Material, OffscreenTarget, Renderer};

const SIZE: u32 = 96;

/// Opaque squares and holes, eight texels each.
fn checker(renderer: &mut Renderer, gpu: &Gpu) -> TextureHandle {
    let n = 64;
    let mut pixels = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let solid = ((x / 8) + (y / 8)) % 2 == 0;
            pixels.extend_from_slice(&[255, 255, 255, if solid { 255 } else { 0 }]);
        }
    }
    renderer.upload_texture_rgba(gpu, n as u32, n as u32, &pixels, true)
}

/// A red card cut out by a checker, over a white floor, seen from above:
/// how many of the middle pixels are red and how many white.
fn card_over_floor(gpu: &Gpu, post: scrap::post::PostProcess) -> (usize, usize) {
    let target = OffscreenTarget::new(gpu, SIZE, SIZE);
    let mut renderer = Renderer::new(gpu, &target);
    let plane = renderer.upload_mesh_owned(gpu, &builtin::plane(1.0, 1));
    let texture = checker(&mut renderer, gpu);
    let frame = Frame {
        sky: Sky {
            mode: SkyMode::Color,
            ..Default::default()
        },
        post,
        camera: Camera {
            position: Vec3::new(0.0, 3.0, 0.01),
            target: Vec3::ZERO,
            fov_y_degrees: 40.0,
            ..Camera::default()
        },
        lighting: Lighting {
            sun_direction: Vec3::new(0.3, -1.0, 0.2).normalize(),
            sun_intensity: 1.0,
            sky_color: Vec3::splat(0.4),
            ground_color: Vec3::splat(0.4),
            ..Lighting::default()
        },
        clear_color: Vec3::ZERO,
        draws: vec![
            Draw {
                mesh: plane,
                transform: Mat4::from_scale(Vec3::new(20.0, 1.0, 20.0)),
                texture: TextureHandle::WHITE,
                material: Material::new(1.0, 1.0, 1.0),
                pose: None,
            },
            Draw {
                mesh: plane,
                transform: Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))
                    * Mat4::from_scale(Vec3::new(4.0, 1.0, 4.0)),
                texture,
                material: Material {
                    alpha_clip: 0.5,
                    render_face: RenderFace::Both,
                    ..Material::new(1.0, 0.0, 0.0)
                },
                pose: None,
            },
        ],
        ..Frame::default()
    };
    renderer.render(gpu, &target, &frame);
    let pixels = target.read_rgba(gpu);
    let (mut red, mut white) = (0, 0);
    for y in SIZE / 4..SIZE * 3 / 4 {
        for x in SIZE / 4..SIZE * 3 / 4 {
            let p = OffscreenTarget::pixel(&pixels, SIZE, x, y);
            let [r, g] = [p[0] as u32, p[1] as u32];
            if r > 60 && g < r / 2 {
                red += 1;
            } else if g > 60 && g * 10 > r * 7 {
                white += 1;
            }
        }
    }
    (red, white)
}

#[test]
fn a_cut_out_card_over_the_prepass_depth_keeps_its_holes() {
    let Ok(gpu) = Gpu::headless_blocking(false) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let (red, white) = card_over_floor(&gpu, scrap::post::PostProcess::default());
    let area = (SIZE / 2 * SIZE / 2) as usize;
    assert!(
        red > area / 4 && white > area / 4,
        "the card and the floor through its holes: {red} red, {white} white of {area}"
    );
    // The same as drawn multisampled, from a cleared depth.
    let (plain_red, plain_white) = card_over_floor(&gpu, scrap::post::PostProcess::OFF);
    assert!(
        red.abs_diff(plain_red) < area / 20 && white.abs_diff(plain_white) < area / 20,
        "over the prepass {red} red, {white} white; multisampled {plain_red} red, {plain_white} white"
    );
}
