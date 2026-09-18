//! A generational-index entity store with change detection and events.
//!
//! Components live in per-type dense arrays indexed by entity slot. It is not
//! an archetype ECS — no table moves, no query planner — but it carries the
//! three things a simulation cannot do without:
//!
//! * **Queries over several components.** A system that moves villagers needs
//!   their position *and* their destination; looking the second one up by
//!   hand for every entity is how iteration order bugs get in.
//! * **Change detection.** The world advances ten times a second and almost
//!   nothing changes in any given tick. Knowing exactly what moved is what
//!   lets a network layer send a delta instead of a world, and what lets a
//!   renderer rebuild only the chunk that was edited.
//! * **Events.** Systems have to tell each other that a tree fell or a
//!   building finished without holding references to one another. Events are
//!   double-buffered, so every event is readable for exactly one full tick no
//!   matter which system runs first — order independence being the property
//!   that keeps a simulation deterministic.
//!
//! Everything here is `unsafe`-free. That costs one real ergonomic
//! compromise: iterating two component types where one is mutable is a
//! closure ([`World::each2_mut`]) rather than an iterator, because handing
//! out `&mut A` and `&B` from the same map at once needs either `unsafe` or a
//! nightly API. The read-only queries are ordinary iterators.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use runity_serialize::{serializable, Deserialize, Reader, Result, Serialize, Writer};

/// A handle to an entity. Reusing a slot bumps its generation, so a stale
/// handle never resolves to the entity that replaced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity {
    index: u32,
    generation: u32,
}

impl Entity {
    /// Which slot the entity occupies. Dense and small, so it doubles as a
    /// network id.
    #[inline]
    pub fn index(self) -> u32 {
        self.index
    }

    /// How many times this slot has been reused.
    #[inline]
    pub fn generation(self) -> u32 {
        self.generation
    }
}

serializable!(Entity { index, generation });

/// Sent automatically whenever an entity is despawned.
///
/// Change detection can report a component that changed, but not one that
/// stopped existing — so replication and any cache keyed by entity need this
/// event to stay honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Despawned(pub Entity);

serializable!(Despawned(0));

/// An exclusive borrow of a component that notices being written to.
///
/// Reading through it is free; the first `DerefMut` stamps the component with
/// the world's current change tick. That is why mutable access is wrapped:
/// without it, "what changed this tick" would have to be maintained by hand
/// at every call site, and it would be wrong within a week.
pub struct Mut<'a, T> {
    value: &'a mut T,
    changed: &'a mut u64,
    tick: u64,
}

impl<T> Mut<'_, T> {
    /// Write without recording a change.
    ///
    /// For bookkeeping that no observer cares about — a cached value
    /// recomputed from data that did not itself change. Use it sparingly:
    /// a missed change shows up as a desync, not as a compile error.
    pub fn bypass_change_detection(&mut self) -> &mut T {
        self.value
    }

    /// The tick at which this component last changed.
    pub fn last_changed(&self) -> u64 {
        *self.changed
    }
}

impl<T> Deref for Mut<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> DerefMut for Mut<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        *self.changed = self.tick;
        self.value
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for Mut<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.value.fmt(f)
    }
}

/// A component together with when it arrived and when it last changed.
struct Slot<T> {
    value: T,
    added: u64,
    changed: u64,
}

trait Storage: Any {
    fn clear_slot(&mut self, index: usize);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct TypedStorage<T> {
    slots: Vec<Option<Slot<T>>>,
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

trait AnyQueue: Any {
    /// Retire the readable buffer and promote what was sent since.
    fn rotate(&mut self);
    fn clear(&mut self);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct Queue<T> {
    readable: Vec<T>,
    incoming: Vec<T>,
}

impl<T: 'static> AnyQueue for Queue<T> {
    fn rotate(&mut self) {
        self.readable.clear();
        core::mem::swap(&mut self.readable, &mut self.incoming);
    }

    fn clear(&mut self) {
        self.readable.clear();
        self.incoming.clear();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Entities, their components, and the events systems send each other.
#[derive(Default)]
pub struct World {
    generations: Vec<u32>,
    alive: Vec<bool>,
    free: Vec<u32>,
    storages: HashMap<TypeId, Box<dyn Storage>>,
    events: HashMap<TypeId, Box<dyn AnyQueue>>,
    change_tick: u64,
}

impl World {
    /// An empty world at change tick 1.
    ///
    /// Ticks start at one so that "anything changed since 0" means
    /// "everything", which is what a freshly connected client wants.
    pub fn new() -> Self {
        Self {
            change_tick: 1,
            ..Self::default()
        }
    }

    // ------------------------------------------------------------- entities

    /// Create an entity, reusing a free slot when there is one.
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

    /// Destroy an entity and everything attached to it. Returns `false` for a
    /// handle that was already stale.
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
        self.send(Despawned(entity));
        true
    }

    /// Whether the handle still refers to a living entity.
    pub fn is_alive(&self, entity: Entity) -> bool {
        let slot = entity.index as usize;
        self.alive.get(slot).copied().unwrap_or(false)
            && self.generations[slot] == entity.generation
    }

    /// How many entities are alive.
    pub fn entity_count(&self) -> usize {
        self.alive.iter().filter(|a| **a).count()
    }

    /// Every living entity, in slot order.
    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.alive
            .iter()
            .enumerate()
            .filter(|(_, alive)| **alive)
            .map(|(slot, _)| Entity {
                index: slot as u32,
                generation: self.generations[slot],
            })
    }

    // ----------------------------------------------------------- change tick

    /// The world's current change tick.
    pub fn change_tick(&self) -> u64 {
        self.change_tick
    }

    /// Start a new tick: stamp later changes with a new number and rotate the
    /// event buffers.
    ///
    /// Call once per world tick, before the systems run.
    pub fn advance_tick(&mut self) -> u64 {
        self.change_tick += 1;
        self.update_events();
        self.change_tick
    }

    // ----------------------------------------------------------- components

    fn storage<T: 'static>(&self) -> Option<&TypedStorage<T>> {
        self.storages
            .get(&TypeId::of::<T>())
            .and_then(|s| s.as_any().downcast_ref())
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

    /// Attach a component, replacing any previous value. Returns `false` if
    /// the handle is stale.
    pub fn insert<T: 'static>(&mut self, entity: Entity, component: T) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let slot_index = entity.index as usize;
        let tick = self.change_tick;
        let storage = self.storage_mut::<T>();
        if storage.slots.len() <= slot_index {
            storage.slots.resize_with(slot_index + 1, || None);
        }
        // Replacing a component keeps its original `added` tick: the entity
        // did not newly gain the component, its value changed.
        let added = match &storage.slots[slot_index] {
            Some(existing) => existing.added,
            None => tick,
        };
        storage.slots[slot_index] = Some(Slot {
            value: component,
            added,
            changed: tick,
        });
        true
    }

    /// Borrow a component.
    pub fn get<T: 'static>(&self, entity: Entity) -> Option<&T> {
        if !self.is_alive(entity) {
            return None;
        }
        Some(
            &self
                .storage::<T>()?
                .slots
                .get(entity.index as usize)?
                .as_ref()?
                .value,
        )
    }

    /// Borrow a component exclusively; writing through the result records a
    /// change at the current tick.
    pub fn get_mut<T: 'static>(&mut self, entity: Entity) -> Option<Mut<'_, T>> {
        if !self.is_alive(entity) {
            return None;
        }
        let slot_index = entity.index as usize;
        let tick = self.change_tick;
        let slot = self
            .storage_mut::<T>()
            .slots
            .get_mut(slot_index)?
            .as_mut()?;
        Some(Mut {
            value: &mut slot.value,
            changed: &mut slot.changed,
            tick,
        })
    }

    /// Detach and return a component.
    pub fn remove<T: 'static>(&mut self, entity: Entity) -> Option<T> {
        if !self.is_alive(entity) {
            return None;
        }
        let slot_index = entity.index as usize;
        let slot = self.storage_mut::<T>().slots.get_mut(slot_index)?.take()?;
        Some(slot.value)
    }

    /// Whether the entity carries a `T`.
    pub fn has<T: 'static>(&self, entity: Entity) -> bool {
        self.get::<T>(entity).is_some()
    }

    /// When the entity's `T` last changed, or `None` if it has none.
    pub fn last_changed<T: 'static>(&self, entity: Entity) -> Option<u64> {
        if !self.is_alive(entity) {
            return None;
        }
        Some(
            self.storage::<T>()?
                .slots
                .get(entity.index as usize)?
                .as_ref()?
                .changed,
        )
    }

    /// Whether the entity's `T` changed after `tick`.
    pub fn is_changed<T: 'static>(&self, entity: Entity, since: u64) -> bool {
        self.last_changed::<T>(entity)
            .is_some_and(|changed| changed > since)
    }

    /// Whether the entity gained its `T` after `tick`.
    pub fn is_added<T: 'static>(&self, entity: Entity, since: u64) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        self.storage::<T>()
            .and_then(|storage| storage.slots.get(entity.index as usize)?.as_ref())
            .is_some_and(|slot| slot.added > since)
    }

    // -------------------------------------------------------------- queries

    /// Every live entity carrying a `T`.
    pub fn iter<T: 'static>(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.slots::<T>()
            .map(|(entity, slot)| (entity, &slot.value))
    }

    /// Same, with exclusive borrows that record changes.
    pub fn iter_mut<T: 'static>(&mut self) -> impl Iterator<Item = (Entity, Mut<'_, T>)> + '_ {
        let generations = &self.generations;
        let alive = &self.alive;
        let tick = self.change_tick;
        self.storages
            .get_mut(&TypeId::of::<T>())
            .and_then(|s| s.as_any_mut().downcast_mut::<TypedStorage<T>>())
            .into_iter()
            .flat_map(|storage| storage.slots.iter_mut().enumerate())
            .filter_map(move |(index, slot)| {
                let slot = slot.as_mut()?;
                if !alive.get(index).copied().unwrap_or(false) {
                    return None;
                }
                let entity = Entity {
                    index: index as u32,
                    generation: generations[index],
                };
                Some((
                    entity,
                    Mut {
                        value: &mut slot.value,
                        changed: &mut slot.changed,
                        tick,
                    },
                ))
            })
    }

    /// Entities carrying both components.
    pub fn query2<A: 'static, B: 'static>(&self) -> impl Iterator<Item = (Entity, &A, &B)> + '_ {
        let second = self.storage::<B>();
        self.iter::<A>().filter_map(move |(entity, a)| {
            let b = second?.slots.get(entity.index as usize)?.as_ref()?;
            Some((entity, a, &b.value))
        })
    }

    /// Entities carrying all three components.
    pub fn query3<A: 'static, B: 'static, C: 'static>(
        &self,
    ) -> impl Iterator<Item = (Entity, &A, &B, &C)> + '_ {
        let third = self.storage::<C>();
        self.query2::<A, B>().filter_map(move |(entity, a, b)| {
            let c = third?.slots.get(entity.index as usize)?.as_ref()?;
            Some((entity, a, b, &c.value))
        })
    }

    /// Entities carrying an `A` but no `B` — the "without" half of a query.
    ///
    /// Reads as a rule of the simulation: villagers without a home, buildings
    /// without a worker, resources nobody has claimed.
    pub fn query_without<A: 'static, B: 'static>(&self) -> impl Iterator<Item = (Entity, &A)> + '_ {
        let second = self.storage::<B>();
        self.iter::<A>().filter(move |(entity, _)| {
            !second.is_some_and(|storage| {
                storage
                    .slots
                    .get(entity.index as usize)
                    .is_some_and(Option::is_some)
            })
        })
    }

    /// Everything whose `T` changed after `since`.
    ///
    /// This is the network layer's whole job in one call: hand it the tick of
    /// the last acknowledged snapshot and it yields precisely what the other
    /// side does not know yet.
    pub fn changed_since<T: 'static>(&self, since: u64) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.slots::<T>()
            .filter(move |(_, slot)| slot.changed > since)
            .map(|(entity, slot)| (entity, &slot.value))
    }

    /// Everything that gained a `T` after `since`.
    pub fn added_since<T: 'static>(&self, since: u64) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.slots::<T>()
            .filter(move |(_, slot)| slot.added > since)
            .map(|(entity, slot)| (entity, &slot.value))
    }

    /// Visit every entity carrying both components, with the first mutable.
    ///
    /// A closure rather than an iterator because borrowing one storage
    /// mutably and another immutably at the same time is not expressible
    /// safely; the storage is moved out of the map for the duration and put
    /// back afterwards, even if the closure panics.
    pub fn each2_mut<A: 'static, B: 'static>(
        &mut self,
        mut body: impl FnMut(Entity, Mut<'_, A>, &B),
    ) {
        let Some(taken) = self.storages.remove(&TypeId::of::<A>()) else {
            return;
        };
        let mut guard = Lease {
            world: self,
            type_id: TypeId::of::<A>(),
            storage: Some(taken),
        };
        guard.each::<A, _>(|entity, item, world| {
            let Some(b) = world.get::<B>(entity) else {
                return;
            };
            body(entity, item, b);
        });
    }

    /// Visit every entity carrying all three, with the first mutable.
    pub fn each3_mut<A: 'static, B: 'static, C: 'static>(
        &mut self,
        mut body: impl FnMut(Entity, Mut<'_, A>, &B, &C),
    ) {
        let Some(taken) = self.storages.remove(&TypeId::of::<A>()) else {
            return;
        };
        let mut guard = Lease {
            world: self,
            type_id: TypeId::of::<A>(),
            storage: Some(taken),
        };
        guard.each::<A, _>(|entity, item, world| {
            let (Some(b), Some(c)) = (world.get::<B>(entity), world.get::<C>(entity)) else {
                return;
            };
            body(entity, item, b, c);
        });
    }

    /// Handles of everything carrying a `T`.
    pub fn entities_with<T: 'static>(&self) -> Vec<Entity> {
        self.iter::<T>().map(|(e, _)| e).collect()
    }

    /// How many entities carry a `T`.
    pub fn count<T: 'static>(&self) -> usize {
        self.iter::<T>().count()
    }

    fn slots<T: 'static>(&self) -> impl Iterator<Item = (Entity, &Slot<T>)> + '_ {
        self.storage::<T>()
            .into_iter()
            .flat_map(|storage| storage.slots.iter().enumerate())
            .filter_map(move |(index, slot)| {
                let slot = slot.as_ref()?;
                if !self.alive.get(index).copied().unwrap_or(false) {
                    return None;
                }
                let entity = Entity {
                    index: index as u32,
                    generation: self.generations[index],
                };
                Some((entity, slot))
            })
    }

    // --------------------------------------------------------------- events

    /// Queue an event. It becomes readable on the next [`World::advance_tick`]
    /// and stays readable for exactly that one tick.
    ///
    /// The delay is deliberate: a system that reacts to an event must see the
    /// same events regardless of whether it happens to run before or after
    /// the system that sent them.
    pub fn send<T: 'static>(&mut self, event: T) {
        let queue = self
            .events
            .entry(TypeId::of::<T>())
            .or_insert_with(|| {
                Box::new(Queue::<T> {
                    readable: Vec::new(),
                    incoming: Vec::new(),
                })
            })
            .as_any_mut()
            .downcast_mut::<Queue<T>>()
            .expect("event queue is keyed by its own TypeId");
        queue.incoming.push(event);
    }

    /// The events of this type readable during the current tick.
    pub fn events<T: 'static>(&self) -> &[T] {
        self.events
            .get(&TypeId::of::<T>())
            .and_then(|q| q.as_any().downcast_ref::<Queue<T>>())
            .map(|q| q.readable.as_slice())
            .unwrap_or(&[])
    }

    /// Whether any event of this type is readable.
    pub fn has_events<T: 'static>(&self) -> bool {
        !self.events::<T>().is_empty()
    }

    /// Take the readable events, leaving the queue empty.
    ///
    /// For an event exactly one system consumes — a command queue, say —
    /// where letting a second reader see it would be a bug.
    pub fn drain_events<T: 'static>(&mut self) -> Vec<T> {
        self.events
            .get_mut(&TypeId::of::<T>())
            .and_then(|q| q.as_any_mut().downcast_mut::<Queue<T>>())
            .map(|q| core::mem::take(&mut q.readable))
            .unwrap_or_default()
    }

    /// Retire this tick's events and promote what was sent during it.
    pub fn update_events(&mut self) {
        for queue in self.events.values_mut() {
            queue.rotate();
        }
    }

    /// Drop every event, sent or readable.
    pub fn clear_events(&mut self) {
        for queue in self.events.values_mut() {
            queue.clear();
        }
    }

    // -------------------------------------------------------- serialization

    /// Write the entity table: which slots are alive and at what generation.
    ///
    /// Components are saved separately, one call per type, because a world
    /// that stores `Box<dyn Any>` cannot know how to encode what it holds.
    /// Listing the types is a few lines in the game and avoids a reflection
    /// system nobody asked for.
    pub fn save_entities(&self, writer: &mut Writer) {
        writer
            .write(&self.change_tick)
            .write(&self.generations)
            .write(&self.alive);
    }

    /// Restore an entity table written by [`World::save_entities`].
    pub fn load_entities(&mut self, reader: &mut Reader) -> Result<()> {
        self.change_tick = reader.read()?;
        self.generations = reader.read()?;
        self.alive = reader.read()?;
        self.alive.resize(self.generations.len(), false);
        self.storages.clear();
        self.clear_events();
        self.free = (0..self.generations.len() as u32)
            .filter(|i| !self.alive[*i as usize])
            .collect();
        // Pop order decides which slot the next spawn takes; reversing makes
        // it the lowest free slot, which keeps saves tidy and reproducible.
        self.free.reverse();
        Ok(())
    }

    /// Write every `T` in the world, with the entity each belongs to.
    pub fn save_components<T: Serialize + 'static>(&self, writer: &mut Writer) {
        let components: Vec<(Entity, &T)> = self.iter::<T>().collect();
        writer.seq_of(components.len(), components);
    }

    /// Read components written by [`World::save_components`], skipping any
    /// whose entity is not alive in this world.
    pub fn load_components<T: Deserialize + 'static>(
        &mut self,
        reader: &mut Reader,
    ) -> Result<usize> {
        let components: Vec<(Entity, T)> = reader.seq()?;
        let mut loaded = 0;
        for (entity, component) in components {
            if self.insert(entity, component) {
                loaded += 1;
            }
        }
        Ok(loaded)
    }
}

/// Holds a component storage outside the world while it is being iterated
/// mutably, and puts it back on drop — including when the body panics, which
/// is the only reason this is a type rather than two lines inline.
struct Lease<'w> {
    world: &'w mut World,
    type_id: TypeId,
    storage: Option<Box<dyn Storage>>,
}

impl Lease<'_> {
    fn each<T: 'static, F>(&mut self, mut body: F)
    where
        F: FnMut(Entity, Mut<'_, T>, &World),
    {
        // Two disjoint fields: the leased storage is mutable, the rest of the
        // world stays readable. That is exactly the split the borrow checker
        // cannot see through a `HashMap`.
        let world = &*self.world;
        let Some(storage) = self
            .storage
            .as_mut()
            .and_then(|s| s.as_any_mut().downcast_mut::<TypedStorage<T>>())
        else {
            return;
        };
        let tick = world.change_tick;
        for (index, slot) in storage.slots.iter_mut().enumerate() {
            let Some(slot) = slot.as_mut() else {
                continue;
            };
            if !world.alive.get(index).copied().unwrap_or(false) {
                continue;
            }
            let entity = Entity {
                index: index as u32,
                generation: world.generations[index],
            };
            let item = Mut {
                value: &mut slot.value,
                changed: &mut slot.changed,
                tick,
            };
            body(entity, item, world);
        }
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(storage) = self.storage.take() {
            self.world.storages.insert(self.type_id, storage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_serialize::Archive;

    #[derive(Debug, PartialEq, Clone, Copy)]
    struct Position(f32, f32);
    #[derive(Debug, PartialEq, Clone, Copy)]
    struct Health(i32);
    #[derive(Debug, PartialEq, Clone, Copy)]
    struct Villager;
    #[derive(Debug, PartialEq, Clone, Copy)]
    struct Home(Entity);

    serializable!(Position(0, 1));
    serializable!(Health(0));

    #[derive(Debug, PartialEq, Clone, Copy)]
    struct TreeFelled {
        by: Entity,
        wood: u32,
    }

    #[derive(Debug, PartialEq)]
    struct BuildingFinished(&'static str);

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
    fn despawn_drops_every_component_and_announces_itself() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Position(0.0, 0.0));
        world.insert(e, Health(3));
        world.despawn(e);
        assert_eq!(world.iter::<Position>().count(), 0);
        assert_eq!(world.iter::<Health>().count(), 0);
        assert_eq!(world.entity_count(), 0);

        // Change detection cannot report a component that no longer exists,
        // so the despawn itself is an event.
        world.advance_tick();
        assert_eq!(world.events::<Despawned>(), &[Despawned(e)]);
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

        for (_, mut p) in world.iter_mut::<Position>() {
            p.0 *= 2.0;
        }
        assert_eq!(world.get::<Position>(a).unwrap().0, 2.0);
        assert_eq!(world.entities_with::<Position>(), vec![a, c]);
        assert_eq!(world.count::<Position>(), 2);
        assert_eq!(world.entities().count(), 3);
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
        assert_eq!(world.query2::<Position, Health>().count(), 0);
        assert_eq!(world.changed_since::<Position>(0).count(), 0);
    }

    // ------------------------------------------------------------- queries

    #[test]
    fn a_query_yields_only_entities_carrying_every_component() {
        let mut world = World::new();
        let both = world.spawn();
        let only_position = world.spawn();
        let only_health = world.spawn();
        let all_three = world.spawn();

        world.insert(both, Position(1.0, 1.0));
        world.insert(both, Health(10));
        world.insert(only_position, Position(2.0, 2.0));
        world.insert(only_health, Health(20));
        world.insert(all_three, Position(3.0, 3.0));
        world.insert(all_three, Health(30));
        world.insert(all_three, Villager);

        let pairs: Vec<Entity> = world
            .query2::<Position, Health>()
            .map(|(e, ..)| e)
            .collect();
        assert_eq!(pairs, vec![both, all_three]);

        let triples: Vec<i32> = world
            .query3::<Position, Health, Villager>()
            .map(|(_, _, h, _)| h.0)
            .collect();
        assert_eq!(triples, vec![30]);

        // The same query the other way round must agree about membership.
        let reversed: Vec<Entity> = world
            .query2::<Health, Position>()
            .map(|(e, ..)| e)
            .collect();
        assert_eq!(reversed, pairs);
    }

    #[test]
    fn a_query_without_finds_what_is_missing() {
        // "Villagers with no home" is the sort of rule a settlement sim is
        // made of, so it should not need a manual filter at every call site.
        let mut world = World::new();
        let housed = world.spawn();
        let homeless = world.spawn();
        let house = world.spawn();
        world.insert(housed, Villager);
        world.insert(homeless, Villager);
        world.insert(housed, Home(house));

        let without: Vec<Entity> = world
            .query_without::<Villager, Home>()
            .map(|(e, _)| e)
            .collect();
        assert_eq!(without, vec![homeless]);

        // Once housed, they drop out of the query.
        world.insert(homeless, Home(house));
        assert_eq!(world.query_without::<Villager, Home>().count(), 0);
    }

    #[test]
    fn a_query_skips_despawned_entities() {
        let mut world = World::new();
        let keep = world.spawn();
        let drop = world.spawn();
        for e in [keep, drop] {
            world.insert(e, Position(0.0, 0.0));
            world.insert(e, Health(1));
        }
        world.despawn(drop);
        assert_eq!(world.query2::<Position, Health>().count(), 1);
        assert_eq!(world.query2::<Position, Health>().next().unwrap().0, keep);
    }

    #[test]
    fn each2_mut_writes_the_first_and_reads_the_second() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        let lonely = world.spawn();
        world.insert(a, Position(0.0, 0.0));
        world.insert(a, Health(3));
        world.insert(b, Position(10.0, 0.0));
        world.insert(b, Health(1));
        world.insert(lonely, Position(99.0, 0.0));

        world.each2_mut::<Position, Health>(|_, mut position, health| {
            position.0 += health.0 as f32;
        });

        assert_eq!(world.get::<Position>(a), Some(&Position(3.0, 0.0)));
        assert_eq!(world.get::<Position>(b), Some(&Position(11.0, 0.0)));
        assert_eq!(
            world.get::<Position>(lonely),
            Some(&Position(99.0, 0.0)),
            "no Health, no visit"
        );
    }

    #[test]
    fn each3_mut_visits_only_the_full_set() {
        let mut world = World::new();
        let full = world.spawn();
        let partial = world.spawn();
        for e in [full, partial] {
            world.insert(e, Position(0.0, 0.0));
            world.insert(e, Health(2));
        }
        world.insert(full, Villager);

        let mut visited = Vec::new();
        world.each3_mut::<Position, Health, Villager>(|entity, mut position, health, _| {
            position.1 = health.0 as f32;
            visited.push(entity);
        });
        assert_eq!(visited, vec![full]);
        assert_eq!(world.get::<Position>(full).unwrap().1, 2.0);
        assert_eq!(world.get::<Position>(partial).unwrap().1, 0.0);
    }

    #[test]
    fn a_panicking_system_does_not_lose_the_storage() {
        // The leased storage lives outside the world while a system runs;
        // if that system panics, the components must still come back.
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Position(1.0, 2.0));
        world.insert(e, Health(5));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.each2_mut::<Position, Health>(|_, _, _| panic!("a system blew up"));
        }));
        assert!(result.is_err());
        assert_eq!(world.get::<Position>(e), Some(&Position(1.0, 2.0)));
        assert_eq!(world.iter::<Position>().count(), 1);
    }

    // ----------------------------------------------------- change detection

    #[test]
    fn writing_marks_a_change_and_reading_does_not() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Position(0.0, 0.0));

        let baseline = world.change_tick();
        world.advance_tick();

        // Reading, even through the mutable accessor, changes nothing.
        assert_eq!(world.get::<Position>(e), Some(&Position(0.0, 0.0)));
        {
            let borrowed = world.get_mut::<Position>(e).unwrap();
            let _read = borrowed.0;
        }
        assert!(
            !world.is_changed::<Position>(e, baseline),
            "a read is not a change"
        );

        world.get_mut::<Position>(e).unwrap().0 = 1.0;
        assert!(world.is_changed::<Position>(e, baseline));
        assert_eq!(world.last_changed::<Position>(e), Some(world.change_tick()));
    }

    #[test]
    fn change_detection_can_be_bypassed_deliberately() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Position(0.0, 0.0));
        let baseline = world.change_tick();
        world.advance_tick();

        world
            .get_mut::<Position>(e)
            .unwrap()
            .bypass_change_detection()
            .0 = 7.0;
        assert_eq!(world.get::<Position>(e).unwrap().0, 7.0);
        assert!(!world.is_changed::<Position>(e, baseline));
    }

    #[test]
    fn added_and_changed_are_different_questions() {
        let mut world = World::new();
        let e = world.spawn();
        let start = world.change_tick();
        world.insert(e, Health(10));
        assert!(world.is_added::<Health>(e, start - 1));
        assert!(world.is_changed::<Health>(e, start - 1));

        let after_insert = world.change_tick();
        world.advance_tick();
        // Overwriting is a change, not an arrival.
        world.insert(e, Health(20));
        assert!(world.is_changed::<Health>(e, after_insert));
        assert!(
            !world.is_added::<Health>(e, after_insert),
            "the component was already there"
        );
    }

    #[test]
    fn a_delta_contains_exactly_what_moved() {
        // The network layer's whole job: given the last acknowledged tick,
        // what does the other side not know yet?
        let mut world = World::new();
        let still = world.spawn();
        let moving = world.spawn();
        world.insert(still, Position(0.0, 0.0));
        world.insert(moving, Position(0.0, 0.0));

        let acknowledged = world.change_tick();
        world.advance_tick();
        world.get_mut::<Position>(moving).unwrap().0 = 4.0;

        let delta: Vec<Entity> = world
            .changed_since::<Position>(acknowledged)
            .map(|(e, _)| e)
            .collect();
        assert_eq!(delta, vec![moving]);

        // A newly spawned entity shows up in both queries.
        let arrival = world.spawn();
        world.insert(arrival, Position(9.0, 9.0));
        let added: Vec<Entity> = world
            .added_since::<Position>(acknowledged)
            .map(|(e, _)| e)
            .collect();
        assert_eq!(added, vec![arrival]);
        let delta: Vec<Entity> = world
            .changed_since::<Position>(acknowledged)
            .map(|(e, _)| e)
            .collect();
        assert_eq!(delta, vec![moving, arrival]);

        // And after acknowledging the current tick, the delta is empty again.
        let acknowledged = world.change_tick();
        world.advance_tick();
        assert_eq!(world.changed_since::<Position>(acknowledged).count(), 0);
    }

    #[test]
    fn iterating_mutably_marks_what_it_touches() {
        let mut world = World::new();
        let e = world.spawn();
        world.insert(e, Position(0.0, 0.0));
        let baseline = world.change_tick();
        world.advance_tick();

        for (_, mut p) in world.iter_mut::<Position>() {
            p.0 += 1.0;
        }
        assert!(world.is_changed::<Position>(e, baseline));

        let baseline = world.change_tick();
        world.advance_tick();
        world.each2_mut::<Position, Health>(|_, mut p, _| p.0 += 1.0);
        assert!(
            !world.is_changed::<Position>(e, baseline),
            "no Health means no visit"
        );
    }

    // ------------------------------------------------------------- events

    #[test]
    fn an_event_is_readable_for_exactly_one_tick() {
        let mut world = World::new();
        let sender = world.spawn();
        world.send(TreeFelled {
            by: sender,
            wood: 12,
        });

        assert!(
            world.events::<TreeFelled>().is_empty(),
            "not until the tick turns over"
        );

        world.advance_tick();
        assert_eq!(
            world.events::<TreeFelled>(),
            &[TreeFelled {
                by: sender,
                wood: 12
            }]
        );
        assert!(world.has_events::<TreeFelled>());

        world.advance_tick();
        assert!(
            world.events::<TreeFelled>().is_empty(),
            "and then it is gone"
        );
    }

    #[test]
    fn event_visibility_does_not_depend_on_system_order() {
        // The property that makes events safe in a deterministic simulation:
        // a reader sees the same events whether it runs before or after the
        // sender within the same tick.
        let mut world = World::new();
        let e = world.spawn();

        world.advance_tick();
        let before: Vec<u32> = world
            .events::<TreeFelled>()
            .iter()
            .map(|t| t.wood)
            .collect();
        world.send(TreeFelled { by: e, wood: 5 });
        let after: Vec<u32> = world
            .events::<TreeFelled>()
            .iter()
            .map(|t| t.wood)
            .collect();
        assert_eq!(before, after);

        world.advance_tick();
        assert_eq!(world.events::<TreeFelled>().len(), 1);
    }

    #[test]
    fn events_keep_their_order_and_their_types_apart() {
        let mut world = World::new();
        let e = world.spawn();
        for wood in [1u32, 2, 3] {
            world.send(TreeFelled { by: e, wood });
        }
        world.send(BuildingFinished("granary"));
        world.advance_tick();

        let wood: Vec<u32> = world
            .events::<TreeFelled>()
            .iter()
            .map(|t| t.wood)
            .collect();
        assert_eq!(wood, vec![1, 2, 3]);
        assert_eq!(world.events::<BuildingFinished>().len(), 1);
        assert_eq!(world.events::<BuildingFinished>()[0].0, "granary");
    }

    #[test]
    fn draining_consumes_events_so_a_second_reader_sees_none() {
        let mut world = World::new();
        let e = world.spawn();
        world.send(TreeFelled { by: e, wood: 7 });
        world.advance_tick();

        let taken = world.drain_events::<TreeFelled>();
        assert_eq!(taken.len(), 1);
        assert!(world.events::<TreeFelled>().is_empty());
        assert!(world.drain_events::<TreeFelled>().is_empty());
    }

    #[test]
    fn an_event_type_nobody_sent_reads_as_empty() {
        let world = World::new();
        assert!(world.events::<BuildingFinished>().is_empty());
        assert!(!world.has_events::<BuildingFinished>());
    }

    #[test]
    fn clearing_events_drops_the_pending_ones_too() {
        let mut world = World::new();
        let e = world.spawn();
        world.send(TreeFelled { by: e, wood: 1 });
        world.advance_tick();
        world.send(TreeFelled { by: e, wood: 2 });
        world.clear_events();
        assert!(world.events::<TreeFelled>().is_empty());
        world.advance_tick();
        assert!(world.events::<TreeFelled>().is_empty());
    }

    // ------------------------------------------------------ serialization

    #[test]
    fn a_world_survives_a_save_and_a_load() {
        let mut world = World::new();
        let alice = world.spawn();
        let bob = world.spawn();
        let doomed = world.spawn();
        world.insert(alice, Position(1.0, 2.0));
        world.insert(alice, Health(10));
        world.insert(bob, Position(3.0, 4.0));
        world.insert(doomed, Position(9.0, 9.0));
        world.despawn(doomed);
        world.advance_tick();
        world.get_mut::<Health>(alice).unwrap().0 = 11;

        let bytes = Archive::write(*b"SAVE", 1, |writer| {
            world.save_entities(writer);
            world.save_components::<Position>(writer);
            world.save_components::<Health>(writer);
        });

        let mut loaded = World::new();
        let (_, mut reader) = Archive::open(&bytes, *b"SAVE", 1).unwrap();
        loaded.load_entities(&mut reader).unwrap();
        assert_eq!(loaded.load_components::<Position>(&mut reader).unwrap(), 2);
        assert_eq!(loaded.load_components::<Health>(&mut reader).unwrap(), 1);
        reader.finish().unwrap();

        assert_eq!(loaded.entity_count(), 2);
        assert_eq!(loaded.get::<Position>(alice), Some(&Position(1.0, 2.0)));
        assert_eq!(loaded.get::<Health>(alice), Some(&Health(11)));
        assert_eq!(loaded.get::<Position>(bob), Some(&Position(3.0, 4.0)));
        assert!(
            !loaded.is_alive(doomed),
            "a despawned entity stays despawned"
        );
        assert_eq!(loaded.change_tick(), world.change_tick());

        // The freed slot is still free, and reusing it still invalidates the
        // old handle.
        let reborn = loaded.spawn();
        assert_eq!(reborn.index(), doomed.index());
        assert_ne!(reborn.generation(), doomed.generation());
    }

    #[test]
    fn loading_skips_components_whose_entity_is_gone() {
        let mut world = World::new();
        let kept = world.spawn();
        let removed = world.spawn();
        world.insert(kept, Health(1));
        world.insert(removed, Health(2));

        let mut writer = Writer::new();
        world.save_components::<Health>(&mut writer);
        let payload = writer.finish();

        // A world where one of those entities never existed.
        let mut other = World::new();
        let only = other.spawn();
        assert_eq!(only, kept);
        let mut reader = Reader::new(&payload);
        assert_eq!(other.load_components::<Health>(&mut reader).unwrap(), 1);
        assert_eq!(other.get::<Health>(kept), Some(&Health(1)));
    }

    #[test]
    fn a_saved_world_encodes_the_same_bytes_twice() {
        let mut world = World::new();
        for i in 0..64 {
            let e = world.spawn();
            world.insert(e, Position(i as f32, -(i as f32)));
            if i % 3 == 0 {
                world.insert(e, Health(i));
            }
        }
        let save = |world: &World| {
            Archive::write(*b"SNAP", 1, |writer| {
                world.save_entities(writer);
                world.save_components::<Position>(writer);
                world.save_components::<Health>(writer);
            })
        };
        let first = save(&world);
        assert_eq!(
            save(&world),
            first,
            "the same world must encode identically"
        );

        let mut loaded = World::new();
        let (_, mut reader) = Archive::open(&first, *b"SNAP", 1).unwrap();
        loaded.load_entities(&mut reader).unwrap();
        loaded.load_components::<Position>(&mut reader).unwrap();
        loaded.load_components::<Health>(&mut reader).unwrap();
        assert_eq!(
            save(&loaded),
            first,
            "and so must a round trip through a save"
        );
    }
}
