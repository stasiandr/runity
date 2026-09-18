//! Transforms that move together.
//!
//! A cart's wheels move with the cart, a torch moves with the hand that holds
//! it, a building's scaffolding moves when the building is placed. Doing that
//! by recomputing every matrix from scratch every frame works until there are
//! a few thousand of them; doing it by hand — "when the cart moves, remember
//! to move the wheels" — stops working the first time somebody forgets.
//!
//! So: a tree of local transforms, world matrices computed from them, and a
//! dirty flag so that only what actually moved is recomputed. The dirty flag
//! is the whole point. Without it this is a more complicated way of doing the
//! same work; with it, a world of ten thousand mostly-still objects costs
//! whatever moved.

use runity_math::{Mat4, Vec3};

use crate::transform::Transform;
use crate::world::Entity;

/// A place in the hierarchy.
///
/// Generational, like [`Entity`]: removing a node and adding another must not
/// let an old handle move the newcomer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Node {
    index: u32,
    generation: u32,
}

impl Node {
    /// Slot number.
    pub fn index(self) -> u32 {
        self.index
    }

    /// How many times the slot has been reused.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Debug)]
struct NodeData {
    local: Transform,
    world: Mat4,
    parent: Option<Node>,
    children: Vec<Node>,
    entity: Option<Entity>,
    generation: u32,
    alive: bool,
    /// Whether `world` needs recomputing from `local` and the parent.
    dirty: bool,
}

/// A tree of transforms.
#[derive(Debug, Default)]
pub struct Scene {
    nodes: Vec<NodeData>,
    free: Vec<u32>,
    roots: Vec<Node>,
}

impl Scene {
    /// An empty scene.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node at the top level.
    pub fn spawn(&mut self, local: Transform) -> Node {
        let data = NodeData {
            local,
            world: Mat4::IDENTITY,
            parent: None,
            children: Vec::new(),
            entity: None,
            generation: 0,
            alive: true,
            dirty: true,
        };
        let node = match self.free.pop() {
            Some(index) => {
                let slot = index as usize;
                let generation = self.nodes[slot].generation;
                self.nodes[slot] = NodeData { generation, ..data };
                Node { index, generation }
            }
            None => {
                self.nodes.push(data);
                Node {
                    index: (self.nodes.len() - 1) as u32,
                    generation: 0,
                }
            }
        };
        self.roots.push(node);
        node
    }

    /// Add a node as a child of another.
    pub fn spawn_child(&mut self, parent: Node, local: Transform) -> Node {
        let node = self.spawn(local);
        self.attach(node, parent);
        node
    }

    /// Whether a handle still refers to a live node.
    pub fn contains(&self, node: Node) -> bool {
        self.nodes
            .get(node.index as usize)
            .is_some_and(|data| data.alive && data.generation == node.generation)
    }

    /// How many nodes there are.
    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|data| data.alive).count()
    }

    /// Whether the scene is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Nodes with no parent, in creation order.
    pub fn roots(&self) -> &[Node] {
        &self.roots
    }

    /// The node's parent, if it has one.
    pub fn parent(&self, node: Node) -> Option<Node> {
        self.data(node).and_then(|data| data.parent)
    }

    /// The node's children, in the order they were attached.
    pub fn children(&self, node: Node) -> &[Node] {
        match self.data(node) {
            Some(data) => &data.children,
            None => &[],
        }
    }

    /// The entity this node stands for, if any.
    pub fn entity(&self, node: Node) -> Option<Entity> {
        self.data(node).and_then(|data| data.entity)
    }

    /// Point a node at an entity, so a system walking the tree can find it.
    pub fn set_entity(&mut self, node: Node, entity: Entity) {
        if let Some(data) = self.data_mut(node) {
            data.entity = Some(entity);
        }
    }

    /// The node's own transform, relative to its parent.
    pub fn local(&self, node: Node) -> Option<&Transform> {
        self.data(node).map(|data| &data.local)
    }

    /// Move a node, marking everything under it as needing recomputation.
    pub fn set_local(&mut self, node: Node, local: Transform) {
        if let Some(data) = self.data_mut(node) {
            data.local = local;
        }
        self.mark_dirty(node);
    }

    /// Change a node's transform in place.
    ///
    /// Anything taken this way is assumed to have been written to, so the
    /// subtree is marked dirty — the alternative is a caller that moves a
    /// cart and wonders why the wheels stayed.
    pub fn local_mut(&mut self, node: Node) -> Option<&mut Transform> {
        if !self.contains(node) {
            return None;
        }
        self.mark_dirty(node);
        self.nodes
            .get_mut(node.index as usize)
            .map(|data| &mut data.local)
    }

    /// The node's transform in world space, recomputing what has changed.
    pub fn world(&mut self, node: Node) -> Option<Mat4> {
        if !self.contains(node) {
            return None;
        }
        self.refresh(node);
        self.data(node).map(|data| data.world)
    }

    /// Where the node is in the world.
    pub fn world_position(&mut self, node: Node) -> Option<Vec3> {
        self.world(node).map(|matrix| {
            let column = matrix.cols[3];
            Vec3::new(column.x, column.y, column.z)
        })
    }

    /// Recompute every world matrix that needs it.
    ///
    /// Call once a frame before drawing. Nodes that did not move cost nothing.
    pub fn update(&mut self) {
        let roots = self.roots.clone();
        for root in roots {
            self.refresh_subtree(root, Mat4::IDENTITY, false);
        }
    }

    /// Attach a node to a parent, keeping its local transform.
    ///
    /// Refuses to make a cycle: attaching a node to its own descendant would
    /// produce a tree that no traversal terminates on.
    pub fn attach(&mut self, child: Node, parent: Node) -> bool {
        if !self.contains(child) || !self.contains(parent) || child == parent {
            return false;
        }
        if self.is_ancestor(child, parent) {
            return false;
        }
        self.detach(child);
        if let Some(data) = self.data_mut(child) {
            data.parent = Some(parent);
        }
        if let Some(data) = self.data_mut(parent) {
            data.children.push(child);
        }
        self.roots.retain(|root| *root != child);
        self.mark_dirty(child);
        true
    }

    /// Attach while keeping the node where it is in the world.
    ///
    /// What you want when a villager picks something up: the object should
    /// stay where it is and then follow the hand, not jump to it.
    pub fn attach_keeping_world(&mut self, child: Node, parent: Node) -> bool {
        let Some(child_world) = self.world(child) else {
            return false;
        };
        let Some(parent_world) = self.world(parent) else {
            return false;
        };
        if !self.attach(child, parent) {
            return false;
        }
        // A parent with a zero scale cannot be undone; leave the child where
        // the plain attach put it rather than producing infinities.
        let Some(inverse) = parent_world.inverse() else {
            return true;
        };
        let local = inverse * child_world;
        if let Some(data) = self.data_mut(child) {
            data.local = Transform::from_matrix(local);
        }
        self.mark_dirty(child);
        true
    }

    /// Detach a node from its parent, making it a root.
    pub fn detach(&mut self, child: Node) {
        let Some(parent) = self.parent(child) else {
            return;
        };
        if let Some(data) = self.data_mut(parent) {
            data.children.retain(|node| *node != child);
        }
        if let Some(data) = self.data_mut(child) {
            data.parent = None;
        }
        if !self.roots.contains(&child) {
            self.roots.push(child);
        }
        self.mark_dirty(child);
    }

    /// Remove a node and everything under it.
    ///
    /// Returns how many nodes went, which is rarely one: removing a cart takes
    /// its wheels with it, and leaving them behind is how a scene fills up
    /// with invisible orphans.
    pub fn remove(&mut self, node: Node) -> usize {
        if !self.contains(node) {
            return 0;
        }
        self.detach(node);
        self.roots.retain(|root| *root != node);

        let mut removed = 0;
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            let Some(data) = self.nodes.get_mut(current.index as usize) else {
                continue;
            };
            if !data.alive {
                continue;
            }
            data.alive = false;
            data.generation = data.generation.wrapping_add(1);
            data.entity = None;
            stack.extend(core::mem::take(&mut data.children));
            self.free.push(current.index);
            removed += 1;
        }
        removed
    }

    /// Whether `ancestor` is somewhere above `node`.
    pub fn is_ancestor(&self, ancestor: Node, node: Node) -> bool {
        let mut current = self.parent(node);
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            current = self.parent(parent);
        }
        false
    }

    /// Every live node, parents before children.
    ///
    /// The order is fixed, which matters as soon as anything derived from the
    /// walk is compared between machines.
    pub fn iter(&self) -> Vec<Node> {
        let mut out = Vec::with_capacity(self.len());
        let mut stack: Vec<Node> = self.roots.iter().rev().copied().collect();
        while let Some(node) = stack.pop() {
            if !self.contains(node) {
                continue;
            }
            out.push(node);
            stack.extend(self.children(node).iter().rev().copied());
        }
        out
    }

    fn data(&self, node: Node) -> Option<&NodeData> {
        self.nodes
            .get(node.index as usize)
            .filter(|data| data.alive && data.generation == node.generation)
    }

    fn data_mut(&mut self, node: Node) -> Option<&mut NodeData> {
        self.nodes
            .get_mut(node.index as usize)
            .filter(|data| data.alive && data.generation == node.generation)
    }

    /// Mark a node and everything below it as needing recomputation.
    fn mark_dirty(&mut self, node: Node) {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            let Some(data) = self.data_mut(current) else {
                continue;
            };
            if data.dirty {
                // Everything below is already marked, so there is nothing to
                // walk — which is what keeps moving a root cheap.
                continue;
            }
            data.dirty = true;
            stack.extend(data.children.iter().copied());
        }
    }

    /// Bring one node's world matrix up to date, and its ancestors' with it.
    fn refresh(&mut self, node: Node) {
        let mut chain = Vec::new();
        let mut current = Some(node);
        while let Some(step) = current {
            let Some(data) = self.data(step) else {
                break;
            };
            chain.push(step);
            if !data.dirty {
                break;
            }
            current = data.parent;
        }

        for step in chain.into_iter().rev() {
            let Some(data) = self.data(step) else {
                continue;
            };
            if !data.dirty {
                continue;
            }
            let parent_world = match data.parent {
                Some(parent) => self.data(parent).map_or(Mat4::IDENTITY, |data| data.world),
                None => Mat4::IDENTITY,
            };
            let local = data.local.matrix();
            if let Some(data) = self.data_mut(step) {
                data.world = parent_world * local;
                data.dirty = false;
            }
        }
    }

    /// Recompute a whole subtree, iteratively.
    fn refresh_subtree(&mut self, node: Node, parent_world: Mat4, force: bool) {
        let mut stack = vec![(node, parent_world, force)];
        while let Some((current, parent_world, force)) = stack.pop() {
            let Some(data) = self.data(current) else {
                continue;
            };
            let needs = force || data.dirty;
            let world = if needs {
                parent_world * data.local.matrix()
            } else {
                data.world
            };
            if needs {
                if let Some(data) = self.data_mut(current) {
                    data.world = world;
                    data.dirty = false;
                }
            }
            let children = self.children(current).to_vec();
            for child in children {
                stack.push((child, world, needs));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::{vec3, Quat};

    fn at(x: f32, y: f32, z: f32) -> Transform {
        Transform::from_position(vec3(x, y, z))
    }

    #[test]
    fn a_child_moves_with_its_parent() {
        let mut scene = Scene::new();
        let cart = scene.spawn(at(10.0, 0.0, 0.0));
        let wheel = scene.spawn_child(cart, at(1.0, 0.0, 0.0));

        assert_eq!(scene.world_position(wheel), Some(vec3(11.0, 0.0, 0.0)));

        scene.set_local(cart, at(20.0, 0.0, 0.0));
        assert_eq!(scene.world_position(wheel), Some(vec3(21.0, 0.0, 0.0)));
        assert_eq!(
            scene.local(wheel).unwrap().position,
            vec3(1.0, 0.0, 0.0),
            "the local is its own"
        );
    }

    #[test]
    fn rotation_and_scale_compose_down_the_tree() {
        let mut scene = Scene::new();
        let parent = scene.spawn(
            Transform::from_position(vec3(0.0, 0.0, 0.0))
                .with_rotation(Quat::from_axis_angle(Vec3::Y, core::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(2.0)),
        );
        let child = scene.spawn_child(parent, at(1.0, 0.0, 0.0));

        // A quarter turn about Y sends +X to -Z, and the scale doubles it.
        let position = scene.world_position(child).unwrap();
        assert!(position.x.abs() < 1e-5, "{position:?}");
        assert!((position.z + 2.0).abs() < 1e-5, "{position:?}");
    }

    #[test]
    fn three_generations_still_agree() {
        let mut scene = Scene::new();
        let a = scene.spawn(at(1.0, 0.0, 0.0));
        let b = scene.spawn_child(a, at(2.0, 0.0, 0.0));
        let c = scene.spawn_child(b, at(4.0, 0.0, 0.0));

        assert_eq!(scene.world_position(c), Some(vec3(7.0, 0.0, 0.0)));
        scene.set_local(a, at(-1.0, 5.0, 0.0));
        assert_eq!(scene.world_position(c), Some(vec3(5.0, 5.0, 0.0)));
        assert_eq!(scene.world_position(b), Some(vec3(1.0, 5.0, 0.0)));
    }

    #[test]
    fn only_what_moved_is_recomputed() {
        // The dirty flag is the entire reason this is not just a more
        // complicated way of multiplying every matrix every frame.
        let mut scene = Scene::new();
        let mut nodes = Vec::new();
        let root = scene.spawn(Transform::IDENTITY);
        for index in 0..100 {
            nodes.push(scene.spawn_child(root, at(index as f32, 0.0, 0.0)));
        }
        scene.update();
        assert!(scene.nodes.iter().all(|data| !data.alive || !data.dirty));

        scene.set_local(nodes[50], at(0.0, 9.0, 0.0));
        let dirty = scene
            .nodes
            .iter()
            .filter(|data| data.alive && data.dirty)
            .count();
        assert_eq!(dirty, 1, "moving one leaf should dirty one node");

        scene.set_local(root, at(1.0, 0.0, 0.0));
        let dirty = scene
            .nodes
            .iter()
            .filter(|data| data.alive && data.dirty)
            .count();
        assert_eq!(
            dirty, 101,
            "and moving the root dirties everything below it"
        );
    }

    #[test]
    fn asking_for_one_world_matrix_updates_only_its_ancestors() {
        let mut scene = Scene::new();
        let root = scene.spawn(at(1.0, 0.0, 0.0));
        let branch = scene.spawn_child(root, at(1.0, 0.0, 0.0));
        let leaf = scene.spawn_child(branch, at(1.0, 0.0, 0.0));
        let other = scene.spawn_child(root, at(9.0, 0.0, 0.0));
        scene.update();

        scene.set_local(root, at(2.0, 0.0, 0.0));
        assert_eq!(scene.world_position(leaf), Some(vec3(4.0, 0.0, 0.0)));
        // The sibling was not asked for, so it is still waiting.
        assert!(scene.data(other).unwrap().dirty);
        scene.update();
        assert!(!scene.data(other).unwrap().dirty);
        assert_eq!(scene.world_position(other), Some(vec3(11.0, 0.0, 0.0)));
    }

    #[test]
    fn taking_a_transform_mutably_marks_it_moved() {
        // The alternative is a caller who moves the cart and wonders why the
        // wheels stayed behind.
        let mut scene = Scene::new();
        let cart = scene.spawn(at(0.0, 0.0, 0.0));
        let wheel = scene.spawn_child(cart, at(1.0, 0.0, 0.0));
        scene.update();

        scene.local_mut(cart).unwrap().position = vec3(5.0, 0.0, 0.0);
        assert_eq!(scene.world_position(wheel), Some(vec3(6.0, 0.0, 0.0)));
    }

    #[test]
    fn reparenting_keeps_the_local_transform_by_default() {
        let mut scene = Scene::new();
        let first = scene.spawn(at(10.0, 0.0, 0.0));
        let second = scene.spawn(at(-10.0, 0.0, 0.0));
        let item = scene.spawn_child(first, at(1.0, 0.0, 0.0));
        assert_eq!(scene.world_position(item), Some(vec3(11.0, 0.0, 0.0)));

        scene.attach(item, second);
        assert_eq!(scene.world_position(item), Some(vec3(-9.0, 0.0, 0.0)));
        assert_eq!(scene.parent(item), Some(second));
        assert_eq!(scene.children(first), &[]);
        assert_eq!(scene.children(second), &[item]);
    }

    #[test]
    fn picking_something_up_does_not_teleport_it() {
        let mut scene = Scene::new();
        let hand = scene.spawn(at(3.0, 1.0, 0.0));
        let axe = scene.spawn(at(-2.0, 0.0, 4.0));
        let before = scene.world_position(axe).unwrap();

        assert!(scene.attach_keeping_world(axe, hand));
        let after = scene.world_position(axe).unwrap();
        assert!(
            (after - before).length() < 1e-4,
            "{before:?} became {after:?}"
        );

        // And from now on it follows the hand.
        scene.set_local(hand, at(13.0, 1.0, 0.0));
        let carried = scene.world_position(axe).unwrap();
        assert!(
            (carried - (before + vec3(10.0, 0.0, 0.0))).length() < 1e-4,
            "{carried:?}"
        );
    }

    #[test]
    fn detaching_leaves_a_node_where_it_was_in_its_own_terms() {
        let mut scene = Scene::new();
        let parent = scene.spawn(at(10.0, 0.0, 0.0));
        let child = scene.spawn_child(parent, at(1.0, 0.0, 0.0));

        scene.detach(child);
        assert_eq!(scene.parent(child), None);
        assert_eq!(scene.world_position(child), Some(vec3(1.0, 0.0, 0.0)));
        assert!(scene.roots().contains(&child));
    }

    #[test]
    fn removing_a_node_takes_its_subtree_with_it() {
        let mut scene = Scene::new();
        let cart = scene.spawn(Transform::IDENTITY);
        let axle = scene.spawn_child(cart, Transform::IDENTITY);
        let wheels: Vec<Node> = (0..4)
            .map(|_| scene.spawn_child(axle, Transform::IDENTITY))
            .collect();
        let unrelated = scene.spawn(Transform::IDENTITY);

        assert_eq!(scene.remove(cart), 6);
        assert!(!scene.contains(cart) && !scene.contains(axle));
        for wheel in wheels {
            assert!(!scene.contains(wheel), "a wheel outlived its cart");
        }
        assert!(scene.contains(unrelated));
        assert_eq!(scene.len(), 1);
    }

    #[test]
    fn a_stale_handle_cannot_move_whatever_reused_its_slot() {
        let mut scene = Scene::new();
        let old = scene.spawn(at(1.0, 0.0, 0.0));
        scene.remove(old);
        let new = scene.spawn(at(5.0, 0.0, 0.0));

        assert_eq!(new.index(), old.index(), "the slot is reused");
        assert_ne!(new.generation(), old.generation());
        assert!(!scene.contains(old));
        scene.set_local(old, at(99.0, 0.0, 0.0));
        assert_eq!(scene.world_position(new), Some(vec3(5.0, 0.0, 0.0)));
        assert_eq!(scene.world(old), None);
    }

    #[test]
    fn a_cycle_is_refused() {
        // A tree with a loop is a tree no traversal ends on.
        let mut scene = Scene::new();
        let a = scene.spawn(Transform::IDENTITY);
        let b = scene.spawn_child(a, Transform::IDENTITY);
        let c = scene.spawn_child(b, Transform::IDENTITY);

        assert!(!scene.attach(a, c), "a is above c");
        assert!(!scene.attach(a, a), "and cannot parent itself");
        assert_eq!(scene.parent(a), None);
        assert!(scene.is_ancestor(a, c));
        assert!(!scene.is_ancestor(c, a));
    }

    #[test]
    fn the_walk_visits_parents_before_children_in_a_fixed_order() {
        let mut scene = Scene::new();
        let a = scene.spawn(Transform::IDENTITY);
        let b = scene.spawn_child(a, Transform::IDENTITY);
        let c = scene.spawn_child(a, Transform::IDENTITY);
        let d = scene.spawn_child(b, Transform::IDENTITY);
        let e = scene.spawn(Transform::IDENTITY);

        let order = scene.iter();
        assert_eq!(order, vec![a, b, d, c, e]);
        for _ in 0..3 {
            assert_eq!(scene.iter(), order);
        }
    }

    #[test]
    fn a_deep_chain_does_not_overflow_the_stack() {
        // Ten thousand deep is absurd for a game and exactly the sort of thing
        // a recursive implementation falls over on.
        let mut scene = Scene::new();
        let mut current = scene.spawn(at(1.0, 0.0, 0.0));
        for _ in 0..10_000 {
            current = scene.spawn_child(current, at(1.0, 0.0, 0.0));
        }
        scene.update();
        let position = scene.world_position(current).unwrap();
        assert!((position.x - 10_001.0).abs() < 2.0, "{position:?}");
        assert_eq!(scene.iter().len(), 10_001);
        assert_eq!(scene.remove(scene.roots()[0]), 10_001);
    }

    #[test]
    fn a_node_can_point_at_an_entity() {
        let mut world = crate::world::World::new();
        let entity = world.spawn();
        let mut scene = Scene::new();
        let node = scene.spawn(Transform::IDENTITY);

        assert_eq!(scene.entity(node), None);
        scene.set_entity(node, entity);
        assert_eq!(scene.entity(node), Some(entity));
        scene.remove(node);
        assert_eq!(scene.entity(node), None);
    }

    #[test]
    fn a_matrix_can_be_taken_apart_again() {
        for transform in [
            Transform::from_position(vec3(3.0, -2.0, 7.0)),
            Transform::from_position(vec3(1.0, 1.0, 1.0))
                .with_rotation(Quat::from_euler(0.7, -0.3, 0.2))
                .with_scale(vec3(2.0, 2.0, 2.0)),
            Transform::IDENTITY.with_scale(vec3(1.5, 0.5, 3.0)),
        ] {
            let taken = Transform::from_matrix(transform.matrix());
            let before = transform.matrix();
            let after = taken.matrix();
            for column in 0..4 {
                for row in 0..4 {
                    assert!(
                        (before.cols[column][row] - after.cols[column][row]).abs() < 1e-4,
                        "{transform:?} came back as {taken:?}"
                    );
                }
            }
        }
    }
}
