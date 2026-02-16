//! Voxel edit replication: intent validation, application, and broadcast.
//!
//! Clients submit [`VoxelEditIntent`] messages to the server, which validates
//! them against the [`AuthoritativeWorld`] state, applies valid edits, and
//! broadcasts [`VoxelEditEvent`] messages to interested clients.

use serde::{Deserialize, Serialize};

use crate::chunk_streaming::ChunkId;
use crate::replication::NetworkId;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default interaction radius in millimeters (6 meters).
const INTERACTION_RADIUS_MM: i64 = 6_000;

/// Chunk size in voxels per axis.
const CHUNK_SIZE: u32 = 32;

// ---------------------------------------------------------------------------
// VoxelMaterial
// ---------------------------------------------------------------------------

/// Lightweight voxel material identifier used in edit messages.
///
/// `Air` (0) represents empty space; all other values are solid materials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VoxelMaterial(pub u16);

impl VoxelMaterial {
    /// Air / empty voxel.
    pub const AIR: Self = Self(0);
    /// Stone material (for tests/demos).
    pub const STONE: Self = Self(1);
    /// Dirt material (for tests/demos).
    pub const DIRT: Self = Self(2);

    /// Returns `true` if this material is air (empty).
    pub fn is_air(self) -> bool {
        self.0 == 0
    }
}

// ---------------------------------------------------------------------------
// VoxelEditIntent
// ---------------------------------------------------------------------------

/// A client's request to modify a single voxel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VoxelEditIntent {
    /// Place a block of the given material.
    Place {
        /// Target chunk.
        chunk_id: ChunkId,
        /// Local position within the chunk (each axis 0..CHUNK_SIZE).
        local_x: u32,
        /// Local Y.
        local_y: u32,
        /// Local Z.
        local_z: u32,
        /// Material to place.
        material: VoxelMaterial,
        /// Inventory slot sourcing the material.
        source_inventory_slot: u8,
    },
    /// Remove (break) the block at the given position.
    Remove {
        /// Target chunk.
        chunk_id: ChunkId,
        /// Local X.
        local_x: u32,
        /// Local Y.
        local_y: u32,
        /// Local Z.
        local_z: u32,
    },
}

impl VoxelEditIntent {
    /// Returns the target [`ChunkId`].
    pub fn chunk_id(&self) -> ChunkId {
        match self {
            Self::Place { chunk_id, .. } | Self::Remove { chunk_id, .. } => *chunk_id,
        }
    }
}

// ---------------------------------------------------------------------------
// VoxelEditEvent (broadcast to clients)
// ---------------------------------------------------------------------------

/// Broadcast message informing clients of a confirmed voxel change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoxelEditEvent {
    /// Affected chunk.
    pub chunk_id: ChunkId,
    /// Local X within the chunk.
    pub local_x: u32,
    /// Local Y within the chunk.
    pub local_y: u32,
    /// Local Z within the chunk.
    pub local_z: u32,
    /// The new material at this position (Air for removal).
    pub new_material: VoxelMaterial,
    /// Network ID of the player who made the edit.
    pub editor_network_id: NetworkId,
    /// Server tick when the edit was applied.
    pub tick: u64,
}

// ---------------------------------------------------------------------------
// EditRejection
// ---------------------------------------------------------------------------

/// Reason a voxel edit intent was rejected by the server.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditRejection {
    /// Target voxel is outside the player's interaction radius.
    #[error("target out of range")]
    OutOfRange,
    /// The target chunk is not loaded on the server.
    #[error("chunk not loaded")]
    ChunkNotLoaded,
    /// `local_pos` is outside chunk bounds (0..CHUNK_SIZE).
    #[error("position out of bounds: ({x}, {y}, {z})")]
    PositionOutOfBounds {
        /// X.
        x: u32,
        /// Y.
        y: u32,
        /// Z.
        z: u32,
    },
    /// Attempted to place a block at a non-air position.
    #[error("target voxel is obstructed")]
    Obstructed,
    /// Attempted to remove an air voxel.
    #[error("target voxel is already empty")]
    AlreadyEmpty,
    /// Player entity not found.
    #[error("unknown player")]
    UnknownPlayer,
}

// ---------------------------------------------------------------------------
// ServerChunkStore — minimal chunk voxel storage for validation
// ---------------------------------------------------------------------------

/// Minimal per-chunk voxel storage used by the authoritative server to track
/// placed/removed voxels for validation purposes.
#[derive(Debug, Default)]
pub struct ServerChunkStore {
    /// Loaded chunks: ChunkId → flat voxel array (CHUNK_SIZE³).
    chunks: std::collections::HashMap<ChunkId, Vec<VoxelMaterial>>,
}

impl ServerChunkStore {
    /// Create an empty chunk store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load (or create) a chunk filled with the given material.
    pub fn load_chunk(&mut self, id: ChunkId, fill: VoxelMaterial) {
        let size = (CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE) as usize;
        self.chunks.insert(id, vec![fill; size]);
    }

    /// Returns `true` if the chunk is loaded.
    pub fn is_loaded(&self, id: &ChunkId) -> bool {
        self.chunks.contains_key(id)
    }

    /// Get the material at a local position.
    pub fn get_voxel(&self, id: &ChunkId, x: u32, y: u32, z: u32) -> Option<VoxelMaterial> {
        let data = self.chunks.get(id)?;
        let idx = (x * CHUNK_SIZE * CHUNK_SIZE + y * CHUNK_SIZE + z) as usize;
        data.get(idx).copied()
    }

    /// Set the material at a local position.
    pub fn set_voxel(&mut self, id: &ChunkId, x: u32, y: u32, z: u32, mat: VoxelMaterial) {
        if let Some(data) = self.chunks.get_mut(id) {
            let idx = (x * CHUNK_SIZE * CHUNK_SIZE + y * CHUNK_SIZE + z) as usize;
            if idx < data.len() {
                data[idx] = mat;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Player position helper
// ---------------------------------------------------------------------------

/// Minimal player position for range checking (millimetre coords).
#[derive(Debug, Clone)]
pub struct PlayerPosition {
    /// X in millimetres.
    pub x: i64,
    /// Y in millimetres.
    pub y: i64,
    /// Z in millimetres.
    pub z: i64,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validates a [`VoxelEditIntent`] against the server state.
///
/// # Errors
///
/// Returns [`EditRejection`] if the edit is invalid.
pub fn validate_voxel_edit(
    player_pos: &PlayerPosition,
    edit: &VoxelEditIntent,
    store: &ServerChunkStore,
) -> Result<(), EditRejection> {
    let (chunk_id, lx, ly, lz) = match edit {
        VoxelEditIntent::Place {
            chunk_id,
            local_x,
            local_y,
            local_z,
            ..
        } => (*chunk_id, *local_x, *local_y, *local_z),
        VoxelEditIntent::Remove {
            chunk_id,
            local_x,
            local_y,
            local_z,
        } => (*chunk_id, *local_x, *local_y, *local_z),
    };

    // Bounds check.
    if lx >= CHUNK_SIZE || ly >= CHUNK_SIZE || lz >= CHUNK_SIZE {
        return Err(EditRejection::PositionOutOfBounds {
            x: lx,
            y: ly,
            z: lz,
        });
    }

    // Chunk loaded check.
    if !store.is_loaded(&chunk_id) {
        return Err(EditRejection::ChunkNotLoaded);
    }

    // Range check (simplified: use chunk origin + local offset as world pos).
    let world_x = chunk_id.x as i64 * CHUNK_SIZE as i64 * 1000 + lx as i64 * 1000;
    let world_y = chunk_id.y as i64 * CHUNK_SIZE as i64 * 1000 + ly as i64 * 1000;
    let world_z = chunk_id.z as i64 * CHUNK_SIZE as i64 * 1000 + lz as i64 * 1000;

    let dx = world_x - player_pos.x;
    let dy = world_y - player_pos.y;
    let dz = world_z - player_pos.z;
    let dist_sq = dx * dx + dy * dy + dz * dz;
    let max_sq = INTERACTION_RADIUS_MM * INTERACTION_RADIUS_MM;
    if dist_sq > max_sq {
        return Err(EditRejection::OutOfRange);
    }

    // Obstruction / emptiness check.
    let current = store
        .get_voxel(&chunk_id, lx, ly, lz)
        .unwrap_or(VoxelMaterial::AIR);

    match edit {
        VoxelEditIntent::Place { .. } => {
            if !current.is_air() {
                return Err(EditRejection::Obstructed);
            }
        }
        VoxelEditIntent::Remove { .. } => {
            if current.is_air() {
                return Err(EditRejection::AlreadyEmpty);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------

/// Applies a **pre-validated** voxel edit to the chunk store and returns the
/// [`VoxelEditEvent`] to broadcast.
pub fn apply_voxel_edit(
    edit: &VoxelEditIntent,
    store: &mut ServerChunkStore,
    editor_network_id: NetworkId,
    tick: u64,
) -> VoxelEditEvent {
    match edit {
        VoxelEditIntent::Place {
            chunk_id,
            local_x,
            local_y,
            local_z,
            material,
            ..
        } => {
            store.set_voxel(chunk_id, *local_x, *local_y, *local_z, *material);
            VoxelEditEvent {
                chunk_id: *chunk_id,
                local_x: *local_x,
                local_y: *local_y,
                local_z: *local_z,
                new_material: *material,
                editor_network_id,
                tick,
            }
        }
        VoxelEditIntent::Remove {
            chunk_id,
            local_x,
            local_y,
            local_z,
        } => {
            store.set_voxel(chunk_id, *local_x, *local_y, *local_z, VoxelMaterial::AIR);
            VoxelEditEvent {
                chunk_id: *chunk_id,
                local_x: *local_x,
                local_y: *local_y,
                local_z: *local_z,
                new_material: VoxelMaterial::AIR,
                editor_network_id,
                tick,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interest::{InterestPosition, within_interest};
    use crate::replication::NetworkId;

    fn test_chunk_id() -> ChunkId {
        ChunkId {
            face: 0,
            lod: 0,
            x: 0,
            y: 0,
            z: 0,
        }
    }

    fn near_player() -> PlayerPosition {
        PlayerPosition {
            x: 500,
            y: 500,
            z: 500,
        }
    }

    fn setup_store(chunk_id: ChunkId, fill: VoxelMaterial) -> ServerChunkStore {
        let mut store = ServerChunkStore::new();
        store.load_chunk(chunk_id, fill);
        store
    }

    #[test]
    fn test_valid_edit_is_applied_and_broadcast() {
        let cid = test_chunk_id();
        let mut store = setup_store(cid, VoxelMaterial::AIR);
        let player = near_player();
        let net_id = NetworkId(1);

        let intent = VoxelEditIntent::Place {
            chunk_id: cid,
            local_x: 0,
            local_y: 0,
            local_z: 0,
            material: VoxelMaterial::STONE,
            source_inventory_slot: 0,
        };

        assert!(validate_voxel_edit(&player, &intent, &store).is_ok());

        let event = apply_voxel_edit(&intent, &mut store, net_id, 10);
        assert_eq!(event.new_material, VoxelMaterial::STONE);
        assert_eq!(event.tick, 10);
        assert_eq!(event.editor_network_id, net_id);

        // Voxel is now stone in the store.
        assert_eq!(store.get_voxel(&cid, 0, 0, 0), Some(VoxelMaterial::STONE));
    }

    #[test]
    fn test_invalid_edit_is_rejected() {
        let cid = ChunkId {
            face: 0,
            lod: 0,
            x: 100,
            y: 100,
            z: 100,
        };
        let store = setup_store(cid, VoxelMaterial::AIR);
        // Player at origin — chunk at (100,100,100) is far away.
        let player = PlayerPosition { x: 0, y: 0, z: 0 };

        let intent = VoxelEditIntent::Place {
            chunk_id: cid,
            local_x: 5,
            local_y: 5,
            local_z: 5,
            material: VoxelMaterial::STONE,
            source_inventory_slot: 0,
        };

        let err = validate_voxel_edit(&player, &intent, &store).unwrap_err();
        assert_eq!(err, EditRejection::OutOfRange);
    }

    #[test]
    fn test_edit_reaches_all_interested_clients() {
        let cid = test_chunk_id();
        let mut store = setup_store(cid, VoxelMaterial::AIR);
        let player = near_player();

        let intent = VoxelEditIntent::Place {
            chunk_id: cid,
            local_x: 1,
            local_y: 1,
            local_z: 1,
            material: VoxelMaterial::DIRT,
            source_inventory_slot: 0,
        };

        validate_voxel_edit(&player, &intent, &store).unwrap();
        let event = apply_voxel_edit(&intent, &mut store, NetworkId(1), 5);

        // Chunk world position at origin.
        let chunk_pos = InterestPosition::new(0.0, 0.0, 0.0);

        // Two nearby clients, one far away.
        let client_a = InterestPosition::new(100.0, 100.0, 100.0);
        let client_b = InterestPosition::new(200.0, 200.0, 200.0);
        let client_c = InterestPosition::new(999_999.0, 999_999.0, 999_999.0);

        let interest_radius = 5000.0;

        let a_receives = within_interest(&client_a, &chunk_pos, interest_radius);
        let b_receives = within_interest(&client_b, &chunk_pos, interest_radius);
        let c_receives = within_interest(&client_c, &chunk_pos, interest_radius);

        assert!(a_receives, "client A should receive the edit");
        assert!(b_receives, "client B should receive the edit");
        assert!(!c_receives, "client C should NOT receive the edit");

        // Event data is correct.
        assert_eq!(event.new_material, VoxelMaterial::DIRT);
    }

    #[test]
    fn test_edit_modifies_correct_voxel() {
        let cid = test_chunk_id();
        let mut store = setup_store(cid, VoxelMaterial::AIR);
        // Player close to target voxel world pos (5000, 10000, 3000) mm.
        let player = PlayerPosition {
            x: 5000,
            y: 10000,
            z: 3000,
        };

        let intent = VoxelEditIntent::Place {
            chunk_id: cid,
            local_x: 5,
            local_y: 10,
            local_z: 3,
            material: VoxelMaterial::STONE,
            source_inventory_slot: 0,
        };

        validate_voxel_edit(&player, &intent, &store).unwrap();
        apply_voxel_edit(&intent, &mut store, NetworkId(1), 1);

        // Target voxel is now stone.
        assert_eq!(store.get_voxel(&cid, 5, 10, 3), Some(VoxelMaterial::STONE));

        // Adjacent voxels are still air.
        assert_eq!(store.get_voxel(&cid, 4, 10, 3), Some(VoxelMaterial::AIR));
        assert_eq!(store.get_voxel(&cid, 5, 11, 3), Some(VoxelMaterial::AIR));
        assert_eq!(store.get_voxel(&cid, 5, 10, 4), Some(VoxelMaterial::AIR));
    }

    #[test]
    fn test_concurrent_edits_dont_conflict() {
        let cid = test_chunk_id();
        let mut store = setup_store(cid, VoxelMaterial::AIR);
        let player = near_player();

        let intent_a = VoxelEditIntent::Place {
            chunk_id: cid,
            local_x: 2,
            local_y: 2,
            local_z: 2,
            material: VoxelMaterial::STONE,
            source_inventory_slot: 0,
        };

        let intent_b = VoxelEditIntent::Place {
            chunk_id: cid,
            local_x: 2,
            local_y: 2,
            local_z: 2,
            material: VoxelMaterial::DIRT,
            source_inventory_slot: 1,
        };

        // First edit succeeds.
        assert!(validate_voxel_edit(&player, &intent_a, &store).is_ok());
        apply_voxel_edit(&intent_a, &mut store, NetworkId(1), 1);

        // Second edit to same position is rejected (obstructed).
        let err = validate_voxel_edit(&player, &intent_b, &store).unwrap_err();
        assert_eq!(err, EditRejection::Obstructed);

        // Final state is from first edit.
        assert_eq!(store.get_voxel(&cid, 2, 2, 2), Some(VoxelMaterial::STONE));
    }
}
