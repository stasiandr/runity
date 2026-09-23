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
            for (slot, field) in MAPS {
                if let Some((guid, _)) = texture(&doc.body, slot) {
                    if matches!(field, "base_map" | "emission_map") {
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
    let shader = m
        .reference("m_Shader")
        .and_then(|r| r.guid)
        .and_then(|g| unity.guids.get(&g))
        .map(|p| {
            format!(
                "// Its shader was {}: only its colours came over.\n",
                super::stem(p)
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
