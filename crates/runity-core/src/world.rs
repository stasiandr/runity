//! A small generational-index entity store.
//!
//! Components live in per-type dense arrays indexed by entity slot. It is not a
//! full archetype ECS — it is the least machinery that makes "spawn things,
//! attach data, iterate over it" work without `unsafe` or dependencies.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// A handle to an entity. Reusing a slot bumps its generation, so a stale
/// handle never resolves to the entity that replaced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity {
    index: u32,
    generation: u32,
}

impl Entity {
    #[inline]
    pub fn index(self) -> u32 {
        self.index
    }
    #[inline]
    pub fn generation(self) -> u32 {
        self.generation
    }
}

trait Storage: Any {
    fn clear_slot(&mut self, index: usize);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct TypedStorage<T> {
    slots: Vec<Option<T>>,
}

impl<T: 'static> Storage for TypedStorage<T> {
    fn clear_slot(&mut self, index: usize) {
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = None;
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Default)]
pub struct World {
    generations: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,
    storages: HashMap<TypeId, Box<dyn Storage>>,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&mut self) -> Entity {
        match self.free.pop() {
            Some(index) => {
                let slot = index as usize;
                self.alive[slot] = true;
                Entity {
                    index,
                    generation: self.generations[slot],
                }
            }
            None => {
                let index = self.generations.len() as u32;
                self.generations.push(0);
                self.alive.push(true);
                Entity {
                    index,
                    generation: 0,
                }
            }
        }
    }

    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let slot = entity.index as usize;
        self.alive[slot] = false;
        // Bumping the generation is what invalidates outstanding handles.
        self.generations[slot] = self.generations[slot].wrapping_add(1);
        for storage in self.storages.values_mut() {
            storage.clear_slot(slot);
        }
        self.free.push(entity.index);
        true
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        let slot = entity.index as usize;
        self.alive.get(slot).copied().unwrap_or(false)
            && self.generations[slot] == entity.generation
    }

    pub fn entity_count(&self) -> usize {
        self.alive.iter().filter(|a| **a).count()
    }

    fn storage<T: 'static>(&self) -> Option<&TypedStorage<T>> {
        self.storages
            .get(&TypeId::of::<T>())
            .and_then(|s| s.as_any().downcast_ref::<TypedStorage<T>>())
    }

    fn storage_mut<T: 'static>(&mut self) -> &mut TypedStorage<T> {
        let entry = self
            .storages
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(TypedStorage::<T> { slots: Vec::new() }));
        entry
            .as_any_mut()
            .downcast_mut::<TypedStorage<T>>()
            .expect("storage is keyed by its own TypeId")
    }

    /// Attach a component, replacing any previous value. Returns `false` if the
    /// handle is stale.
    pub fn insert<T: 'static>(&mut self, entity: Entity, component: T) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let slot = entity.index as usize;
        let storage = self.storage_mut::<T>();
        if storage.slots.len() <= slot {
            storage.slots.resize_with(slot + 1, || None);
        }
        storage.slots[slot] = Some(component);
        true
    }

    pub fn get<T: 'static>(&self, entity: Entity) -> Option<&T> {
        if !self.is_alive(entity) {
            return None;
        }
        self.storage::<T>()?
            .slots
            .get(entity.index as usize)?
            .as_ref()
    }

    pub fn get_mut<T: 'static>(&mut self, entity: Entity) -> Option<&mut T> {
        if !self.is_alive(entity) {
            return None;
        }
        let slot = entity.index as usize;
        self.storage_mut::<T>().slots.get_mut(slot)?.as_mut()
    }

    pub fn remove<T: 'static>(&mut self, entity: Entity) -> Option<T> {
        if !self.is_alive(entity) {
            return None;
        }
        let slot = entity.index as usize;
        self.storage_mut::<T>().slots.get_mut(slot)?.take()
    }

    pub fn has<T: 'static>(&self, entity: Entity) -> bool {
        self.get::<T>(entity).is_some()
    }

    /// Every live entity carrying a `T`, with a shared borrow of it.
    pub fn iter<T: 'static>(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.storage::<T>()
            .into_iter()
            .flat_map(|storage| storage.slots.iter().enumerate())
            .filter_map(move |(slot, component)| {
                let component = component.as_ref()?;
                let entity = Entity {
                    index: slot as u32,
                    generation: self.generations[slot],
                };
                self.alive[slot].then_some((entity, component))
            })
    }

    /// Same, with exclusive borrows.
    pub fn iter_mut<T: 'static>(&mut self) -> impl Iterator<Item = (Entity, &mut T)> + '_ {
        let generations = &self.generations;
        let alive = &self.alive;
        self.storages
            .get_mut(&TypeId::of::<T>())
            .and_then(|s| s.as_any_mut().downcast_mut::<TypedStorage<T>>())
            .into_iter()
            .flat_map(|storage| storage.slots.iter_mut().enumerate())
            .filter_map(move |(slot, component)| {
                let component = component.as_mut()?;
                let entity = Entity {
                    index: slot as u32,
                    generation: generations[slot],
                };
                alive[slot].then_some((entity, component))
            })
    }

    /// Handles of everything carrying a `T` — useful when a system needs to
    /// touch several component types at once.
    pub fn entities_with<T: 'static>(&self) -> Vec<Entity> {
        self.iter::<T>().map(|(e, _)| e).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Position(f32, f32);
    #[derive(Debug, PartialEq)]
    struct Health(i32);

    #[test]
    fn components_round_trip() {
        let mut world = World::new();
        let e = world.spawn();
        assert!(world.insert(e, Position(1.0, 2.0)));
        assert_eq!(world.get::<Position>(e), Some(&Position(1.0, 2.0)));
        world.get_mut::<Position>(e).unwrap().0 = 5.0;
        assert_eq!(world.get::<Position>(e).unwrap().0, 5.0);
        assert_eq!(world.remove::<Position>(e), Some(Position(5.0, 2.0)));
        assert!(!world.has::<Position>(e));
    }

    #[test]
    fn a_stale_handle_cannot_reach_the_entity_that_replaced_it() {
        let mut world = World::new();
        let old = world.spawn();
        world.insert(old, Health(10));
        assert!(world.despawn(old));
        assert!(!world.is_alive(old));

        let new = world.spawn();
        assert_eq!(new.index(), old.index(), "the slot is reused");
        assert_ne!(
            new.generation(),
            old.generation(),
            "but the generation moved on"
        );

        world.insert(new, Health(99));
        assert_eq!(
            world.get::<Health>(old),
            None,
            "the stale handle sees nothing"
        );
        assert_eq!(world.get::<Health>(new), Some(&Health(99)));
        assert!(!world.despawn(old), "despawning twice is a no-op");
    }

    #[test]
    fn despawn_drops_every_component() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Position(0.0, 0.0));
        world.insert(e, Health(3));
        world.despawn(e);
        assert_eq!(world.iter::<Position>().count(), 0);
        assert_eq!(world.iter::<Health>().count(), 0);
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn iteration_visits_only_live_entities_with_the_component() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.insert(a, Position(1.0, 0.0));
        world.insert(c, Position(3.0, 0.0));
        world.insert(b, Health(1));

        let mut found: Vec<f32> = world.iter::<Position>().map(|(_, p)| p.0).collect();
        found.sort_by(f32::total_cmp);
        assert_eq!(found, vec![1.0, 3.0]);

        for (_, p) in world.iter_mut::<Position>() {
            p.0 *= 2.0;
        }
        assert_eq!(world.get::<Position>(a).unwrap().0, 2.0);
        assert_eq!(world.entities_with::<Position>(), vec![a, c]);
    }

    #[test]
    fn inserting_on_a_dead_entity_fails_instead_of_corrupting_a_slot() {
        let mut world = World::new();
        let e = world.spawn();
        world.despawn(e);
        assert!(!world.insert(e, Health(1)));
        let reused = world.spawn();
        assert_eq!(world.get::<Health>(reused), None);
    }

    #[test]
    fn iterating_a_component_no_one_has_is_empty() {
        let world = World::new();
        assert_eq!(world.iter::<Position>().count(), 0);
    }
}
