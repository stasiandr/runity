//! URP Lit materials (`.mat`) as `.rmat`.

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

fn float(m: &Yaml, name: &str) -> Option<f32> {
    property(m, "m_Floats", name)
        .and_then(yaml::number)
        .map(|n| n as f32)
}

fn color(m: &Yaml, name: &str) -> Option<[f32; 4]> {
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

/// Which texture slots come over, and what they are called in `.rmat`.
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
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for doc in yaml::documents(&text) {
            let mut base = false;
            for (slot, field) in MAPS {
                if let Some((guid, _)) = texture(&doc.body, slot) {
                    if matches!(field, "base_map" | "emission_map") {
                        colour.insert(guid.clone());
                    }
                    base |= field == "base_map";
                    used.insert(guid);
                }
            }
            if !base && own_shader(unity, &doc.body).is_some() {
                if let Some(guid) = own_texture(unity, &doc.body) {
                    colour.insert(guid.clone());
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
            // URP: 0 both, 1 back, 2 front.
            face: match number("m_RenderFace").as_deref() {
                Some("0") => Some("Both"),
                Some("1") => Some("Back"),
                _ => None,
            },
            clip: number("m_AlphaClip").as_deref() == Some("true"),
            unlit: text.contains("UniversalUnlitSubTarget")
                || text.contains("UniversalSpriteUnlitSubTarget"),
        }
    } else {
        let lower = text.to_lowercase();
        ShaderLook {
            transparent: lower.contains("\"queue\"=\"transparent")
                || lower.contains("blend srcalpha"),
            face: lower.contains("cull off").then_some("Both"),
            clip: false,
            unlit: !lower.contains("lightmode\"=\"universalforward"),
        }
    }
}

/// The material's own shader — a Shader Graph or a `.shader` in the
/// project, not one of URP's — as the name a `.rmat` gives it and where
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

/// A `.mat` as the text of a `.rmat`: what URP Lit says, with the rest of
/// a custom shader's colour carried as far as it goes.
pub fn convert(unity: &Unity, path: &Path) -> Result<String> {
    let text = std::fs::read_to_string(path).with_context(|| format!("{}", path.display()))?;
    let doc = yaml::documents(&text)
        .into_iter()
        .find(|d| d.kind == "Material")
        .context("no Material in it")?;
    let m = &doc.body;
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
    if float(m, "_AlphaClip") == Some(1.0) {
        fields.push(format!(
            "alpha_clip: {}",
            float(m, "_Cutoff").unwrap_or(0.5)
        ));
    }
    match float(m, "_Cull") {
        Some(0.0) => fields.push("render_face: Both".into()),
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
        let Some((kind, name)) = unity.named(&guid) else {
            continue;
        };
        if kind != "texture" {
            continue;
        }
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
        if let Some((_, name)) = own_texture(unity, m).and_then(|g| unity.named(&g)) {
            fields.push(format!("base_map: {name:?}"));
        }
    }
    if let Some((name, path)) = &own {
        fields.push(format!("shader: {name:?}"));
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
            fields.push("shading: Unlit".into());
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
    fn a_custom_shader_says_how_it_is_drawn() {
        let dir = std::env::temp_dir().join(format!("runity-unity-look-{}", std::process::id()));
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
                face: Some("Both"),
                clip: true,
                unlit: true,
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
            look.transparent && look.face == Some("Both") && !look.unlit,
            "{look:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_urp_lit_material_becomes_an_rmat_that_builds() {
        let dir = std::env::temp_dir().join(format!("runity-unity-mat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Wet Stone.mat");
        std::fs::write(&path, MAT).unwrap();
        let unity = Unity {
            layers: Default::default(),
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
        assert!(text.contains("render_face: Both"), "{text}");
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
