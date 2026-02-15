# Async Mesh Generation

## Problem

Greedy meshing (story 02) with AO (story 04) for a 32x32x32 chunk takes on the order of hundreds of microseconds. While a single chunk meshes fast, the engine must mesh dozens to hundreds of chunks per frame during initial loading, player movement across chunk boundaries, and terrain editing. Running all meshing on the main thread would consume the entire frame budget (16ms at 60fps), stalling rendering, input handling, and game logic. The meshing workload must be offloaded to background threads.

However, meshing requires read access to chunk voxel data and neighbor data. Holding locks on world data while meshing on a thread pool would block the main thread from processing edits or loading new chunks. The solution requires a snapshot-based approach: capture the data needed for meshing, hand it to a worker, and receive the result asynchronously.

## Solution

Implement an asynchronous meshing pipeline in the `nebula_meshing` crate using a thread pool, snapshot-based tasks, and channels for result delivery.

### MeshingTask

A `MeshingTask` encapsulates everything a worker thread needs to produce a `ChunkMesh` — a snapshot of the chunk data and its neighborhood, plus a reference to the immutable voxel type registry.

```rust
/// A self-contained meshing task that can run on any thread.
pub struct MeshingTask {
    /// The chunk position this mesh is for (used to match results to chunks).
    pub chunk_pos: ChunkPosition,
    /// Snapshot of the chunk's voxel data (owned, no references into world).
    pub neighborhood: ChunkNeighborhood,
    /// Version number of the chunk data at snapshot time.
    pub data_version: u64,
}

/// The result of a completed meshing task.
pub struct MeshingResult {
    pub chunk_pos: ChunkPosition,
    pub mesh: ChunkMesh,
    pub data_version: u64,
}
```

### MeshingPipeline

The pipeline manages task submission, worker coordination, and result collection.

```rust
pub struct MeshingPipeline {
    /// Channel sender for submitting tasks to workers.
    task_sender: crossbeam_channel::Sender<MeshingTask>,
    /// Channel receiver for collecting completed results on the main thread.
    result_receiver: crossbeam_channel::Receiver<MeshingResult>,
    /// Handle to the thread pool (for shutdown).
    worker_handles: Vec<std::thread::JoinHandle<()>>,
    /// Maximum number of tasks that can be in-flight simultaneously.
    budget: usize,
    /// Current number of in-flight tasks.
    in_flight: AtomicUsize,
    /// Shared reference to the voxel type registry (immutable, Arc'd).
    registry: Arc<VoxelTypeRegistry>,
}

impl MeshingPipeline {
    pub fn new(
        worker_count: usize,
        budget: usize,
        registry: Arc<VoxelTypeRegistry>,
    ) -> Self {
        let (task_tx, task_rx) = crossbeam_channel::bounded(budget);
        let (result_tx, result_rx) = crossbeam_channel::unbounded();
        let in_flight = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let rx = task_rx.clone();
            let tx = result_tx.clone();
            let reg = Arc::clone(&registry);
            let flight = Arc::clone(&in_flight);

            handles.push(std::thread::spawn(move || {
                while let Ok(task) = rx.recv() {
                    let visible = compute_visible_faces(
                        task.neighborhood.center(),
                        &task.neighborhood,
                        &reg,
                    );
                    let mesh = greedy_mesh(
                        task.neighborhood.center(),
                        &visible,
                        &task.neighborhood,
                        &reg,
                    );

                    let _ = tx.send(MeshingResult {
                        chunk_pos: task.chunk_pos,
                        mesh,
                        data_version: task.data_version,
                    });
                    flight.fetch_sub(1, Ordering::Relaxed);
                }
            }));
        }

        Self {
            task_sender: task_tx,
            result_receiver: result_rx,
            worker_handles: handles,
            budget,
            in_flight,
            registry,
        }
    }

    /// Submit a meshing task. Returns false if the budget is exhausted.
    pub fn submit(&self, task: MeshingTask) -> bool {
        if self.in_flight.load(Ordering::Relaxed) >= self.budget {
            return false;
        }
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        self.task_sender.send(task).is_ok()
    }

    /// Drain all completed results. Called once per frame on the main thread.
    pub fn drain_results(&self) -> Vec<MeshingResult> {
        let mut results = Vec::new();
        while let Ok(result) = self.result_receiver.try_recv() {
            results.push(result);
        }
        results
    }

    /// Number of tasks currently being processed by workers.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Shut down all worker threads gracefully.
    pub fn shutdown(self) {
        drop(self.task_sender); // closing the channel causes workers to exit
        for handle in self.worker_handles {
            let _ = handle.join();
        }
    }
}
```

### Task Cancellation

When a chunk is unloaded or remeshed before its previous task completes, the stale result is discarded. The `data_version` field enables this: when a result arrives, the main thread compares its `data_version` to the chunk's current version. If the versions don't match, the result is dropped. No explicit cancellation signal is needed — workers simply complete the stale task and the result is ignored.

For eager cancellation (e.g., the player teleported and all pending tasks are irrelevant), the task sender channel can be drained or the pipeline can be rebuilt.

### Budget

The `budget` limits the number of in-flight meshing tasks. This prevents the pipeline from queuing thousands of tasks during initial world load (which would consume memory for all the snapshots). A typical budget is 2-4x the worker count (e.g., 8 workers, budget of 32). The main thread prioritizes which chunks to submit based on distance to the camera.

## Outcome

The `nebula_meshing` crate exports `MeshingTask`, `MeshingResult`, and `MeshingPipeline`. The main thread creates neighborhood snapshots, submits `MeshingTask`s, and collects `MeshingResult`s each frame. Meshing never blocks the main thread. Worker count and budget are configurable. Running `cargo test -p nebula_meshing` passes all async meshing tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Meshing happens on background threads. The demo loads chunks faster because meshing does not block the main thread. New chunks "pop in" as their meshes complete asynchronously.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `crossbeam-channel` | `0.5` | Bounded/unbounded MPMC channels for task submission and result collection |

Worker threads are spawned with `std::thread::spawn`. No async runtime (tokio/async-std) is needed — the work is CPU-bound and channels provide all necessary coordination. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_registry() -> Arc<VoxelTypeRegistry> {
        let mut reg = VoxelTypeRegistry::new();
        reg.register(VoxelType::AIR, VoxelProperties { transparent: true, ..Default::default() });
        reg.register(VoxelType::STONE, VoxelProperties { transparent: false, ..Default::default() });
        Arc::new(reg)
    }

    /// A submitted meshing task should produce a valid mesh result.
    #[test]
    fn test_meshing_task_produces_valid_mesh() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(2, 8, registry);

        let mut chunk = ChunkVoxelData::new_filled(32, VoxelType::AIR);
        chunk.set(16, 16, 16, VoxelType::STONE);
        let neighborhood = ChunkNeighborhood::from_center_only(chunk);

        let task = MeshingTask {
            chunk_pos: ChunkPosition::ORIGIN,
            neighborhood,
            data_version: 1,
        };

        assert!(pipeline.submit(task));

        // Wait for result (with timeout)
        let start = std::time::Instant::now();
        loop {
            let results = pipeline.drain_results();
            if !results.is_empty() {
                assert_eq!(results[0].chunk_pos, ChunkPosition::ORIGIN);
                assert!(!results[0].mesh.is_empty());
                assert_eq!(results[0].data_version, 1);
                break;
            }
            assert!(start.elapsed().as_secs() < 5, "Timed out waiting for mesh result");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        pipeline.shutdown();
    }

    /// Multiple concurrent tasks should not interfere with each other.
    #[test]
    fn test_concurrent_tasks_do_not_interfere() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(4, 16, registry);

        let positions: Vec<ChunkPosition> = (0..8)
            .map(|i| ChunkPosition::new(i, 0, 0))
            .collect();

        for pos in &positions {
            let mut chunk = ChunkVoxelData::new_filled(32, VoxelType::STONE);
            let neighborhood = ChunkNeighborhood::from_center_only(chunk);
            let task = MeshingTask {
                chunk_pos: *pos,
                neighborhood,
                data_version: 1,
            };
            assert!(pipeline.submit(task));
        }

        // Collect all results
        let mut received = Vec::new();
        let start = std::time::Instant::now();
        while received.len() < 8 {
            received.extend(pipeline.drain_results());
            assert!(start.elapsed().as_secs() < 10, "Timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // All 8 positions should have results
        let mut received_positions: Vec<_> = received.iter().map(|r| r.chunk_pos).collect();
        received_positions.sort();
        let mut expected = positions.clone();
        expected.sort();
        assert_eq!(received_positions, expected);

        pipeline.shutdown();
    }

    /// Completed meshes should arrive via the result channel.
    #[test]
    fn test_completed_meshes_arrive_via_channel() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(1, 4, registry);

        let chunk = ChunkVoxelData::new_filled(32, VoxelType::AIR);
        let neighborhood = ChunkNeighborhood::from_center_only(chunk);
        let task = MeshingTask {
            chunk_pos: ChunkPosition::ORIGIN,
            neighborhood,
            data_version: 42,
        };

        pipeline.submit(task);

        let start = std::time::Instant::now();
        loop {
            let results = pipeline.drain_results();
            if !results.is_empty() {
                assert_eq!(results[0].data_version, 42);
                break;
            }
            assert!(start.elapsed().as_secs() < 5, "Timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        pipeline.shutdown();
    }

    /// Stale results (version mismatch) can be detected and discarded.
    #[test]
    fn test_task_cancellation_via_version_mismatch() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(1, 4, registry);

        let chunk = ChunkVoxelData::new_filled(32, VoxelType::STONE);
        let neighborhood = ChunkNeighborhood::from_center_only(chunk);
        let task = MeshingTask {
            chunk_pos: ChunkPosition::ORIGIN,
            neighborhood,
            data_version: 1, // old version
        };

        pipeline.submit(task);

        let current_version = 2u64; // chunk was edited after snapshot

        let start = std::time::Instant::now();
        loop {
            let results = pipeline.drain_results();
            if !results.is_empty() {
                // The result has version 1, but current is 2 — stale, discard
                assert_ne!(results[0].data_version, current_version);
                break;
            }
            assert!(start.elapsed().as_secs() < 5, "Timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        pipeline.shutdown();
    }

    /// The budget should prevent submitting more tasks than allowed.
    #[test]
    fn test_budget_limits_active_tasks() {
        let registry = test_registry();
        // Budget of 2 with 1 slow worker
        let pipeline = MeshingPipeline::new(1, 2, registry);

        let mut submitted = 0;
        for i in 0..10 {
            let chunk = ChunkVoxelData::new_filled(32, VoxelType::STONE);
            let neighborhood = ChunkNeighborhood::from_center_only(chunk);
            let task = MeshingTask {
                chunk_pos: ChunkPosition::new(i, 0, 0),
                neighborhood,
                data_version: 1,
            };
            if pipeline.submit(task) {
                submitted += 1;
            }
        }

        // Should not have been able to submit all 10
        assert!(submitted <= 2, "Budget should limit submissions, got {submitted}");

        pipeline.shutdown();
    }
}
```
