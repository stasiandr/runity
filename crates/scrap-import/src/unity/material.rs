//! URP Lit materials (`.mat`) as `.scrmat`.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use yaml_rust2::Yaml;

use super::yaml::{self, Get};
use super::Unity;

/// A material's saved property of one kind, by Unity's name: its
/// `m_Floats`, `m_Colors` and `m_TexEnvs` are lists of one-key maps.
fn property<'a>(material: &'a Yaml, list: &str, name: &str) -> Option<&'a Yaml> {
    material["m_SavedProperties"]
        .list(list)
        .iter()
        .find_map(|item| match item {
            Yaml::Hash(h) => h
                .iter()
                .find(|(k, _)| k.as_str() == Some(name))
                .map(|(_, v)| v),
            _ => None,
        })
}

pub(super) fn float(m: &Yaml, name: &str) -> Option<f32> {
    property(m, "m_Floats", name)
        .and_then(yaml::number)
        .map(|n| n as f32)
}

pub(super) fn color(m: &Yaml, name: &str) -> Option<[f32; 4]> {
    let c = property(m, "m_Colors", name)?;
    Some([
        yaml::number(&c["r"])? as f32,
        yaml::number(&c["g"])? as f32,
        yaml::number(&c["b"])? as f32,
        yaml::number(&c["a"]).unwrap_or(1.0) as f32,
    ])
}

fn texture<'a>(m: &'a Yaml, name: &str) -> Option<(String, &'a Yaml)> {
    let t = property(m, "m_TexEnvs", name)?;
    let r = t.reference("m_Texture")?;
    Some((r.guid?, t))
}

/// The GUID Unity's built-in extra resources go by: a texture of theirs is
/// told apart by its fileID alone.
const BUILTIN_EXTRA: &str = "0000000000000000f000000000000000";

/// One of Unity's built-in textures a material names (`Default-Checker-Gray`
/// on a prototyping floor), made again: its name in the project and its
/// picture. Unity ships them inside the editor, not in the project, so
/// there is no file to copy.
pub fn builtin_texture(file_id: i64) -> Option<(&'static str, image::RgbaImage)> {
    // Two squares by two, each colour a flat sRGB grey, as Unity's are.
    let checker = |light: u8, dark: u8| {
        image::RgbaImage::from_fn(64, 64, |x, y| {
            let v = if (x < 32) == (y < 32) { light } else { dark };
            image::Rgba([v, v, v, 255])
        })
    };
    match file_id {
        10309 => Some(("Default-Checker-Gray", checker(196, 157))),
        _ => None,
    }
}

/// The built-in texture a material's slot names, when it is one made
/// again here: its fileID.
fn builtin_in(m: &Yaml, slot: &str) -> Option<i64> {
    let r = property(m, "m_TexEnvs", slot)?.reference("m_Texture")?;
    (r.guid.as_deref() == Some(BUILTIN_EXTRA) && builtin_texture(r.file_id).is_some()).then_some(r.file_id)
}

/// The built-in textures the materials use, by fileID: what to make.
pub fn builtin_textures_used(unity: &Unity) -> BTreeSet<i64> {
    let mut used = BTreeSet::new();
    for (_, path) in unity.of_kind("material") {
        let Some(body) = material_body(unity, path) else {
            continue;
        };
        used.extend(MAPS.iter().filter_map(|(slot, _)| builtin_in(&body, slot)));
    }
    used
}

/// A `.mat`'s Material as it draws: a variant (`m_Parent` set) saves only
/// what it overrides, so its parents' properties come first and its own
/// replace them by name; the shader is the variant's own when it says one.
pub fn material_body(unity: &Unity, path: &Path) -> Option<Yaml> {
    material_body_at(unity, path, 0)
}

fn material_body_at(unity: &Unity, path: &Path, depth: usize) -> Option<Yaml> {
    let text = std::fs::read_to_string(path).ok()?;
    let body = yaml::documents(&text)
        .into_iter()
        .find(|d| d.kind == "Material")?
        .body;
    let Some(parent) = body
        .reference("m_Parent")
        .and_then(|r| r.guid)
        .and_then(|g| unity.guids.get(&g))
        .filter(|_| depth < 8)
        .and_then(|p| material_body_at(unity, p, depth + 1))
    else {
        return Some(body);
    };
    Some(overlay(parent, body))
}

/// `child`'s saved properties over `parent`'s, and its other fields as
/// they are (a variant's shader is its parent's unless it says otherwise).
fn overlay(parent: Yaml, child: Yaml) -> Yaml {
    let (Yaml::Hash(mut merged), Yaml::Hash(own)) = (parent, child) else {
        return Yaml::BadValue;
    };
    let props_key = Yaml::String("m_SavedProperties".into());
    let mut props = match merged.remove(&props_key) {
        Some(Yaml::Hash(h)) => h,
        _ => Default::default(),
    };
    for (k, v) in own {
        if k == props_key {
            let Yaml::Hash(child_props) = v else { continue };
            for (list, items) in child_props {
                let (Some(Yaml::Array(base)), Yaml::Array(over)) = (props.get(&list).cloned(), &items)
                else {
                    props.insert(list, items);
                    continue;
                };
                let name = |y: &Yaml| match y {
                    Yaml::Hash(h) => h.keys().next().cloned(),
                    _ => None,
                };
                let mut out: Vec<Yaml> = base
                    .into_iter()
                    .filter(|b| !over.iter().any(|o| name(o).is_some() && name(o) == name(b)))
                    .collect();
                out.extend(over.iter().cloned());
                props.insert(list, Yaml::Array(out));
            }
        } else if k.as_str() == Some("m_Shader") && yaml::reference(&v).is_none_or(|r| r.is_none()) {
            continue;
        } else {
            merged.insert(k, v);
        }
    }
    merged.insert(props_key, Yaml::Hash(props));
    Yaml::Hash(merged)
}

/// Which texture slots come over, and what they are called in `.scrmat`.
const MAPS: [(&str, &str); 5] = [
    ("_BaseMap", "base_map"),
    ("_MainTex", "base_map"),
    ("_BumpMap", "normal_map"),
    ("_EmissionMap", "emission_map"),
    ("_MaskMap", "mask_map"),
];

/// The textures every material uses — what is worth copying — and which
/// of them are only ever data (a normal map, a mask), never colour.
pub fn textures_used(unity: &Unity) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut used = BTreeSet::new();
    let mut colour = BTreeSet::new();
    for (_, path) in unity.of_kind("material") {
        let Some(body) = material_body(unity, path) else {
            continue;
        };
        {
            let mut base = false;
            for (slot, field) in MAPS {
                if let Some((guid, _)) = texture(&body, slot) {
                    if matches!(field, "base_map" | "emission_map") {
                        colour.insert(guid.clone());
                    }
                    base |= field == "base_map";
                    used.insert(guid);
                }
            }
            let own = own_shader(unity, &body);
            if !base && own.is_some() {
                if let Some(guid) = own_texture(unity, &body) {
                    colour.insert(guid.clone());
                    used.insert(guid);
                }
            }
            // What it hands its own shader: colour, unless its name says
            // it is a normal map (Unity's import settings may still say
            // it is data).
            if let Some((_, shader)) = &own {
                for (property, guid) in shader_textures(unity, &body, shader) {
                    let lower = property.to_lowercase();
                    if !lower.contains("normal") && !lower.contains("bump") {
                        colour.insert(guid.clone());
                    }
                    used.insert(guid);
                }
            }
        }
    }
    let data = used.difference(&colour).cloned().collect();
    (used, data)
}

fn hex(c: [f32; 4]) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c[0]), byte(c[1]), byte(c[2]))
}

/// The first texture a custom shader's material sets that is not a normal
/// map: what its surface function reads as the base map.
fn own_texture(unity: &Unity, m: &Yaml) -> Option<String> {
    m["m_SavedProperties"]
        .list("m_TexEnvs")
        .iter()
        .find_map(|item| {
            let Yaml::Hash(h) = item else { return None };
            let (k, v) = h.iter().next()?;
            let slot = k.as_str()?.to_lowercase();
            if slot.contains("normal") || slot.contains("bump") {
                return None;
            }
            let guid = v.reference("m_Texture")?.guid?;
            matches!(unity.named(&guid), Some(("texture", _))).then_some(guid)
        })
}

/// The textures a custom shader's material sets that the shader reads:
/// each property's name and its texture's GUID, in the `.mat`'s order.
///
/// A material keeps the properties of every shader it ever had — a stale
/// `_BaseMap`, a Sample Texture 2D node since deleted — so only those the
/// shader's file still mentions come over: an exposed property by its
/// name, a texture set in a node (`_SampleTexture2D_<node>_Texture_1_…`,
/// as Unity saves it on the material) by the node's id. A shader that
/// cannot be read keeps them all. A picture the import does not copy (a
/// `.psd`) is left out rather than named: the material would not build.
pub fn shader_textures(unity: &Unity, m: &Yaml, shader: &Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(shader).ok();
    let copied = |guid: &str| {
        unity.guids.get(guid).is_some_and(|p| {
            p.extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .is_some_and(|e| matches!(e.as_str(), "png" | "jpg" | "jpeg" | "tga" | "bmp"))
        })
    };
    let read = |property: &str| {
        let Some(text) = &text else { return true };
        match property
            .strip_prefix("_SampleTexture2D_")
            .and_then(|rest| rest.split('_').next())
        {
            Some(node) => text.contains(node),
            None => text.contains(property),
        }
    };
    m["m_SavedProperties"]
        .list("m_TexEnvs")
        .iter()
        .filter_map(|item| {
            let Yaml::Hash(h) = item else { return None };
            let (k, v) = h.iter().next()?;
            let property = k.as_str()?;
            let guid = v.reference("m_Texture")?.guid?;
            (matches!(unity.named(&guid), Some(("texture", _))) && copied(&guid) && read(property))
                .then(|| (property.to_string(), guid))
        })
        .collect()
}

/// A picture the shader graph holds itself — a Sample Texture 2D with its
/// texture set in the graph, not a property the material fills: the
/// palette every low-poly model of a pack is coloured from by its UVs.
fn graph_texture(unity: &Unity, graph: &Path) -> Option<String> {
    let text = std::fs::read_to_string(graph).ok()?;
    text.split("\"m_SerializedTexture\"")
        .skip(1)
        .find_map(|rest| {
            let value = rest.split('\n').next()?;
            let after = &value[value.find("guid")? + 4..];
            let start = after.find(|c: char| c.is_ascii_hexdigit())?;
            let guid: String = after[start..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            (guid.len() == 32 && matches!(unity.named(&guid), Some(("texture", _)))).then_some(guid)
        })
}

/// A colour property a custom shader named its own way (`_Main_Color`,
/// `_Tint`): the first whose name says colour and not emission.
fn any_colour(m: &Yaml) -> Option<[f32; 4]> {
    m["m_SavedProperties"]
        .list("m_Colors")
        .iter()
        .find_map(|item| {
            let Yaml::Hash(h) = item else { return None };
            let (k, _) = h.iter().next()?;
            let name = k.as_str()?.to_lowercase();
            (name.contains("color") || name.contains("colour") || name.contains("tint"))
                .then_some(())
                .filter(|_| !name.contains("emission") && !name.contains("fresnel"))
                .and_then(|_| color(m, k.as_str()?))
        })
}

/// How a custom shader draws, as it says itself: see-through, which faces,
/// cut out by alpha.
#[derive(Debug, Default, PartialEq)]
pub struct ShaderLook {
    pub transparent: bool,
    pub face: Option<&'static str>,
    pub clip: bool,
    /// Not lit: URP's Unlit and Sprite Unlit targets.
    pub unlit: bool,
    /// Drawn over everything: a graph's `m_ZTestMode` 8, Always.
    pub on_top: bool,
}

/// A Shader Graph's URP target (`m_SurfaceType`, `m_RenderFace`,
/// `m_AlphaClip`; sprite targets are always blended), or a `.shader`'s
/// tags and states (`Queue`, `Blend`, `Cull`).
pub fn shader_look(path: &Path) -> ShaderLook {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let number = |key: &str| {
        text.split(&format!("\"{key}\": "))
            .nth(1)
            .and_then(|r| r.split([',', '\n']).next())
            .map(|v| v.trim().to_string())
    };
    if path.extension().is_some_and(|e| e == "shadergraph") {
        let sprite = text.contains("UniversalSpriteUnlitSubTarget")
            || text.contains("UniversalSpriteLitSubTarget");
        ShaderLook {
            transparent: sprite || number("m_SurfaceType").as_deref() == Some("1"),
            // URP: 0 both, 1 back, 2 front. Its back faces keep their
            // normal (no double-sided normal mode): both as front.
            face: match number("m_RenderFace").as_deref() {
                Some("0") => Some("BothAsFront"),
                Some("1") => Some("Back"),
                _ => None,
            },
            clip: number("m_AlphaClip").as_deref() == Some("true"),
            unlit: text.contains("UniversalUnlitSubTarget")
                || text.contains("UniversalSpriteUnlitSubTarget"),
            on_top: number("m_ZTestMode").as_deref() == Some("8"),
        }
    } else {
        let lower = text.to_lowercase();
        ShaderLook {
            transparent: lower.contains("\"queue\"=\"transparent")
                || lower.contains("blend srcalpha"),
            face: lower.contains("cull off").then_some("BothAsFront"),
            clip: false,
            unlit: !lower.contains("lightmode\"=\"universalforward"),
            on_top: lower.contains("ztest always"),
        }
    }
}

/// The material's own shader — a Shader Graph or a `.shader` in the
/// project, not one of URP's — as the name a `.scrmat` gives it and where
/// it was.
pub fn own_shader(unity: &Unity, m: &Yaml) -> Option<(String, std::path::PathBuf)> {
    let path = m
        .reference("m_Shader")
        .and_then(|r| r.guid)
        .and_then(|g| unity.guids.get(&g))?;
    Some((super::snake(&super::stem(path)), path.clone()))
}

/// What a stub says of itself: a file that still says it is written over.
pub const STUB_MARK: &str = "to be written again from";

/// A starting point for a shader to write again: where it was, what it
/// exposed, what it was made of, and a `surface` that leaves the standard
/// shader's work as it is — so the material shows in its colours until
/// someone writes it.
pub fn shader_stub(unity: &Unity, name: &str, path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut properties: Vec<String> = Vec::new();
    let mut nodes: std::collections::BTreeSet<String> = Default::default();
    // A Shader Graph is JSON objects one after another: each object's type
    // and its first name.
    for object in text.split("\n\n{") {
        let kind = object
            .split("\"m_Type\": \"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .unwrap_or("");
        let first_name = object
            .split("\"m_Name\": \"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .unwrap_or("");
        if let Some(property) = kind
            .strip_prefix("UnityEditor.ShaderGraph.Internal.")
            .and_then(|k| k.strip_suffix("ShaderProperty"))
        {
            properties.push(format!("//   {first_name} ({property})"));
        } else if let Some(node) = kind
            .strip_prefix("UnityEditor.ShaderGraph.")
            .and_then(|k| k.strip_suffix("Node"))
        {
            nodes.insert(node.to_string());
        }
    }
    // A hand-written `.shader`: its Properties block says as much.
    if properties.is_empty() {
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('_') && line.contains('(') && line.contains('"') {
                properties.push(format!("//   {line}"));
            }
        }
    }
    let nodes: Vec<String> = nodes.into_iter().collect();
    format!(
        "// {name}: {STUB_MARK} {}.\n\
         //\n\
         // What it exposed:\n{}\n\
         //{}\n\
         //\n\
         // Until then the standard shader draws it, in its material's colours.\n\n\
         fn surface(in: SurfaceIn, out: Surface) -> Surface {{\n    return out;\n}}\n",
        path.strip_prefix(&unity.root).unwrap_or(path).display(),
        if properties.is_empty() {
            "//   (nothing)".to_string()
        } else {
            properties.join("\n")
        },
        if nodes.is_empty() {
            String::new()
        } else {
            format!(" What it was made of: {}.", nodes.join(", "))
        }
    )
}

/// A `.mat` as the text of a `.scrmat`: what URP Lit says, with the rest of
/// a custom shader's colour carried as far as it goes.
/// What a shader's `// scrap:params` line says its eight numbers are:
/// Unity property names, `_Speed`, or a colour's channel, `_Tint.r`.
pub fn declared_params(shader: &str) -> Vec<String> {
    shader
        .lines()
        .find_map(|l| l.trim().strip_prefix("// scrap:params"))
        .map(|rest| {
            rest.split_whitespace()
                .take(8)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A `.mat`'s values for those names: floats as they are, colour channels
/// as written (sRGB for a colour property); 0 for one it does not set.
fn param_values(m: &Yaml, names: &[String]) -> Vec<f32> {
    names
        .iter()
        .map(|name| match name.rsplit_once('.') {
            Some((property, channel)) if matches!(channel, "r" | "g" | "b" | "a") => {
                color(m, property)
                    .map(|c| {
                        c[match channel {
                            "r" => 0,
                            "g" => 1,
                            "b" => 2,
                            _ => 3,
                        }]
                    })
                    .unwrap_or(0.0)
            }
            _ => float(m, name).unwrap_or(0.0),
        })
        .collect()
}

/// A line a shader written again says of itself: `// scrap:<key> value`.
pub fn declared(shader: &str, key: &str) -> Option<String> {
    let mark = format!("// scrap:{key} ");
    shader
        .lines()
        .find_map(|l| l.trim().strip_prefix(&mark).map(|v| v.trim().to_string()))
}

/// [`convert_with`] for a shader not written again yet.
#[cfg(test)]
pub fn convert(unity: &Unity, path: &Path) -> Result<String> {
    convert_with(unity, path, &|_| None)
}

/// A `.mat` as the text of a `.scrmat`. `shader_text` finds, by name, the
/// shader written again for it, which may say what its material needs: its
/// eight numbers (`// scrap:params`), a base map (`// scrap:base_map
/// render:mirror`), how that is laid (`// scrap:screen_map Mirror`).
pub fn convert_with(
    unity: &Unity,
    path: &Path,
    shader_text: &dyn Fn(&str) -> Option<String>,
) -> Result<String> {
    std::fs::metadata(path).with_context(|| format!("{}", path.display()))?;
    let body = material_body(unity, path).context("no Material in it")?;
    let m = &body;
    let mut fields: Vec<String> = Vec::new();
    let base = color(m, "_BaseColor")
        .or_else(|| color(m, "_Color"))
        .or_else(|| color(m, "_FaceColor"))
        .or_else(|| any_colour(m))
        .unwrap_or([1.0; 4]);
    fields.push(format!("color: {:?}", hex(base)));
    if let Some(metallic) = float(m, "_Metallic").filter(|v| *v != 0.0) {
        fields.push(format!("metallic: {metallic}"));
    }
    if let Some(smooth) = float(m, "_Smoothness").or_else(|| float(m, "_Glossiness")) {
        fields.push(format!("smoothness: {smooth}"));
    }
    if let Some(e) = color(m, "_EmissionColor") {
        // HDR: past white, the brightest channel is the intensity.
        let peak = e[0].max(e[1]).max(e[2]);
        if peak > 0.0 {
            let intensity = peak.max(1.0);
            fields.push(format!(
                "emission: {:?}",
                hex([e[0] / intensity, e[1] / intensity, e[2] / intensity, 1.0])
            ));
            if intensity > 1.0 {
                fields.push(format!("emission_intensity: {intensity}"));
            }
        }
    }
    if float(m, "_Surface") == Some(1.0) {
        fields.push("surface: Transparent".into());
        if base[3] < 1.0 {
            fields.push(format!("alpha: {}", base[3]));
        }
        match float(m, "_Blend") {
            Some(1.0) => fields.push("blend: Premultiply".into()),
            Some(2.0) => fields.push("blend: Additive".into()),
            _ => {}
        }
    }
    // Blending said by its factors, where `_Surface` does not: a built-in
    // particle shader's (SrcAlpha, OneMinusSrcAlpha) is see-through, a
    // (SrcAlpha or One, One) adds.
    if !fields.iter().any(|f| f.starts_with("surface")) {
        match (float(m, "_SrcBlend"), float(m, "_DstBlend")) {
            (Some(5.0) | Some(1.0), Some(10.0)) => {
                fields.push("surface: Transparent".into());
                if base[3] < 1.0 {
                    fields.push(format!("alpha: {}", base[3]));
                }
                if float(m, "_SrcBlend") == Some(1.0) {
                    fields.push("blend: Premultiply".into());
                }
            }
            (Some(5.0) | Some(1.0), Some(1.0)) => {
                fields.push("surface: Transparent".into());
                fields.push("blend: Additive".into());
            }
            _ => {}
        }
    }
    // Unity's own particle shaders (built in, fileID 200 up, bar the lit
    // Standard Surface) light nothing.
    if let Some(r) = m.reference("m_Shader") {
        let builtin = r.guid.as_deref() == Some("0000000000000000f000000000000000");
        if builtin && (200..=211).contains(&r.file_id) && r.file_id != 210 {
            fields.push("unlit: true".into());
        }
    }
    if float(m, "_AlphaClip") == Some(1.0) {
        fields.push(format!(
            "alpha_clip: {}",
            float(m, "_Cutoff").unwrap_or(0.5)
        ));
    }
    match float(m, "_Cull") {
        // URP never turns a back face's normal: Both as front.
        Some(0.0) => fields.push("render_face: BothAsFront".into()),
        Some(1.0) => fields.push("render_face: Back".into()),
        _ => {}
    }
    let mut tiled = false;
    for (slot, field) in MAPS {
        if fields.iter().any(|f| f.starts_with(field)) {
            continue;
        }
        let Some((guid, t)) = texture(m, slot) else {
            continue;
        };
        let name = match builtin_in(m, slot).and_then(builtin_texture) {
            Some((name, _)) => name.to_string(),
            None => match unity.named(&guid) {
                Some((kind, name)) if kind == "texture" => name.to_string(),
                _ => continue,
            },
        };
        fields.push(format!("{field}: {name:?}"));
        if field == "base_map" && !tiled {
            tiled = true;
            if let Some(scale) = t.get("m_Scale").as_hash().map(|_| &t["m_Scale"]) {
                let (x, y) = (scale.f32("x").unwrap_or(1.0), scale.f32("y").unwrap_or(1.0));
                if (x, y) != (1.0, 1.0) {
                    fields.push(format!("tiling: ({x}, {y})"));
                }
            }
            let offset = &t["m_Offset"];
            let (x, y) = (
                offset.f32("x").unwrap_or(0.0),
                offset.f32("y").unwrap_or(0.0),
            );
            if (x, y) != (0.0, 0.0) {
                fields.push(format!("offset: ({x}, {y})"));
            }
        }
    }
    if let Some(scale) = float(m, "_BumpScale").filter(|v| *v != 1.0) {
        if fields.iter().any(|f| f.starts_with("normal_map")) {
            fields.push(format!("normal_scale: {scale}"));
        }
    }
    let own = own_shader(unity, m);
    // A custom shader's picture, in a slot of its own naming: the base
    // map, for its surface function to read (there is no other).
    if own.is_some() && !fields.iter().any(|f| f.starts_with("base_map")) {
        let graph = own
            .as_ref()
            .and_then(|(_, path)| graph_texture(unity, path));
        if let Some((_, name)) = own_texture(unity, m)
            .or(graph)
            .and_then(|g| unity.named(&g))
        {
            fields.push(format!("base_map: {name:?}"));
        }
    }
    if let Some((name, path)) = &own {
        fields.push(format!("shader: {name:?}"));
        let written = shader_text(name).unwrap_or_default();
        let names = declared_params(&written);
        // `none`: no base map, so the standard shader hands the surface
        // function the colour as it is — a particle's own.
        if let Some(map) = declared(&written, "base_map") {
            fields.retain(|f| !f.starts_with("base_map") && !f.starts_with("tiling") && !f.starts_with("offset"));
            if map != "none" {
                fields.push(format!("base_map: {map:?}"));
            }
        }
        if let Some(how) = declared(&written, "screen_map") {
            fields.push(format!("screen_map: {how}"));
        }
        // What the engine already does for it, in `.scrmat` fields: grass
        // sways by the standard shader's wind rather than its own.
        if let Some(own) = declared(&written, "material") {
            fields.push(own);
        }
        // Every texture it sets that the shader reads, by the name the
        // shader reads it by; the written shader's `// scrap:textures`
        // line picks which go in its slots.
        let textures: Vec<String> = shader_textures(unity, m, path)
            .into_iter()
            .filter_map(|(property, guid)| {
                let (_, texture) = unity.named(&guid)?;
                Some(format!("{property:?}: {texture:?}"))
            })
            .collect();
        if !textures.is_empty() {
            fields.push(format!("textures: {{{}}}", textures.join(", ")));
        }
        if !names.is_empty() {
            let values: Vec<String> = param_values(m, &names)
                .into_iter()
                .map(|v| format!("{v}"))
                .collect();
            fields.push(format!("params: [{}]", values.join(", ")));
        }
        // What the shader itself says about how it is drawn, where the
        // material's own settings are silent (a graph forces them).
        let look = shader_look(path);
        let has = |fields: &[String], field: &str| fields.iter().any(|f| f.starts_with(field));
        if look.transparent && !has(&fields, "surface") {
            fields.push("surface: Transparent".into());
            if base[3] < 1.0 && !has(&fields, "alpha") {
                fields.push(format!("alpha: {}", base[3]));
            }
        }
        if let Some(face) = look.face.filter(|_| !has(&fields, "render_face")) {
            fields.push(format!("render_face: {face}"));
        }
        if look.clip && !has(&fields, "alpha_clip") {
            fields.push(format!(
                "alpha_clip: {}",
                float(m, "_Cutoff").unwrap_or(0.5)
            ));
        }
        if look.unlit {
            fields.push("unlit: true".into());
        }
        if look.on_top && look.transparent {
            fields.push("on_top: true".into());
        }
    }
    let shader = own
        .map(|(name, p)| {
            format!(
                "// Its shader was {}: shaders/{name}.wgsl is where it is written again.\n",
                p.strip_prefix(&unity.root).unwrap_or(&p).display()
            )
        })
        .unwrap_or_default();
    Ok(format!(
        "// From {}.\n{shader}({})\n",
        path.strip_prefix(&unity.root).unwrap_or(path).display(),
        fields.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAT: &str = "%YAML 1.1
--- !u!21 &2100000
Material:
  m_Name: Wet Stone
  m_Shader: {fileID: 4800000, guid: 933532a4fcc9baf4fa0491de14d08ed7, type: 3}
  m_SavedProperties:
    m_TexEnvs:
    - _BaseMap:
        m_Texture: {fileID: 2800000, guid: ttt, type: 3}
        m_Scale: {x: 2, y: 2}
        m_Offset: {x: 0, y: 0}
    m_Floats:
    - _Metallic: 0.1
    - _Smoothness: 0.8
    - _Surface: 0
    - _Cull: 0
    m_Colors:
    - _BaseColor: {r: 0.5, g: 0.5, b: 0.5, a: 1}
    - _EmissionColor: {r: 2, g: 1, b: 0, a: 1}
";

    #[test]
    fn a_shaders_declared_params_are_filled_from_the_material() {
        assert_eq!(
            declared_params(
                "// Light.\n// scrap:params _Metallic _BaseColor.r _Nothing\nfn surface() {}"
            ),
            ["_Metallic", "_BaseColor.r", "_Nothing"]
        );
        let doc = yaml::documents(MAT)
            .into_iter()
            .find(|d| d.kind == "Material")
            .unwrap();
        let names: Vec<String> = ["_Metallic", "_EmissionColor.g", "_Nothing"]
            .map(String::from)
            .to_vec();
        assert_eq!(param_values(&doc.body, &names), [0.1, 1.0, 0.0]);
    }

    #[test]
    fn a_custom_shader_says_how_it_is_drawn() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-look-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let graph = dir.join("Hologram.shadergraph");
        std::fs::write(
            &graph,
            "{\n    \"m_Type\": \"UnityEditor.Rendering.Universal.ShaderGraph.UniversalUnlitSubTarget\"\n}\n\n{\n    \"m_SurfaceType\": 1,\n    \"m_RenderFace\": 0,\n    \"m_AlphaClip\": true,\n}\n",
        )
        .unwrap();
        assert_eq!(
            shader_look(&graph),
            ShaderLook {
                transparent: true,
                face: Some("BothAsFront"),
                clip: true,
                unlit: true,
                on_top: false,
            }
        );
        let hlsl = dir.join("Water.shader");
        std::fs::write(
            &hlsl,
            "Shader \"Water\" { SubShader { Tags { \"Queue\"=\"Transparent\" \"LightMode\"=\"UniversalForward\" } Cull Off Blend SrcAlpha OneMinusSrcAlpha } }",
        )
        .unwrap();
        let look = shader_look(&hlsl);
        assert!(
            look.transparent && look.face == Some("BothAsFront") && !look.unlit,
            "{look:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shader_graphs_material_hands_it_the_textures_the_graph_reads() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-textures-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The graph exposes `_Road` and `_Normal` and holds a texture in
        // the node 5f1339acdb164c14b236042c20de1ef1; `_Stale` is from a
        // shader the material had before, and `_Empty` is set to nothing.
        let graph = dir.join("Landscape_Shader.shadergraph");
        std::fs::write(
            &graph,
            "{\n    \"m_DefaultReferenceName\": \"_Road\"\n}\n\n{\n    \"m_DefaultReferenceName\": \"_Normal\"\n}\n\n{\n    \"m_ObjectId\": \"5f1339acdb164c14b236042c20de1ef1\"\n}\n",
        )
        .unwrap();
        let mat = dir.join("M_Landscape.mat");
        std::fs::write(
            &mat,
            "%YAML 1.1
--- !u!21 &2100000
Material:
  m_Name: M_Landscape
  m_Shader: {fileID: -6465566751694194690, guid: ggg, type: 3}
  m_SavedProperties:
    m_TexEnvs:
    - _Empty:
        m_Texture: {fileID: 0}
    - _Road:
        m_Texture: {fileID: 2800000, guid: road, type: 3}
    - _SampleTexture2D_5f1339acdb164c14b236042c20de1ef1_Texture_1_Texture2D:
        m_Texture: {fileID: 2800000, guid: noise, type: 3}
    - _Stale:
        m_Texture: {fileID: 2800000, guid: fridge, type: 3}
    - _Normal:
        m_Texture: {fileID: 2800000, guid: bumps, type: 3}
    m_Colors:
    - _Color: {r: 1, g: 1, b: 1, a: 1}
",
        )
        .unwrap();
        let file = |name: &str| dir.join(name);
        let unity = Unity {
            layers: Default::default(),
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            declared_params: Default::default(),
            root: dir.clone(),
            guids: [
                ("ggg", file("Landscape_Shader.shadergraph")),
                ("mmm", mat.clone()),
                ("road", file("T_Road.tga")),
                ("noise", file("T_Noise.tga")),
                ("fridge", file("T_Fridge.png")),
                ("bumps", file("T_Bumps.png")),
            ]
            .into_iter()
            .map(|(g, p)| (g.to_string(), p))
            .collect(),
            names: [
                ("road", "T_Road"),
                ("noise", "T_Noise"),
                ("fridge", "T_Fridge"),
                ("bumps", "T_Bumps"),
            ]
            .into_iter()
            .map(|(g, n)| (g.to_string(), n.to_string()))
            .collect(),
        };
        let text = convert(&unity, &mat).unwrap();
        assert!(text.contains(r#"shader: "landscape_shader""#), "{text}");
        let source: crate::MaterialSource = ron::from_str(
            text.lines()
                .filter(|l| !l.starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        )
        .unwrap();
        let textures: Vec<(&str, &str)> = source
            .textures
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            textures,
            [
                ("_Normal", "T_Bumps"),
                ("_Road", "T_Road"),
                ("_SampleTexture2D_5f1339acdb164c14b236042c20de1ef1_Texture_1_Texture2D", "T_Noise"),
            ],
            "{text}"
        );
        // And they are copied over: the colour ones as colour, the normal
        // map as data; the stale one not at all.
        let (used, data) = textures_used(&unity);
        assert!(["road", "noise", "bumps"].iter().all(|g| used.contains(*g)), "{used:?}");
        assert!(!used.contains("fridge"), "{used:?}");
        assert_eq!(data.into_iter().collect::<Vec<_>>(), ["bumps"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_builtin_particle_material_is_read_back_unlit() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-unlit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let smoke = dir.join("Smoke.mat");
        std::fs::write(
            &smoke,
            "%YAML 1.1
--- !u!21 &2100000
Material:
  m_Name: Smoke
  m_Shader: {fileID: 211, guid: 0000000000000000f000000000000000, type: 0}
  m_SavedProperties:
    m_TexEnvs: []
    m_Floats:
    - _Surface: 1
    m_Colors: []
",
        )
        .unwrap();
        let unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            layers: Default::default(),
            declared_params: Default::default(),
            root: dir.clone(),
            guids: Default::default(),
            names: Default::default(),
        };
        let text = convert(&unity, &smoke).unwrap();
        let written = dir.join("Smoke.scrmat");
        std::fs::write(&written, &text).unwrap();
        // What the importer writes is what the material reads: a field it
        // does not know would be dropped without a word, and the smoke lit.
        assert!(crate::material_source(&written).unwrap().unlit, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_builtin_checker_is_made_again_and_tiled() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-checker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let checker = dir.join("Checker.mat");
        std::fs::write(
            &checker,
            "%YAML 1.1
--- !u!21 &2100000
Material:
  m_Name: Checker
  m_Shader: {fileID: 4800000, guid: 933532a4fcc9baf4fa0491de14d08ed7, type: 3}
  m_SavedProperties:
    m_TexEnvs:
    - _BaseMap:
        m_Texture: {fileID: 10309, guid: 0000000000000000f000000000000000, type: 0}
        m_Scale: {x: 1000, y: 1000}
        m_Offset: {x: 0, y: 0}
    m_Floats: []
    m_Colors: []
",
        )
        .unwrap();
        let mut unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            layers: Default::default(),
            declared_params: Default::default(),
            root: dir.clone(),
            guids: Default::default(),
            names: Default::default(),
        };
        unity.guids.insert("checker".into(), checker.clone());
        unity.names.insert("checker".into(), "Checker".into());
        let text = convert(&unity, &checker).unwrap();
        assert!(text.contains("base_map: \"Default-Checker-Gray\""), "{text}");
        assert!(text.contains("tiling: (1000, 1000)"), "{text}");
        assert_eq!(builtin_textures_used(&unity).into_iter().collect::<Vec<_>>(), [10309]);
        // Unity's: two by two squares of sRGB grey.
        let (_, picture) = builtin_texture(10309).unwrap();
        assert_eq!(picture.dimensions(), (64, 64));
        assert_eq!((picture.get_pixel(0, 0)[0], picture.get_pixel(40, 0)[0], picture.get_pixel(40, 40)[0]), (196, 157, 196));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_material_variant_keeps_what_its_parent_sets() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-variant-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let parent = dir.join("Parent.mat");
        std::fs::write(&parent, MAT).unwrap();
        let child = dir.join("Child.mat");
        std::fs::write(
            &child,
            "%YAML 1.1
--- !u!21 &2100000
Material:
  m_Name: Child
  m_Shader: {fileID: 0}
  m_Parent: {fileID: 2100000, guid: ppp, type: 2}
  m_SavedProperties:
    m_TexEnvs: []
    m_Floats:
    - _Smoothness: 0.2
    m_Colors: []
",
        )
        .unwrap();
        let unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            layers: Default::default(),
            declared_params: Default::default(),
            root: dir.clone(),
            guids: [
                ("ttt".to_string(), dir.join("stone_albedo.png")),
                ("ppp".to_string(), parent.clone()),
            ]
            .into_iter()
            .collect(),
            names: [("ttt".to_string(), "stone_albedo".to_string())]
                .into_iter()
                .collect(),
        };
        let text = convert(&unity, &child).unwrap();
        assert!(text.contains(r#"base_map: "stone_albedo""#), "{text}");
        assert!(text.contains(r##"color: "#808080""##), "{text}");
        assert!(text.contains("smoothness: 0.2"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_urp_lit_material_becomes_an_rmat_that_builds() {
        let dir = std::env::temp_dir().join(format!("scrap-unity-mat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Wet Stone.mat");
        std::fs::write(&path, MAT).unwrap();
        let unity = Unity {
            pieces: Default::default(),
            mesh_pieces: Default::default(),
            layers: Default::default(),
            declared_params: Default::default(),
            root: dir.clone(),
            guids: [("ttt".to_string(), dir.join("stone_albedo.png"))]
                .into_iter()
                .collect(),
            names: [("ttt".to_string(), "stone_albedo".to_string())]
                .into_iter()
                .collect(),
        };
        let text = convert(&unity, &path).unwrap();
        assert!(text.contains(r##"color: "#808080""##), "{text}");
        assert!(text.contains("smoothness: 0.8"), "{text}");
        assert!(text.contains(r#"base_map: "stone_albedo""#), "{text}");
        assert!(text.contains("tiling: (2, 2)"), "{text}");
        assert!(text.contains("render_face: BothAsFront"), "{text}");
        assert!(text.contains("emission_intensity: 2"), "{text}");
        let source: crate::MaterialSource = ron::from_str(
            text.lines()
                .filter(|l| !l.starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        )
        .unwrap();
        assert_eq!(source.smoothness, 0.8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
