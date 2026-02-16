//! High-level multiplayer: server-authoritative state, entity replication,
//! client-side prediction, and session management.

pub mod authority;
pub mod interest;
pub mod replication;

pub use authority::{
    AuthoritativeWorld, ClientIntent, IntentValidationError, IntentValidator, PlayerState,
    ServerTickSchedule,
};
pub use interest::{
    ClientInterestSet, InterestArea, InterestPosition, InterestTransitions, SpatialInterestSystem,
    TrackedEntity, within_interest,
};
pub use replication::{
    ComponentDescriptor, ComponentTypeTag, DespawnEntity, EntityUpdate, NetworkId,
    ReplicationClientSystem, ReplicationMessages, ReplicationServerSystem, ReplicationSet,
    SpawnEntity,
};
