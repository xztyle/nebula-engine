//! High-level multiplayer: server-authoritative state, entity replication,
//! client-side prediction, and session management.

pub mod authority;
pub mod replication;

pub use authority::{
    AuthoritativeWorld, ClientIntent, IntentValidationError, IntentValidator, PlayerState,
    ServerTickSchedule,
};
pub use replication::{
    ComponentDescriptor, ComponentTypeTag, DespawnEntity, EntityUpdate, NetworkId,
    ReplicationClientSystem, ReplicationMessages, ReplicationServerSystem, ReplicationSet,
    SpawnEntity,
};
