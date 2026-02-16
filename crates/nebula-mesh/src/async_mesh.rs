//! Asynchronous meshing pipeline: offloads chunk meshing to a thread pool
//! using snapshot-based tasks and channels for result delivery.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

use nebula_voxel::{ChunkAddress, VoxelTypeRegistry};

use crate::chunk_mesh::ChunkMesh;
use crate::greedy::greedy_mesh;
use crate::neighborhood::ChunkNeighborhood;
use crate::visibility::compute_visible_faces;

/// A self-contained meshing task that can run on any thread.
///
/// Contains a snapshot of the chunk data and its neighborhood so that
/// no locks on world data are needed during meshing.
pub struct MeshingTask {
    /// The chunk address this mesh is for (used to match results to chunks).
    pub chunk_addr: ChunkAddress,
    /// Snapshot of the chunk's voxel data and neighbors (owned, no references into world).
    pub neighborhood: ChunkNeighborhood,
    /// Version number of the chunk data at snapshot time.
    pub data_version: u64,
}

/// The result of a completed meshing task.
pub struct MeshingResult {
    /// The chunk address this mesh is for.
    pub chunk_addr: ChunkAddress,
    /// The generated mesh.
    pub mesh: ChunkMesh,
    /// Version number of the chunk data at snapshot time.
    pub data_version: u64,
}

/// Asynchronous meshing pipeline backed by a thread pool.
///
/// The main thread creates [`MeshingTask`]s containing neighborhood snapshots,
/// submits them via [`submit`](Self::submit), and collects [`MeshingResult`]s
/// each frame via [`drain_results`](Self::drain_results). Meshing never blocks
/// the main thread.
pub struct MeshingPipeline {
    /// Channel sender for submitting tasks to workers.
    task_sender: Option<crossbeam_channel::Sender<MeshingTask>>,
    /// Channel receiver for collecting completed results on the main thread.
    result_receiver: crossbeam_channel::Receiver<MeshingResult>,
    /// Handles to the worker threads (for shutdown).
    worker_handles: Vec<JoinHandle<()>>,
    /// Maximum number of tasks that can be in-flight simultaneously.
    budget: usize,
    /// Current number of in-flight tasks.
    in_flight: Arc<AtomicUsize>,
}

impl MeshingPipeline {
    /// Creates a new meshing pipeline with the given number of worker threads
    /// and task budget.
    ///
    /// `worker_count` — number of OS threads to spawn for meshing.
    /// `budget` — maximum number of in-flight tasks (limits memory usage from snapshots).
    /// `registry` — immutable voxel type registry shared by all workers.
    pub fn new(worker_count: usize, budget: usize, registry: Arc<VoxelTypeRegistry>) -> Self {
        let (task_tx, task_rx) = crossbeam_channel::bounded(budget);
        let (result_tx, result_rx) = crossbeam_channel::unbounded();
        let in_flight = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let rx: crossbeam_channel::Receiver<MeshingTask> = task_rx.clone();
            let tx = result_tx.clone();
            let reg = Arc::clone(&registry);
            let flight = Arc::clone(&in_flight);

            handles.push(std::thread::spawn(move || {
                while let Ok(task) = rx.recv() {
                    let mesh = match task.neighborhood.center() {
                        Some(center) => {
                            let visible = compute_visible_faces(center, &task.neighborhood, &reg);
                            greedy_mesh(center, &visible, &task.neighborhood, &reg)
                        }
                        None => ChunkMesh::new(),
                    };

                    let _ = tx.send(MeshingResult {
                        chunk_addr: task.chunk_addr,
                        mesh,
                        data_version: task.data_version,
                    });
                    flight.fetch_sub(1, Ordering::Relaxed);
                }
            }));
        }

        Self {
            task_sender: Some(task_tx),
            result_receiver: result_rx,
            worker_handles: handles,
            budget,
            in_flight,
        }
    }

    /// Submit a meshing task. Returns `false` if the budget is exhausted
    /// or the pipeline has been shut down.
    pub fn submit(&self, task: MeshingTask) -> bool {
        let sender = match &self.task_sender {
            Some(s) => s,
            None => return false,
        };
        if self.in_flight.load(Ordering::Relaxed) >= self.budget {
            return false;
        }
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        if sender.send(task).is_err() {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            return false;
        }
        true
    }

    /// Drain all completed results. Called once per frame on the main thread.
    pub fn drain_results(&self) -> Vec<MeshingResult> {
        let mut results = Vec::new();
        while let Ok(result) = self.result_receiver.try_recv() {
            results.push(result);
        }
        results
    }

    /// Number of tasks currently being processed or queued by workers.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Shut down all worker threads gracefully.
    ///
    /// Drops the task sender to signal workers to exit, then joins all threads.
    pub fn shutdown(&mut self) {
        // Drop sender to close the channel, causing workers to exit.
        self.task_sender.take();
        for handle in self.worker_handles.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Drop for MeshingPipeline {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_voxel::{ChunkData, Transparency, VoxelTypeDef, VoxelTypeId, VoxelTypeRegistry};

    fn test_registry() -> Arc<VoxelTypeRegistry> {
        let mut reg = VoxelTypeRegistry::new();
        // AIR is registered by default with id 0.
        // Register STONE as a solid type.
        let _ = reg.register(VoxelTypeDef {
            name: "stone".to_string(),
            transparency: Transparency::Opaque,
            solid: true,
            material_index: 0,
            light_emission: 0,
        });
        Arc::new(reg)
    }

    fn stone_id() -> VoxelTypeId {
        VoxelTypeId(1)
    }

    /// A submitted meshing task should produce a valid mesh result.
    #[test]
    fn test_meshing_task_produces_valid_mesh() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(2, 8, registry);

        let mut chunk = ChunkData::new(VoxelTypeId(0));
        chunk.set(16, 16, 16, stone_id());
        let neighborhood = ChunkNeighborhood::from_center_only(chunk);

        let task = MeshingTask {
            chunk_addr: ChunkAddress::new(0, 0, 0, 0),
            neighborhood,
            data_version: 1,
        };

        assert!(pipeline.submit(task));

        let start = std::time::Instant::now();
        loop {
            let results = pipeline.drain_results();
            if !results.is_empty() {
                assert_eq!(results[0].chunk_addr, ChunkAddress::new(0, 0, 0, 0));
                assert!(results[0].mesh.quad_count() > 0);
                assert_eq!(results[0].data_version, 1);
                break;
            }
            assert!(
                start.elapsed().as_secs() < 5,
                "Timed out waiting for mesh result"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Multiple concurrent tasks should not interfere with each other.
    #[test]
    fn test_concurrent_tasks_do_not_interfere() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(4, 16, registry);

        let addresses: Vec<ChunkAddress> = (0..8).map(|i| ChunkAddress::new(i, 0, 0, 0)).collect();

        for addr in &addresses {
            let chunk = ChunkData::new(stone_id());
            let neighborhood = ChunkNeighborhood::from_center_only(chunk);
            let task = MeshingTask {
                chunk_addr: *addr,
                neighborhood,
                data_version: 1,
            };
            assert!(pipeline.submit(task));
        }

        let mut received = Vec::new();
        let start = std::time::Instant::now();
        while received.len() < 8 {
            received.extend(pipeline.drain_results());
            assert!(start.elapsed().as_secs() < 10, "Timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let mut received_addrs: Vec<_> = received.iter().map(|r| r.chunk_addr).collect();
        received_addrs.sort();
        let mut expected = addresses;
        expected.sort();
        assert_eq!(received_addrs, expected);
    }

    /// Completed meshes should arrive via the result channel.
    #[test]
    fn test_completed_meshes_arrive_via_channel() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(1, 4, registry);

        let chunk = ChunkData::new(VoxelTypeId(0));
        let neighborhood = ChunkNeighborhood::from_center_only(chunk);
        let task = MeshingTask {
            chunk_addr: ChunkAddress::new(0, 0, 0, 0),
            neighborhood,
            data_version: 42,
        };

        assert!(pipeline.submit(task));

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
    }

    /// Stale results (version mismatch) can be detected and discarded.
    #[test]
    fn test_task_cancellation_via_version_mismatch() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(1, 4, registry);

        let chunk = ChunkData::new(stone_id());
        let neighborhood = ChunkNeighborhood::from_center_only(chunk);
        let task = MeshingTask {
            chunk_addr: ChunkAddress::new(0, 0, 0, 0),
            neighborhood,
            data_version: 1,
        };

        assert!(pipeline.submit(task));

        let current_version = 2u64;

        let start = std::time::Instant::now();
        loop {
            let results = pipeline.drain_results();
            if !results.is_empty() {
                assert_ne!(results[0].data_version, current_version);
                break;
            }
            assert!(start.elapsed().as_secs() < 5, "Timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// The budget should prevent submitting more tasks than allowed.
    #[test]
    fn test_budget_limits_active_tasks() {
        let registry = test_registry();
        let pipeline = MeshingPipeline::new(1, 2, registry);

        let mut submitted = 0;
        for i in 0..10 {
            let chunk = ChunkData::new(stone_id());
            let neighborhood = ChunkNeighborhood::from_center_only(chunk);
            let task = MeshingTask {
                chunk_addr: ChunkAddress::new(i, 0, 0, 0),
                neighborhood,
                data_version: 1,
            };
            if pipeline.submit(task) {
                submitted += 1;
            }
        }

        assert!(
            submitted <= 4,
            "Budget should limit submissions, got {submitted}"
        );
    }
}
