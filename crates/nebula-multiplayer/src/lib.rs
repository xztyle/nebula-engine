//! High-level multiplayer: server-authoritative state, entity replication,
//! client-side prediction, and session management.

pub mod authority;
pub mod budget;
pub mod chat;
pub mod chunk_streaming;
pub mod clock;
pub mod interest;
pub mod player_session;
pub mod prediction;
pub mod reconciliation;
pub mod replication;
pub mod snapshot;
pub mod voxel_edit;

pub use authority::{
    AuthoritativeWorld, ClientIntent, IntentValidationError, IntentValidator, PlayerState,
    ServerTickSchedule,
};
pub use budget::{
    AdaptiveRate, BandwidthConfig, BandwidthStats, ClientBandwidthTracker, ClientId,
    MessagePriority, MessageSender, PrioritizedMessage, send_tick_messages,
};
pub use chat::{
    ChatConfig, ChatMessage, ChatMessageIntent, ChatRejection, ChatScope, ConnectedClient,
    RateTracker, broadcast_chat, validate_chat_message,
};
pub use chunk_streaming::{
    ChunkDataMessage, ChunkDecompressError, ChunkId, ChunkSendEntry, ChunkSendQueue,
    ChunkStreamConfig, ClientChunkCache, compress_chunk, decompress_chunk,
};
pub use clock::{
    ClockSync, Ping, Pong, RttEstimator, TICK_DURATION, TICK_RATE, TickAdjustment, TickCounter,
    compute_tick_adjustment,
};
pub use interest::{
    ClientInterestSet, InterestArea, InterestPosition, InterestTransitions, SpatialInterestSystem,
    TrackedEntity, within_interest,
};
pub use player_session::{
    AuthResult, ConnectionRequest, ConnectionState, DisconnectReason, DisconnectRequest,
    InitialWorldState, PROTOCOL_VERSION, PlayerSaveData,
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
pub use snapshot::{
    CURRENT_SNAPSHOT_VERSION, ChunkSnapshot, DirtyChunkTracker, EntitySnapshot, SnapshotConfig,
    SnapshotError, SnapshotHeader, SnapshotTimer, WorldSnapshot, check_version, load_snapshot,
    write_snapshot,
};
pub use voxel_edit::{
    EditRejection, PlayerPosition, ServerChunkStore, VoxelEditEvent, VoxelEditIntent,
    VoxelMaterial, apply_voxel_edit, validate_voxel_edit,
};
