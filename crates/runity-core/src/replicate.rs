//! Sending the world to somebody else.
//!
//! The server holds the world; every client holds a copy of the part of it
//! that concerns them. Keeping those in step is the whole job, and it has
//! three parts, each of which is a decision rather than a detail:
//!
//! * **Send changes, not state.** A settlement of two thousand entities is
//!   megabytes; what actually changed in a tenth of a second is a few hundred
//!   bytes. Change detection already knows the difference, so replication is
//!   `changed_since(acked_tick)` and nothing more.
//! * **Send only what concerns them.** A player in the north does not need
//!   the harvest in the south. Interest is a filter over entities, and it is
//!   as much a bandwidth decision as a fairness one: what is not sent cannot
//!   be read out of the client's memory by somebody cheating.
//! * **Tolerate not understanding.** Every component's bytes are length
//!   prefixed, so a client from an older build skips what it does not know
//!   instead of desynchronising. Without that, adding a component type is a
//!   breaking change for everyone connected.
//!
//! Entity handles are *not* the same on both sides: the client keeps a map
//! from the server's handles to its own. Pretending otherwise works right up
//! until a client reconnects, and then goes wrong invisibly.

use std::any::TypeId;
use std::collections::HashMap;

use runity_math::Vec3;
use runity_serialize::{Deserialize, Error, Reader, Result, Serialize, Writer};

use crate::world::{Entity, World};

/// Which entities a particular client is told about.
pub trait Interest {
    /// Whether this entity concerns the client.
    fn includes(&self, world: &World, entity: Entity) -> bool;
}

/// Everything, for a client that should see the whole world.
#[derive(Debug, Clone, Copy, Default)]
pub struct Everything;

impl Interest for Everything {
    fn includes(&self, _world: &World, _entity: Entity) -> bool {
        true
    }
}

/// Everything within a radius of a point.
///
/// Position is read through a component the game nominates, so this crate
/// stays out of the question of what "where" means for a given game.
pub struct Nearby<P> {
    /// Centre of the area of interest, usually the player.
    pub centre: Vec3,
    /// How far the client is told about.
    pub radius: f32,
    /// How to find an entity's position.
    pub position: P,
}

impl<P: Fn(&World, Entity) -> Option<Vec3>> Interest for Nearby<P> {
    fn includes(&self, world: &World, entity: Entity) -> bool {
        match (self.position)(world, entity) {
            // Something with no position is not somewhere else, so it is sent:
            // the alternative is a client that never hears about the weather.
            None => true,
            Some(position) => {
                let (dx, dz) = (position.x - self.centre.x, position.z - self.centre.z);
                dx * dx + dz * dz <= self.radius * self.radius
            }
        }
    }
}

/// Appends every instance of one component type that changed since a tick and
/// passes the filter, returning how many that was.
type WriteFn = Box<dyn Fn(&World, u64, &dyn Fn(Entity) -> bool, &mut Writer) -> usize>;

/// Decodes one instance of a component type onto an entity.
type ReadFn = Box<dyn Fn(&mut World, Entity, &mut Reader) -> Result<()>>;

/// How one component type crosses the wire.
struct Replicated {
    id: u16,
    type_id: TypeId,
    write: WriteFn,
    read: ReadFn,
}

/// What a snapshot contained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotStats {
    /// The world tick the snapshot describes.
    pub tick: u64,
    /// Component values written or applied.
    pub components: usize,
    /// Entities reported as gone.
    pub despawns: usize,
    /// Component values skipped because the type was not registered.
    pub unknown: usize,
}

/// The registry of what gets replicated.
#[derive(Default)]
pub struct Replication {
    types: Vec<Replicated>,
}

impl Replication {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replicate a component type under a stable id.
    ///
    /// The id is spelled out rather than derived from the type's name, for the
    /// same reason enum tags are: renaming a Rust type must not silently
    /// change the protocol, and inserting one must not renumber the rest.
    pub fn register<T: Serialize + Deserialize + 'static>(&mut self, id: u16) {
        assert!(
            !self.types.iter().any(|entry| entry.id == id),
            "component id {id} is already registered"
        );
        self.types.push(Replicated {
            id,
            type_id: TypeId::of::<T>(),
            write: Box::new(|world, since, keep, writer| {
                let mut count = 0;
                // Count first, then write: the count has to precede the
                // entries, and a filter means it cannot be known in advance.
                let mut body = Writer::new();
                for (entity, value) in world.changed_since::<T>(since) {
                    if !keep(entity) {
                        continue;
                    }
                    body.write(&entity);
                    let mut payload = Writer::new();
                    value.serialize(&mut payload);
                    body.bytes(payload.as_bytes());
                    count += 1;
                }
                writer.varint(count as u64).raw(body.as_bytes());
                count
            }),
            read: Box::new(|world, entity, reader| {
                let value = T::deserialize(reader)?;
                world.insert(entity, value);
                Ok(())
            }),
        });
    }

    /// How many component types are registered.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Whether a type will be replicated.
    pub fn replicates<T: 'static>(&self) -> bool {
        self.types
            .iter()
            .any(|entry| entry.type_id == TypeId::of::<T>())
    }

    /// Everything that changed since `since` and passes `interest`.
    ///
    /// Take it *after* the tick's systems have run. A change is stamped with
    /// the tick that was current when it happened, so a snapshot taken before
    /// the systems run describes the tick before them — and, because the
    /// client acknowledges the tick it was told, those changes are then never
    /// sent at all.
    pub fn snapshot(&self, world: &World, since: u64, interest: &dyn Interest) -> Vec<u8> {
        self.snapshot_with_stats(world, since, interest).0
    }

    /// The same, and what went into it — for a bandwidth readout.
    pub fn snapshot_with_stats(
        &self,
        world: &World,
        since: u64,
        interest: &dyn Interest,
    ) -> (Vec<u8>, SnapshotStats) {
        let mut writer = Writer::new();
        let mut stats = SnapshotStats {
            tick: world.change_tick(),
            ..Default::default()
        };
        writer.write(&stats.tick);

        // Despawns come from the world's own log rather than from events:
        // an event is readable for one tick, and a client can easily be
        // several behind.
        let gone: Vec<Entity> = world.despawned_since(since).collect();
        stats.despawns = gone.len();
        writer.seq(&gone);

        writer.varint(self.types.len() as u64);
        for entry in &self.types {
            writer.u16(entry.id);
            let keep = |entity: Entity| interest.includes(world, entity);
            stats.components += (entry.write)(world, since, &keep, &mut writer);
        }
        (writer.finish(), stats)
    }

    /// The same, for a client that should see everything.
    pub fn snapshot_all(&self, world: &World, since: u64) -> Vec<u8> {
        self.snapshot(world, since, &Everything)
    }

    fn reader_for(&self, id: u16) -> Option<&Replicated> {
        self.types.iter().find(|entry| entry.id == id)
    }
}

/// The client's copy of somebody else's world.
///
/// Holds the map from the server's entity handles to the local ones. They are
/// not interchangeable: two worlds hand out handles independently, and
/// assuming they match works until the first reconnect.
#[derive(Debug, Default)]
pub struct Replica {
    remote_to_local: HashMap<(u32, u32), Entity>,
    tick: u64,
}

impl Replica {
    /// An empty replica.
    pub fn new() -> Self {
        Self::default()
    }

    /// The newest tick applied.
    ///
    /// This is what the client sends back as its acknowledgement, and what the
    /// server then uses as the `since` of the next snapshot.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// How many entities the replica is tracking.
    pub fn len(&self) -> usize {
        self.remote_to_local.len()
    }

    /// Whether it tracks nothing.
    pub fn is_empty(&self) -> bool {
        self.remote_to_local.is_empty()
    }

    /// The local entity standing in for a remote one.
    pub fn local(&self, remote: Entity) -> Option<Entity> {
        self.remote_to_local
            .get(&(remote.index(), remote.generation()))
            .copied()
    }

    /// Apply a snapshot to the local world.
    pub fn apply(
        &mut self,
        world: &mut World,
        registry: &Replication,
        bytes: &[u8],
    ) -> Result<SnapshotStats> {
        let mut reader = Reader::new(bytes);
        let mut stats = SnapshotStats {
            tick: reader.read()?,
            ..Default::default()
        };

        // An older snapshot arriving late would undo newer state, which shows
        // up as entities twitching backwards. Ignore it.
        if stats.tick <= self.tick && self.tick != 0 {
            return Ok(stats);
        }
        self.tick = stats.tick;

        let gone: Vec<Entity> = reader.seq()?;
        stats.despawns = gone.len();
        for remote in gone {
            if let Some(local) = self
                .remote_to_local
                .remove(&(remote.index(), remote.generation()))
            {
                world.despawn(local);
            }
        }

        let type_count = reader.varint()?;
        for _ in 0..type_count {
            let id = reader.u16()?;
            let count = reader.varint()?;
            if count > reader.remaining() as u64 {
                return Err(Error::LengthOutOfRange {
                    position: reader.position(),
                    length: count,
                    available: reader.remaining(),
                });
            }
            for _ in 0..count {
                let remote: Entity = reader.read()?;
                let payload = reader.bytes()?;
                let local = self.entity_for(world, remote);
                match registry.reader_for(id) {
                    Some(entry) => {
                        // Each value is decoded from its own slice, so a type
                        // whose format has drifted corrupts one component
                        // rather than the rest of the packet.
                        let mut payload = Reader::versioned(payload, stats.tick as u32);
                        (entry.read)(world, local, &mut payload)?;
                        stats.components += 1;
                    }
                    None => stats.unknown += 1,
                }
            }
        }
        Ok(stats)
    }

    /// The local entity for a remote one, spawning it the first time.
    fn entity_for(&mut self, world: &mut World, remote: Entity) -> Entity {
        let key = (remote.index(), remote.generation());
        if let Some(local) = self.remote_to_local.get(&key) {
            if world.is_alive(*local) {
                return *local;
            }
        }
        let local = world.spawn();
        self.remote_to_local.insert(key, local);
        local
    }

    /// Forget everything — after a reconnection, where the server's handles
    /// mean nothing any more.
    pub fn reset(&mut self) {
        self.remote_to_local.clear();
        self.tick = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::vec3;
    use runity_serialize::serializable;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Position(Vec3);
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Health(u32);
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Secret(u32);

    serializable!(Position(0));
    serializable!(Health(0));
    serializable!(Secret(0));

    /// Server and client, wired up the way a game would wire them.
    struct Pair {
        server: World,
        client: World,
        registry: Replication,
        replica: Replica,
        acked: u64,
    }

    impl Pair {
        fn new() -> Self {
            let mut registry = Replication::new();
            registry.register::<Position>(1);
            registry.register::<Health>(2);
            Self {
                server: World::new(),
                client: World::new(),
                registry,
                replica: Replica::new(),
                acked: 0,
            }
        }

        /// One tick: the server advances, then sends what changed.
        fn sync(&mut self) -> SnapshotStats {
            let bytes = self.registry.snapshot_all(&self.server, self.acked);
            let stats = self
                .replica
                .apply(&mut self.client, &self.registry, &bytes)
                .unwrap();
            self.acked = self.replica.tick();
            self.server.forget_despawns_before(self.acked);
            self.server.advance_tick();
            stats
        }

        fn local(&self, remote: Entity) -> Entity {
            self.replica
                .local(remote)
                .expect("the entity should have been replicated")
        }
    }

    #[test]
    fn a_client_receives_the_world_it_has_not_seen() {
        let mut pair = Pair::new();
        let villager = pair.server.spawn();
        pair.server.insert(villager, Position(vec3(1.0, 0.0, 2.0)));
        pair.server.insert(villager, Health(10));

        let stats = pair.sync();
        assert_eq!(stats.components, 2);

        let local = pair.local(villager);
        assert_eq!(
            pair.client.get::<Position>(local),
            Some(&Position(vec3(1.0, 0.0, 2.0)))
        );
        assert_eq!(pair.client.get::<Health>(local), Some(&Health(10)));
    }

    #[test]
    fn only_what_changed_is_sent() {
        let mut pair = Pair::new();
        let still = pair.server.spawn();
        let moving = pair.server.spawn();
        for entity in [still, moving] {
            pair.server.insert(entity, Position(Vec3::ZERO));
            pair.server.insert(entity, Health(5));
        }
        assert_eq!(
            pair.sync().components,
            4,
            "the first snapshot carries everything"
        );

        // Nothing happens.
        assert_eq!(
            pair.sync().components,
            0,
            "an unchanged world costs nothing"
        );

        // One thing moves.
        pair.server.get_mut::<Position>(moving).unwrap().0.x = 4.0;
        let stats = pair.sync();
        assert_eq!(stats.components, 1, "exactly the thing that moved");

        let local = pair.local(moving);
        assert_eq!(pair.client.get::<Position>(local).unwrap().0.x, 4.0);
        assert_eq!(
            pair.client.get::<Position>(pair.local(still)).unwrap().0.x,
            0.0
        );
    }

    #[test]
    fn a_reading_system_on_the_server_does_not_cost_bandwidth() {
        // The trap: if reads marked components as changed, a server that
        // merely looks at its world would send the whole thing every tick.
        let mut pair = Pair::new();
        let entity = pair.server.spawn();
        pair.server.insert(entity, Position(Vec3::ZERO));
        pair.sync();

        for _ in 0..5 {
            let _ = pair.server.get::<Position>(entity);
            let borrowed = pair.server.get_mut::<Position>(entity).unwrap();
            let _read = borrowed.0;
            assert_eq!(pair.sync().components, 0);
        }
    }

    #[test]
    fn a_despawn_reaches_a_client_that_missed_the_event() {
        let mut pair = Pair::new();
        let doomed = pair.server.spawn();
        pair.server.insert(doomed, Health(1));
        pair.sync();
        let local = pair.local(doomed);
        assert!(pair.client.is_alive(local));

        pair.server.despawn(doomed);
        // The server ticks twice before the client hears anything, so the
        // Despawned event itself is long gone by then.
        pair.server.advance_tick();
        pair.server.advance_tick();
        let stats = pair.sync();

        assert_eq!(stats.despawns, 1);
        assert!(!pair.client.is_alive(local), "the copy has to go too");
        assert!(pair.replica.local(doomed).is_none());
    }

    #[test]
    fn the_despawn_log_is_emptied_once_everyone_has_heard() {
        let mut pair = Pair::new();
        let entity = pair.server.spawn();
        pair.server.insert(entity, Health(1));
        pair.sync();

        pair.server.despawn(entity);
        assert_eq!(pair.server.despawn_log(), 1);

        pair.sync();
        assert_eq!(pair.server.despawn_log(), 0, "nobody needs telling twice");
    }

    #[test]
    fn interest_keeps_the_far_side_of_the_world_off_the_wire() {
        let mut pair = Pair::new();
        let near = pair.server.spawn();
        let far = pair.server.spawn();
        pair.server.insert(near, Position(vec3(3.0, 0.0, 0.0)));
        pair.server.insert(far, Position(vec3(400.0, 0.0, 0.0)));

        let interest = Nearby {
            centre: Vec3::ZERO,
            radius: 50.0,
            position: |world: &World, entity: Entity| {
                world.get::<Position>(entity).map(|position| position.0)
            },
        };
        let bytes = pair.registry.snapshot(&pair.server, 0, &interest);
        let stats = pair
            .replica
            .apply(&mut pair.client, &pair.registry, &bytes)
            .unwrap();

        assert_eq!(stats.components, 1, "only the near one crosses");
        assert!(pair.replica.local(near).is_some());
        assert!(
            pair.replica.local(far).is_none(),
            "and the far one is not even known about"
        );
    }

    #[test]
    fn an_unregistered_component_stays_on_the_server() {
        let mut pair = Pair::new();
        let entity = pair.server.spawn();
        pair.server.insert(entity, Position(Vec3::ZERO));
        pair.server.insert(entity, Secret(42));
        pair.sync();

        let local = pair.local(entity);
        assert!(pair.client.get::<Position>(local).is_some());
        assert!(!pair.registry.replicates::<Secret>());
        assert!(
            pair.client.get::<Secret>(local).is_none(),
            "what is not registered never leaves"
        );
    }

    #[test]
    fn a_client_skips_component_types_it_does_not_know() {
        // An older client against a newer server: the unknown type must be
        // stepped over, not treated as corruption.
        let mut server_registry = Replication::new();
        server_registry.register::<Position>(1);
        server_registry.register::<Health>(2);
        server_registry.register::<Secret>(3);

        let mut client_registry = Replication::new();
        client_registry.register::<Position>(1);

        let mut server = World::new();
        let entity = server.spawn();
        server.insert(entity, Position(vec3(9.0, 0.0, 9.0)));
        server.insert(entity, Health(3));
        server.insert(entity, Secret(7));

        let bytes = server_registry.snapshot_all(&server, 0);
        let mut client = World::new();
        let mut replica = Replica::new();
        let stats = replica
            .apply(&mut client, &client_registry, &bytes)
            .unwrap();

        assert_eq!(stats.components, 1, "only Position is understood");
        assert_eq!(
            stats.unknown, 2,
            "and the rest is skipped rather than fatal"
        );
        let local = replica.local(entity).unwrap();
        assert_eq!(
            client.get::<Position>(local),
            Some(&Position(vec3(9.0, 0.0, 9.0)))
        );
    }

    #[test]
    fn a_snapshot_that_arrives_late_is_ignored() {
        let mut pair = Pair::new();
        let entity = pair.server.spawn();
        pair.server.insert(entity, Position(Vec3::ZERO));
        pair.sync();

        // Two snapshots, applied in the wrong order.
        pair.server.get_mut::<Position>(entity).unwrap().0.x = 1.0;
        pair.server.advance_tick();
        let first = pair.registry.snapshot_all(&pair.server, pair.acked);
        pair.server.get_mut::<Position>(entity).unwrap().0.x = 2.0;
        pair.server.advance_tick();
        let second = pair.registry.snapshot_all(&pair.server, pair.acked);

        pair.replica
            .apply(&mut pair.client, &pair.registry, &second)
            .unwrap();
        let stats = pair
            .replica
            .apply(&mut pair.client, &pair.registry, &first)
            .unwrap();
        assert_eq!(stats.components, 0, "the old one must not undo the new one");

        let local = pair.local(entity);
        assert_eq!(pair.client.get::<Position>(local).unwrap().0.x, 2.0);
    }

    #[test]
    fn entity_handles_are_translated_rather_than_assumed() {
        // The client's world is not empty and its handles are its own.
        let mut pair = Pair::new();
        for _ in 0..7 {
            let local_only = pair.client.spawn();
            pair.client.insert(local_only, Health(99));
        }
        let remote = pair.server.spawn();
        pair.server.insert(remote, Health(1));
        pair.sync();

        let local = pair.local(remote);
        assert_ne!(
            local.index(),
            remote.index(),
            "the handles genuinely differ here"
        );
        assert_eq!(pair.client.get::<Health>(local), Some(&Health(1)));
        assert_eq!(
            pair.client.count::<Health>(),
            8,
            "and the local entities are untouched"
        );
    }

    #[test]
    fn a_replica_can_be_reset_for_a_reconnection() {
        let mut pair = Pair::new();
        let entity = pair.server.spawn();
        pair.server.insert(entity, Health(4));
        pair.sync();
        assert!(!pair.replica.is_empty());

        pair.replica.reset();
        assert!(pair.replica.is_empty());
        assert_eq!(pair.replica.tick(), 0);

        // And a full snapshot from scratch rebuilds it.
        let bytes = pair.registry.snapshot_all(&pair.server, 0);
        pair.replica
            .apply(&mut pair.client, &pair.registry, &bytes)
            .unwrap();
        assert_eq!(pair.replica.len(), 1);
    }

    #[test]
    fn a_snapshot_is_the_same_bytes_every_time() {
        let mut pair = Pair::new();
        for index in 0..20 {
            let entity = pair.server.spawn();
            pair.server
                .insert(entity, Position(vec3(index as f32, 0.0, 0.0)));
            pair.server.insert(entity, Health(index));
        }
        let first = pair.registry.snapshot_all(&pair.server, 0);
        for _ in 0..5 {
            assert_eq!(pair.registry.snapshot_all(&pair.server, 0), first);
        }
    }

    #[test]
    fn rubbish_is_rejected_rather_than_applied() {
        let mut pair = Pair::new();
        let entity = pair.server.spawn();
        pair.server.insert(entity, Position(Vec3::ZERO));
        let good = pair.registry.snapshot_all(&pair.server, 0);

        // Every truncation of a valid snapshot, and some noise.
        for length in 0..good.len() {
            let mut world = World::new();
            let mut replica = Replica::new();
            let _ = replica.apply(&mut world, &pair.registry, &good[..length]);
        }
        let mut rng = runity_math::Rng::named(1, "snapshot fuzz");
        for _ in 0..2_000 {
            let length = rng.below(64) as usize;
            let noise: Vec<u8> = (0..length).map(|_| rng.below(256) as u8).collect();
            let mut world = World::new();
            let mut replica = Replica::new();
            let _ = replica.apply(&mut world, &pair.registry, &noise);
        }
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn registering_two_types_under_one_id_is_refused() {
        let mut registry = Replication::new();
        registry.register::<Position>(1);
        registry.register::<Health>(1);
    }
}
