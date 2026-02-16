//! Entity replication: selective, delta-compressed component replication
//! from server to clients.
//!
//! The server assigns a [`NetworkId`] to every replicated entity. Each tick,
//! the [`ReplicationServerSystem`] diffs component state against per-client
//! shadow state and produces [`EntityUpdate`], [`SpawnEntity`], or
//! [`DespawnEntity`] messages as needed. The [`ReplicationClientSystem`]
//! applies those messages to the local ECS world.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// NetworkId
// ---------------------------------------------------------------------------

/// Unique network identifier for a replicated entity. Allocated by the server
/// from a monotonically increasing counter. Clients reference entities
/// exclusively by `NetworkId`.
#[derive(Component, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkId(pub u64);

// ---------------------------------------------------------------------------
// ComponentDescriptor
// ---------------------------------------------------------------------------

/// A type-erased descriptor for a replicated component. Contains the
/// [`TypeId`] plus function pointers for serializing from / deserializing
/// into a Bevy [`World`].
pub struct ComponentDescriptor {
    /// Rust [`TypeId`] of the component.
    pub type_id: TypeId,
    /// A stable string tag used in serialized messages (e.g. `"Position128"`).
    pub tag: &'static str,
    /// Serialize the component from `entity` in `world`. Returns `None` if
    /// the entity does not have this component.
    pub serializer: fn(&World, Entity) -> Option<Vec<u8>>,
    /// Deserialize and insert the component onto `entity` in `world`.
    pub deserializer: fn(&mut World, Entity, &[u8]),
}

// ---------------------------------------------------------------------------
// ReplicationSet
// ---------------------------------------------------------------------------

/// Defines which components should be replicated. Components are registered
/// at startup; only registered components participate in replication.
pub struct ReplicationSet {
    descriptors: Vec<ComponentDescriptor>,
}

impl ReplicationSet {
    /// Creates an empty replication set.
    pub fn new() -> Self {
        Self {
            descriptors: Vec::new(),
        }
    }

    /// Registers a component type for replication. The component must
    /// implement `Serialize + DeserializeOwned + Component`.
    pub fn register<T>(&mut self, tag: &'static str)
    where
        T: Component + Serialize + serde::de::DeserializeOwned + Clone,
    {
        let descriptor = ComponentDescriptor {
            type_id: TypeId::of::<T>(),
            tag,
            serializer: |world, entity| {
                world
                    .get::<T>(entity)
                    .and_then(|c| postcard::to_allocvec(c).ok())
            },
            deserializer: |world, entity, bytes| {
                if let Ok(val) = postcard::from_bytes::<T>(bytes) {
                    // Insert or overwrite.
                    if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                        entity_mut.insert(val);
                    }
                }
            },
        };
        self.descriptors.push(descriptor);
    }

    /// Returns the registered descriptors.
    pub fn descriptors(&self) -> &[ComponentDescriptor] {
        &self.descriptors
    }
}

impl Default for ReplicationSet {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Wire messages
// ---------------------------------------------------------------------------

/// A tag identifying a component type in serialized messages.
pub type ComponentTypeTag = String;

/// Update message for changed components on an existing entity.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct EntityUpdate {
    /// The entity's network identifier.
    pub network_id: NetworkId,
    /// Server tick when the update was produced.
    pub tick: u64,
    /// Only the components that changed since the client's last ack.
    pub changed_components: Vec<(ComponentTypeTag, Vec<u8>)>,
}

/// Spawn message: full component snapshot for a newly-visible entity.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SpawnEntity {
    /// The entity's network identifier.
    pub network_id: NetworkId,
    /// All replicated components.
    pub components: Vec<(ComponentTypeTag, Vec<u8>)>,
}

/// Despawn message: entity no longer exists or left interest area.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DespawnEntity {
    /// The entity's network identifier.
    pub network_id: NetworkId,
}

/// Aggregated output of one replication tick for one client.
#[derive(Debug, Clone, Default)]
pub struct ReplicationMessages {
    /// New entities the client should spawn.
    pub spawns: Vec<SpawnEntity>,
    /// Component updates for entities the client already knows about.
    pub updates: Vec<EntityUpdate>,
    /// Entities the client should remove.
    pub despawns: Vec<DespawnEntity>,
}

// ---------------------------------------------------------------------------
// Per-client shadow state
// ---------------------------------------------------------------------------

/// Shadow state for one client: tracks which entities the client knows about
/// and the last-serialized component bytes for delta comparison.
#[derive(Debug, Clone, Default)]
pub(crate) struct ClientShadow {
    /// Map from `NetworkId` → (tag → last-sent bytes).
    pub(crate) entities: HashMap<u64, HashMap<String, Vec<u8>>>,
}

// ---------------------------------------------------------------------------
// ReplicationServerSystem
// ---------------------------------------------------------------------------

/// Server-side replication system. Each tick, compares authoritative component
/// state against per-client shadow state and emits minimal messages.
pub struct ReplicationServerSystem {
    /// Next `NetworkId` to allocate.
    next_network_id: u64,
    /// Per-client shadow state, keyed by client ID.
    shadows: HashMap<u64, ClientShadow>,
}

impl ReplicationServerSystem {
    /// Creates a new server replication system.
    pub fn new() -> Self {
        Self {
            next_network_id: 1,
            shadows: HashMap::new(),
        }
    }

    /// Allocates the next [`NetworkId`].
    pub fn allocate_network_id(&mut self) -> NetworkId {
        let id = NetworkId(self.next_network_id);
        self.next_network_id += 1;
        id
    }

    /// Registers a client so we track shadow state for them.
    pub fn add_client(&mut self, client_id: u64) {
        self.shadows.entry(client_id).or_default();
    }

    /// Removes a client and its shadow state.
    pub fn remove_client(&mut self, client_id: u64) {
        self.shadows.remove(&client_id);
    }

    /// Runs one replication tick. Scans all entities with [`NetworkId`] in
    /// `world`, serializes their replicated components via `rep_set`, and
    /// produces per-client [`ReplicationMessages`].
    pub fn replicate(
        &mut self,
        world: &World,
        rep_set: &ReplicationSet,
        tick: u64,
    ) -> HashMap<u64, ReplicationMessages> {
        // Collect current replicated entities: NetworkId → Entity + serialized components.
        let mut current_entities: HashMap<u64, Vec<(String, Vec<u8>)>> = HashMap::new();
        let mut current_entity_set: HashSet<u64> = HashSet::new();

        // We need a mutable reference for query, but only read. Use unsafe same pattern as authority.
        let world_ptr = world as *const World as *mut World;
        let entities_with_net_id: Vec<(Entity, NetworkId)> = unsafe {
            let mut query = (*world_ptr).query::<(Entity, &NetworkId)>();
            query.iter(&*world_ptr).map(|(e, n)| (e, *n)).collect()
        };

        for (entity, net_id) in &entities_with_net_id {
            current_entity_set.insert(net_id.0);
            let mut components = Vec::new();
            for desc in rep_set.descriptors() {
                if let Some(bytes) = (desc.serializer)(world, *entity) {
                    components.push((desc.tag.to_string(), bytes));
                }
            }
            current_entities.insert(net_id.0, components);
        }

        // For each client, diff against shadow.
        let mut result: HashMap<u64, ReplicationMessages> = HashMap::new();

        for (client_id, shadow) in &mut self.shadows {
            let mut msgs = ReplicationMessages::default();

            // Detect despawns: entities in shadow but not in current.
            let shadow_ids: Vec<u64> = shadow.entities.keys().copied().collect();
            for nid in shadow_ids {
                if !current_entity_set.contains(&nid) {
                    msgs.despawns.push(DespawnEntity {
                        network_id: NetworkId(nid),
                    });
                    shadow.entities.remove(&nid);
                }
            }

            // Detect spawns and updates.
            for (&nid, components) in &current_entities {
                use std::collections::hash_map::Entry;
                match shadow.entities.entry(nid) {
                    Entry::Vacant(vacant) => {
                        // New entity → spawn.
                        msgs.spawns.push(SpawnEntity {
                            network_id: NetworkId(nid),
                            components: components.clone(),
                        });
                        let comp_map: HashMap<String, Vec<u8>> =
                            components.iter().cloned().collect();
                        vacant.insert(comp_map);
                    }
                    Entry::Occupied(mut occupied) => {
                        // Existing entity → delta check.
                        let shadow_comps = occupied.get_mut();
                        let mut changed = Vec::new();
                        for (tag, bytes) in components {
                            let is_changed = match shadow_comps.get(tag.as_str()) {
                                Some(old) => old != bytes,
                                None => true,
                            };
                            if is_changed {
                                changed.push((tag.clone(), bytes.clone()));
                                shadow_comps.insert(tag.clone(), bytes.clone());
                            }
                        }
                        if !changed.is_empty() {
                            msgs.updates.push(EntityUpdate {
                                network_id: NetworkId(nid),
                                tick,
                                changed_components: changed,
                            });
                        }
                    }
                }
            }

            result.insert(*client_id, msgs);
        }

        result
    }
}

impl Default for ReplicationServerSystem {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ReplicationClientSystem
// ---------------------------------------------------------------------------

/// Client-side replication system. Applies [`ReplicationMessages`] from the
/// server to the local ECS [`World`].
pub struct ReplicationClientSystem {
    /// Maps `NetworkId` → local ECS [`Entity`].
    net_to_local: HashMap<u64, Entity>,
}

impl ReplicationClientSystem {
    /// Creates a new client replication system.
    pub fn new() -> Self {
        Self {
            net_to_local: HashMap::new(),
        }
    }

    /// Applies a batch of replication messages to the local `world`.
    pub fn apply(
        &mut self,
        world: &mut World,
        rep_set: &ReplicationSet,
        msgs: &ReplicationMessages,
    ) {
        // Process spawns.
        for spawn in &msgs.spawns {
            let entity = world.spawn(spawn.network_id).id();
            self.net_to_local.insert(spawn.network_id.0, entity);
            for (tag, bytes) in &spawn.components {
                if let Some(desc) = rep_set.descriptors().iter().find(|d| d.tag == tag.as_str()) {
                    (desc.deserializer)(world, entity, bytes);
                }
            }
        }

        // Process updates.
        for update in &msgs.updates {
            if let Some(&entity) = self.net_to_local.get(&update.network_id.0) {
                for (tag, bytes) in &update.changed_components {
                    if let Some(desc) = rep_set.descriptors().iter().find(|d| d.tag == tag.as_str())
                    {
                        (desc.deserializer)(world, entity, bytes);
                    }
                }
            }
        }

        // Process despawns.
        for despawn in &msgs.despawns {
            if let Some(entity) = self.net_to_local.remove(&despawn.network_id.0)
                && world.get_entity(entity).is_ok()
            {
                world.despawn(entity);
            }
        }
    }

    /// Returns the local [`Entity`] for a given [`NetworkId`], if known.
    pub fn local_entity(&self, net_id: NetworkId) -> Option<Entity> {
        self.net_to_local.get(&net_id.0).copied()
    }
}

impl Default for ReplicationClientSystem {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "replication_tests.rs"]
mod tests;
