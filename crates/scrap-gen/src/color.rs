//! A generated mesh's one colour.
//!
//! The networks paint a texture; the engine's materials are a colour
//! (`docs/stack.md`, «Материал — ассет»). Until a draft carries colour per
//! face, it gets the average of what it was painted — so a well comes out
//! stone-grey and a barrel brown, not both the default white.

use anyhow::{Context as _, Result};

/// The average colour of the surface, as sRGB bytes.
///
/// Sampled at the vertices rather than over the texture: an atlas is mostly
/// gutter, and the vertices are where the surface is. Averaged in linear
/// light, for the reason mips are (`docs/stack.md`, «Текстуры»).
pub fn average(glb: &[u8]) -> Result<[u8; 3]> {
    let (document, buffers, images) = gltf::import_slice(glb).context("reading the glb")?;
    let mut sum = [0.0f64; 3];
    let mut count = 0.0f64;
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|b| Some(&buffers[b.index()]));
            let pbr = primitive.material().pbr_metallic_roughness();
            let factor = pbr.base_color_factor();
            let texture = pbr
                .base_color_texture()
                .and_then(|info| images.get(info.texture().source().index()));
            let colors: Option<Vec<[f32; 4]>> =
                reader.read_colors(0).map(|c| c.into_rgba_f32().collect());
            let uvs: Option<Vec<[f32; 2]>> =
                reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            for (i, _) in positions.enumerate() {
                let mut linear = [factor[0], factor[1], factor[2]];
                if let Some(color) = colors.as_ref().and_then(|c| c.get(i)) {
                    for (l, c) in linear.iter_mut().zip(color) {
                        *l *= c;
                    }
                }
                if let (Some(image), Some(uv)) = (texture, uvs.as_ref().and_then(|u| u.get(i))) {
                    if let Some(texel) = sample(image, *uv) {
                        for (l, t) in linear.iter_mut().zip(texel) {
                            *l *= to_linear(t);
                        }
                    }
                }
                for (s, l) in sum.iter_mut().zip(linear) {
                    *s += f64::from(l);
                }
                count += 1.0;
            }
        }
    }
    if count == 0.0 {
        return Ok([200, 200, 200]);
    }
    Ok(sum.map(|s| to_srgb_byte((s / count) as f32)))
}

/// The texel under a UV, nearest, wrapping as glTF's default sampler does.
fn sample(image: &gltf::image::Data, uv: [f32; 2]) -> Option<[u8; 3]> {
    let channels = match image.format {
        gltf::image::Format::R8G8B8 => 3,
        gltf::image::Format::R8G8B8A8 => 4,
        _ => return None,
    };
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let wrap = |t: f32, n: usize| ((t.rem_euclid(1.0) * n as f32) as usize).min(n - 1);
    let (x, y) = (wrap(uv[0], w), wrap(uv[1], h));
    let at = (y * w + x) * channels;
    image.pixels.get(at..at + 3).map(|p| [p[0], p[1], p[2]])
}

fn to_linear(byte: u8) -> f32 {
    let c = f32::from(byte) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb_byte(linear: f32) -> u8 {
    let l = linear.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_survives_the_round_trip() {
        for byte in [0u8, 1, 64, 128, 200, 255] {
            assert_eq!(to_srgb_byte(to_linear(byte)), byte);
        }
    }
}
