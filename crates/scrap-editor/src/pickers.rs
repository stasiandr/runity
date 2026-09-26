//! What the Inspector's pickers offer, beyond the assets a link names by
//! name ([`Session::assets_of_kind`]): assets a value holds by ID alone — a
//! material's maps and shader — with their names, the entities of the
//! scene with where each is, and the bones an entity can ride on.
//!
//! A person picks by name and never sees the ID; the value keeps what the
//! engine keeps.

use scrap::asset::AssetId;
use scrap::mesh_asset::TextureLibrary;
#[allow(unused_imports)]
use scrap::prelude::*;
use scrap::scene::EntityDesc;
use scrap::EntityId;

use crate::{EditError, EditResult, Session};

/// One entity a picker offers: its id, its name, and the names of the
/// lines above it (`gantry beam / pivot`), empty at the top.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityChoice {
    pub id: EntityId,
    pub name: String,
    pub path: String,
}

impl Session {
    /// Every asset of a kind a value names by its ID alone, with the name
    /// a person knows it by, sorted by name: `texture` — the library's
    /// images — and `shader`, the project's `shaders/*.wgsl`.
    pub fn asset_ids_of_kind(&self, kind: &str) -> Vec<(String, AssetId)> {
        let mut out: Vec<(String, AssetId)> = match kind {
            "texture" => self
                .library
                .as_ref()
                .map(|l| {
                    l.names_of(scrap::asset::TEXTURE)
                        .filter_map(|n| {
                            Some((n.to_string(), AssetId::from(&l.texture_by_name(n)?.id)))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            "shader" => self
                .project
                .as_ref()
                .and_then(|p| std::fs::read_dir(p.root().join(scrap::project::SHADERS)).ok())
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    // `water.wgsl` and a shader graph `lava.graph.ron` alike.
                    let name = scrap::render::material_shader_name(&e.path())?;
                    let id = scrap::asset::shader_id(&name);
                    Some((name, id))
                })
                .collect(),
            _ => Vec::new(),
        };
        out.sort();
        out.dedup();
        out
    }

    /// The name of the asset of `kind` with this ID, when there is one.
    pub fn asset_name_of_id(&self, kind: &str, id: AssetId) -> Option<String> {
        self.asset_ids_of_kind(kind)
            .into_iter()
            .find(|(_, i)| *i == id)
            .map(|(n, _)| n)
    }

    /// A texture's picture, `size` pixels square, RGBA: its pixels sampled
    /// down, what a picker shows beside its name.
    pub fn texture_picture(&self, name: &str, size: u32) -> EditResult<Vec<u8>> {
        let texture = self
            .library
            .as_ref()
            .and_then(|l| l.texture_by_name(name))
            .ok_or_else(|| EditError::Scene(format!("no texture named `{name}`")))?;
        let (w, h) = (texture.width.to_native(), texture.height.to_native());
        let pixels = texture.pixels.as_slice();
        let size = size.clamp(1, 512);
        let mut out = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let (sx, sy) = (x * w.max(1) / size, y * h.max(1) / size);
                let at = ((sy * w + sx) * 4) as usize;
                out.extend_from_slice(pixels.get(at..at + 4).unwrap_or(&[0, 0, 0, 255]));
            }
        }
        Ok(out)
    }

    /// Every entity of the scene as it is drawn — a prefab's parts too —
    /// in the Hierarchy's order, with the lines above it: what an entity
    /// field's picker lists.
    pub fn entity_choices(&self) -> Vec<EntityChoice> {
        fn walk(entities: &[EntityDesc], path: &str, out: &mut Vec<EntityChoice>) {
            for e in entities {
                out.push(EntityChoice {
                    id: e.id,
                    name: e.name.clone(),
                    path: path.to_string(),
                });
                let below = if path.is_empty() {
                    e.name.clone()
                } else {
                    format!("{path} / {}", e.name)
                };
                walk(&e.children, &below, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.expanded().entities, "", &mut out);
        out
    }

    /// The bones an entity can be held by (its `bone`): the joints of the
    /// skeleton of the model its parent draws, in the skeleton's order.
    /// Empty when the parent draws nothing skinned, or it has no parent.
    pub fn bone_names(&self, id: EntityId) -> Vec<String> {
        fn parent_of(entities: &[EntityDesc], id: EntityId) -> Option<EntityId> {
            for e in entities {
                if e.children.iter().any(|c| c.id == id) {
                    return Some(e.id);
                }
                if let Some(p) = parent_of(&e.children, id) {
                    return Some(p);
                }
            }
            None
        }
        let Some(parent) = parent_of(&self.expanded().entities, id) else {
            return Vec::new();
        };
        let Some(model) = self.entity_model(parent).or_else(|| {
            self.expanded()
                .get(parent)
                .map(|l| l.model().to_string())
                .filter(|m| !m.is_empty())
        }) else {
            return Vec::new();
        };
        self.library
            .as_ref()
            .and_then(|l| l.mesh_by_name(&model))
            .and_then(|m| m.skin_owned())
            .map(|skin| skin.skeleton.joints.into_iter().map(|j| j.name).collect())
            .unwrap_or_default()
    }
}
