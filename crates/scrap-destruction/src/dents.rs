//! Surfaces that dent: a line's `dents`. `dents: (depth: 0.12)` on a
//! thing drawn from a builtin model — a car's body, a barrel, a locker —
//! pushes its surface in where it is struck, as deep as the blow was hard
//! and wider for a harder one; blow after blow adds up. The vertices move
//! (a car's crumpled wing in a racing game); the collider stays as it was.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use scrap_core::world::WorldTransform;
use scrap_geometry::mesh_asset::Vertex;

use crate::fracture::Blow;

/// A surface that dents, as a scene line writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dents {
    /// Metres the surface goes in under a blow of `strength` metres a
    /// second; harder goes deeper, to three times this.
    pub depth: f32,
    /// Metres across a dent, at `strength`.
    pub radius: f32,
    /// Metres a second: a blow slower than a tenth of this leaves no mark.
    pub strength: f32,
    /// How it goes over the network (docs/netsim.md): `Local` unless
    /// the line says.
    #[serde(default, skip_serializing_if = "scrap_core::netsim::NetMode::is_local")]
    pub net: scrap_core::netsim::NetMode,
}

impl Default for Dents {
    fn default() -> Self {
        Self { depth: 0.08, radius: 0.3, strength: 6.0, net: scrap_core::netsim::NetMode::Local }
    }
}

scrap_core::impl_parts! {
    Dents => "dents";
}

/// The dents of a line, read off it.
pub trait DentsLine {
    fn dents(&self) -> Option<Dents>;
}

impl DentsLine for scrap_core::EntityDesc {
    fn dents(&self) -> Option<Dents> {
        self.part()
    }
}

impl DentsLine for scrap_core::scene::Override {
    fn dents(&self) -> Option<Dents> {
        self.part()
    }
}

/// A dented surface: its mesh as it is now, in its own space.
#[derive(Debug, Clone)]
pub struct Dented {
    pub dents: Dents,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Changed since it was last drawn.
    pub fresh: bool,
    /// How many blows it has taken.
    pub blows: u32,
    /// Every blow it has taken, in its own space: what an `Event` dent
    /// tells everyone else, and what someone joining late is dented by.
    pub history: Vec<(Vec3, Vec3, f32)>,
}

impl Dented {
    pub fn new(dents: Dents, vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self { dents, vertices, indices, fresh: true, blows: 0, history: Vec::new() }
    }

    /// A blow at `at` going `way` (in its own space), at `speed`.
    pub fn strike(&mut self, at: Vec3, way: Vec3, speed: f32) {
        let d = self.dents;
        let hard = (speed / d.strength.max(1e-3)).clamp(0.0, 3.0);
        if hard < 0.1 {
            return;
        }
        let depth = d.depth * hard;
        let radius = d.radius * hard.sqrt().max(0.5);
        let way = way.normalize_or(Vec3::NEG_Y);
        for v in &mut self.vertices {
            let p = Vec3::from_array(v.position);
            let r = p.distance(at) / radius;
            if r >= 1.0 {
                continue;
            }
            // A smooth bowl: deepest at the blow, nothing at its rim.
            let fall = (1.0 - r * r) * (1.0 - r * r);
            v.position = (p + way * depth * fall).to_array();
        }
        // Lit as it is bent: each normal from the triangles round it.
        let mut normals = vec![Vec3::ZERO; self.vertices.len()];
        for t in self.indices.chunks_exact(3) {
            let at = |i: u32| Vec3::from_array(self.vertices[i as usize].position);
            let n = (at(t[1]) - at(t[0])).cross(at(t[2]) - at(t[0]));
            for i in t {
                normals[*i as usize] += n;
            }
        }
        for (v, n) in self.vertices.iter_mut().zip(normals) {
            v.normal = n.normalize_or(Vec3::from_array(v.normal)).to_array();
        }
        self.fresh = true;
        self.blows += 1;
        if self.history.len() < MOST_TOLD {
            self.history.push((at, way, speed));
        }
    }
}

/// Dent what `blows` struck.
pub fn run_dents(world: &mut hecs::World, blows: &[Blow]) {
    for blow in blows {
        // Someone else's `Event` dents: told by its owner, not struck here.
        let told = world.get::<&scrap_core::world::Replica>(blow.entity).is_ok()
            && world.get::<&Dented>(blow.entity).is_ok_and(|d| d.dents.net == scrap_core::netsim::NetMode::Event);
        if told {
            continue;
        }
        let Ok(placed) = world.get::<&WorldTransform>(blow.entity).map(|p| p.0) else { continue };
        let Ok(mut dented) = world.get::<&mut Dented>(blow.entity) else { continue };
        let back: Mat4 = placed.inverse();
        dented.strike(back.transform_point3(blow.at), back.transform_vector3(blow.way), blow.speed);
    }
}

/// A builtin model's mesh subdivided until no edge is longer than `most`
/// (in its own space): a cube of 12 triangles has nothing to bend.
pub fn finely(vertices: &[Vertex], indices: &[u32], most: f32) -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = vertices.to_vec();
    let mut indices = indices.to_vec();
    for _ in 0..6 {
        let longest = indices
            .chunks_exact(3)
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .map(|(a, b)| Vec3::from_array(vertices[a as usize].position).distance(Vec3::from_array(vertices[b as usize].position)))
            .fold(0.0, f32::max);
        if longest <= most {
            break;
        }
        let mut next = Vec::with_capacity(indices.len() * 4);
        let mut middles: std::collections::HashMap<(u32, u32), u32> = Default::default();
        let mut middle = |a: u32, b: u32, vertices: &mut Vec<Vertex>| {
            *middles.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let (va, vb) = (vertices[a as usize], vertices[b as usize]);
                let mix = |x: [f32; 3], y: [f32; 3]| [(x[0] + y[0]) * 0.5, (x[1] + y[1]) * 0.5, (x[2] + y[2]) * 0.5];
                vertices.push(Vertex {
                    position: mix(va.position, vb.position),
                    normal: mix(va.normal, vb.normal),
                    uv: [(va.uv[0] + vb.uv[0]) * 0.5, (va.uv[1] + vb.uv[1]) * 0.5],
                });
                vertices.len() as u32 - 1
            })
        };
        for t in indices.chunks_exact(3) {
            let (a, b, c) = (t[0], t[1], t[2]);
            let (ab, bc, ca) = (middle(a, b, &mut vertices), middle(b, c, &mut vertices), middle(c, a, &mut vertices));
            next.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
        }
        indices = next;
    }
    (vertices, indices)
}

/// The destruction module's dresser for dents: a builtin model made fine
/// enough to bend.
pub struct DentsDress;

impl scrap_core::world::Dress for DentsDress {
    fn parts(&self) -> &[&'static str] {
        &["dents", "model"]
    }

    fn dress(
        &mut self,
        line: &scrap_core::EntityDesc,
        entity: hecs::Entity,
        world: &mut hecs::World,
        _: scrap_core::world::Changed,
        _: &mut Vec<scrap_core::world::Unresolved>,
    ) {
        use scrap_geometry::line::GeometryLine;
        let model = line.dents().and_then(|d| Some((d, scrap_geometry::builtin::by_name(line.model().as_str())?)));
        match model {
            Some((dents, mesh)) => {
                let (vertices, indices) = finely(&mesh.vertices, &mesh.indices, 0.08);
                let _ = world.insert_one(entity, Dented::new(dents, vertices, indices));
            }
            None => {
                scrap_core::world::take_off::<Dented>(world, entity);
            }
        }
    }
}


/// The most blows a dented thing tells: past this it has been dented
/// enough, and the list would outgrow a datagram.
pub const MOST_TOLD: usize = 48;

/// Dents' state for the network (`Components::register_state`): every
/// blow taken, from the owner, when it is `Event`.
pub fn gather_net(world: &hecs::World, entity: hecs::Entity) -> Option<Vec<u8>> {
    let dented = world.get::<&Dented>(entity).ok()?;
    if dented.dents.net != scrap_core::netsim::NetMode::Event || world.get::<&scrap_core::world::Replica>(entity).is_ok() || dented.history.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(dented.history.len() * 28);
    for (at, way, speed) in &dented.history {
        for x in [at.x, at.y, at.z, way.x, way.y, way.z, *speed] {
            out.extend_from_slice(&x.to_le_bytes());
        }
    }
    Some(out)
}

/// The owner's blows onto everyone else's: those not taken here yet.
pub fn take_net(world: &mut hecs::World, entity: hecs::Entity, _sender: u32, _tick: u64, bytes: &[u8]) {
    let Ok(mut dented) = world.get::<&mut Dented>(entity) else { return };
    let f = |i: usize| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap_or_default());
    let told = bytes.len() / 28;
    for k in dented.history.len()..told {
        let b = k * 7;
        dented.strike(Vec3::new(f(b), f(b + 1), f(b + 2)), Vec3::new(f(b + 3), f(b + 4), f(b + 5)), f(b + 6));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blow_pushes_the_surface_in_where_it_lands_and_a_harder_one_deeper() {
        let cube = scrap_geometry::builtin::cube(1.0);
        let (vertices, indices) = finely(&cube.vertices, &cube.indices, 0.1);
        assert!(vertices.len() > 500, "fine enough to bend: {}", vertices.len());
        let mut soft = Dented::new(Dents::default(), vertices.clone(), indices.clone());
        let mut hard = Dented::new(Dents::default(), vertices, indices);
        // Struck on its top, straight down.
        soft.strike(Vec3::new(0.0, 0.5, 0.0), Vec3::NEG_Y, 6.0);
        hard.strike(Vec3::new(0.0, 0.5, 0.0), Vec3::NEG_Y, 15.0);
        let lowest_top = |d: &Dented| {
            d.vertices
                .iter()
                .filter(|v| v.normal[1] > 0.5 && v.position[0].abs() < 0.05 && v.position[2].abs() < 0.05)
                .map(|v| v.position[1])
                .fold(f32::MAX, f32::min)
        };
        assert!((lowest_top(&soft) - (0.5 - 0.08)).abs() < 0.01, "{}", lowest_top(&soft));
        assert!(lowest_top(&hard) < lowest_top(&soft) - 0.05);
        // Its corners are where they were.
        assert!(soft.vertices.iter().any(|v| Vec3::from_array(v.position).distance(Vec3::splat(0.5)) < 1e-5));
        // A tap leaves nothing.
        let before = soft.vertices.clone();
        soft.strike(Vec3::new(0.3, 0.5, 0.3), Vec3::NEG_Y, 0.3);
        assert_eq!(before, soft.vertices);
    }
}
