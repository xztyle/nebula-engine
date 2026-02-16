//! High-level multiplayer: server-authoritative state, entity replication,
//! client-side prediction, and session management.

pub mod authority;
pub mod interest;
pub mod prediction;
pub mod reconciliation;
pub mod replication;

pub use authority::{
    AuthoritativeWorld, ClientIntent, IntentValidationError, IntentValidator, PlayerState,
    ServerTickSchedule,
};
pub use interest::{
    ClientInterestSet, InterestArea, InterestPosition, InterestTransitions, SpatialInterestSystem,
    TrackedEntity, within_interest,
};
pub use prediction::{
    InputBuffer, InputEntry, MovementResult, PredictionState, client_prediction_step,
    simulate_movement,
};
pub use reconciliation::{
    AuthoritativePlayerState, CorrectionSmoothing, ReconciliationResult, positions_match, reconcile,
};
pub use replication::{
    ComponentDescriptor, ComponentTypeTag, DespawnEntity, EntityUpdate, NetworkId,
    ReplicationClientSystem, ReplicationMessages, ReplicationServerSystem, ReplicationSet,
    SpawnEntity,
};
