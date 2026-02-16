//! Voxel modification event system.
//!
//! Provides [`VoxelModifiedEvent`] and [`VoxelBatchModifiedEvent`] for notifying
//! downstream systems (meshing, lighting, physics, networking) when voxels change.
//! Events are collected into a [`VoxelEventBuffer`] that is double-buffered per frame.

use crate::chunk_manager::ChunkAddress;
use crate::registry::VoxelTypeId;

/// Emitted when a single voxel in a chunk is modified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoxelModifiedEvent {
    /// The chunk containing the modified voxel.
    pub chunk: ChunkAddress,
    /// Local position within the chunk (each component in `[0, 32)`).
    pub local_pos: (u8, u8, u8),
    /// The voxel type that was at this position before the modification.
    pub old_type: VoxelTypeId,
    /// The voxel type now at this position after the modification.
    pub new_type: VoxelTypeId,
}

/// Emitted for bulk modifications (e.g., explosions, terrain generation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoxelBatchModifiedEvent {
    /// The chunk that was modified.
    pub chunk: ChunkAddress,
    /// Number of voxels changed in this batch.
    pub count: u32,
}

/// Double-buffered event storage for voxel modifications.
///
/// Events written in the current frame are readable in the current and next frame.
/// After two [`swap`](VoxelEventBuffer::swap) calls, events are dropped.
/// Call [`swap`](VoxelEventBuffer::swap) once per frame.
pub struct VoxelEventBuffer {
    /// Events from the previous frame (readable).
    prev: Vec<VoxelModifiedEvent>,
    /// Events from the current frame (being written).
    current: Vec<VoxelModifiedEvent>,
    /// Batch events from the previous frame.
    batch_prev: Vec<VoxelBatchModifiedEvent>,
    /// Batch events from the current frame.
    batch_current: Vec<VoxelBatchModifiedEvent>,
}

impl VoxelEventBuffer {
    /// Creates a new empty event buffer.
    pub fn new() -> Self {
        Self {
            prev: Vec::new(),
            current: Vec::new(),
            batch_prev: Vec::new(),
            batch_current: Vec::new(),
        }
    }

    /// Sends a single voxel modification event.
    pub fn send(&mut self, event: VoxelModifiedEvent) {
        self.current.push(event);
    }

    /// Sends a batch modification event.
    pub fn send_batch(&mut self, event: VoxelBatchModifiedEvent) {
        self.batch_current.push(event);
    }

    /// Returns all readable voxel modification events (previous + current frame).
    pub fn read(&self) -> impl Iterator<Item = &VoxelModifiedEvent> {
        self.prev.iter().chain(self.current.iter())
    }

    /// Returns all readable batch modification events (previous + current frame).
    pub fn read_batch(&self) -> impl Iterator<Item = &VoxelBatchModifiedEvent> {
        self.batch_prev.iter().chain(self.batch_current.iter())
    }

    /// Returns the number of readable voxel modification events.
    pub fn len(&self) -> usize {
        self.prev.len() + self.current.len()
    }

    /// Returns `true` if there are no readable events.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of readable batch events.
    pub fn batch_len(&self) -> usize {
        self.batch_prev.len() + self.batch_current.len()
    }

    /// Advances the frame: previous events are dropped, current becomes previous.
    ///
    /// Call this once per frame before writing new events.
    pub fn swap(&mut self) {
        self.prev.clear();
        std::mem::swap(&mut self.prev, &mut self.current);
        self.batch_prev.clear();
        std::mem::swap(&mut self.batch_prev, &mut self.batch_current);
    }

    /// Clears all events from both buffers.
    pub fn clear(&mut self) {
        self.prev.clear();
        self.current.clear();
        self.batch_prev.clear();
        self.batch_current.clear();
    }
}

impl Default for VoxelEventBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Sets a voxel in the chunk manager and emits a [`VoxelModifiedEvent`] if the type changed.
///
/// Returns `true` if the voxel was actually modified (type changed).
/// Returns `false` if the chunk was not found or the voxel already had the requested type.
pub fn set_voxel(
    chunk_manager: &mut crate::chunk_manager::ChunkManager,
    addr: &ChunkAddress,
    x: u8,
    y: u8,
    z: u8,
    new_type: VoxelTypeId,
    events: &mut VoxelEventBuffer,
) -> bool {
    let Some(chunk) = chunk_manager.get_chunk_mut(addr) else {
        return false;
    };

    let old_type = chunk.get(x, y, z);

    // Skip if the voxel is already the requested type.
    if old_type == new_type {
        return false;
    }

    chunk.set(x, y, z, new_type);

    events.send(VoxelModifiedEvent {
        chunk: *addr,
        local_pos: (x, y, z),
        old_type,
        new_type,
    });

    true
}

/// Sets multiple voxels in a chunk and emits individual events plus a batch event.
///
/// Each `(x, y, z, type)` tuple is applied in order. Only voxels that actually
/// change emit events. Returns the number of voxels modified.
pub fn set_voxels_batch(
    chunk_manager: &mut crate::chunk_manager::ChunkManager,
    addr: &ChunkAddress,
    modifications: &[(u8, u8, u8, VoxelTypeId)],
    events: &mut VoxelEventBuffer,
) -> u32 {
    let Some(chunk) = chunk_manager.get_chunk_mut(addr) else {
        return 0;
    };

    let mut count = 0u32;

    for &(x, y, z, new_type) in modifications {
        let old_type = chunk.get(x, y, z);
        if old_type == new_type {
            continue;
        }
        chunk.set(x, y, z, new_type);
        events.send(VoxelModifiedEvent {
            chunk: *addr,
            local_pos: (x, y, z),
            old_type,
            new_type,
        });
        count += 1;
    }

    if count > 0 {
        events.send_batch(VoxelBatchModifiedEvent {
            chunk: *addr,
            count,
        });
    }

    count
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk_api::Chunk;
    use crate::chunk_manager::ChunkManager;

    fn addr(x: i64, y: i64, z: i64) -> ChunkAddress {
        ChunkAddress::new(x, y, z, 0)
    }

    #[test]
    fn test_set_voxel_emits_event() {
        let mut mgr = ChunkManager::new();
        let a = addr(0, 3, 0);
        mgr.load_chunk(a, Chunk::new());

        let mut events = VoxelEventBuffer::new();
        let stone = VoxelTypeId(1);

        set_voxel(&mut mgr, &a, 5, 17, 8, stone, &mut events);

        let evts: Vec<_> = events.read().collect();
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].chunk, a);
        assert_eq!(evts[0].local_pos, (5, 17, 8));
        assert_eq!(evts[0].old_type, VoxelTypeId(0));
        assert_eq!(evts[0].new_type, stone);
    }

    #[test]
    fn test_batch_set_emits_batch() {
        let mut mgr = ChunkManager::new();
        let a = addr(0, 0, 0);
        mgr.load_chunk(a, Chunk::new());

        let mut events = VoxelEventBuffer::new();
        let stone = VoxelTypeId(1);

        let mods: Vec<_> = (0..10).map(|i| (i, 0, 0, stone)).collect();
        let count = set_voxels_batch(&mut mgr, &a, &mods, &mut events);

        assert_eq!(count, 10);

        let individual: Vec<_> = events.read().collect();
        assert_eq!(individual.len(), 10);

        let batch: Vec<_> = events.read_batch().collect();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].count, 10);
        assert_eq!(batch[0].chunk, a);
    }

    #[test]
    fn test_event_contains_correct_old_new_types() {
        let mut mgr = ChunkManager::new();
        let a = addr(0, 0, 0);
        mgr.load_chunk(a, Chunk::new());

        let mut events = VoxelEventBuffer::new();
        let stone = VoxelTypeId(1);
        let dirt = VoxelTypeId(2);

        set_voxel(&mut mgr, &a, 5, 5, 5, stone, &mut events);
        set_voxel(&mut mgr, &a, 5, 5, 5, dirt, &mut events);

        let evts: Vec<_> = events.read().collect();
        assert_eq!(evts.len(), 2);
        assert_eq!(evts[1].old_type, stone);
        assert_eq!(evts[1].new_type, dirt);
    }

    #[test]
    fn test_no_event_on_set_to_same_type() {
        let mut mgr = ChunkManager::new();
        let a = addr(0, 0, 0);
        mgr.load_chunk(a, Chunk::new());

        let mut events = VoxelEventBuffer::new();
        let stone = VoxelTypeId(1);

        set_voxel(&mut mgr, &a, 0, 0, 0, stone, &mut events);
        let result = set_voxel(&mut mgr, &a, 0, 0, 0, stone, &mut events);

        assert!(!result);
        assert_eq!(events.read().count(), 1);
    }

    #[test]
    fn test_events_cleared_after_frame() {
        let mut events = VoxelEventBuffer::new();
        events.send(VoxelModifiedEvent {
            chunk: addr(0, 0, 0),
            local_pos: (0, 0, 0),
            old_type: VoxelTypeId(0),
            new_type: VoxelTypeId(1),
        });

        // Frame N: event is readable
        assert_eq!(events.len(), 1);

        // Frame N+1: swap — event moves to prev, still readable
        events.swap();
        assert_eq!(events.len(), 1);

        // Frame N+2: swap — prev is cleared, event is gone
        events.swap();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_set_voxel_missing_chunk_returns_false() {
        let mut mgr = ChunkManager::new();
        let mut events = VoxelEventBuffer::new();
        let result = set_voxel(
            &mut mgr,
            &addr(99, 99, 99),
            0,
            0,
            0,
            VoxelTypeId(1),
            &mut events,
        );
        assert!(!result);
        assert!(events.is_empty());
    }

    #[test]
    fn test_batch_skips_same_type() {
        let mut mgr = ChunkManager::new();
        let a = addr(0, 0, 0);
        mgr.load_chunk(a, Chunk::new());

        let mut events = VoxelEventBuffer::new();
        // Air(0) -> Air(0) should be skipped
        let mods = vec![
            (0, 0, 0, VoxelTypeId(0)),
            (1, 0, 0, VoxelTypeId(1)),
            (2, 0, 0, VoxelTypeId(0)),
        ];
        let count = set_voxels_batch(&mut mgr, &a, &mods, &mut events);
        assert_eq!(count, 1);
        assert_eq!(events.read().count(), 1);
    }
}
