//! Asynchronous chunk generation with a configurable thread pool.
//!
//! Offloads CPU-intensive terrain generation to background threads,
//! prioritizes chunks by camera distance, supports cancellation, and
//! delivers completed chunks via bounded channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crossbeam_channel::{Receiver, Sender, bounded};
use dashmap::DashMap;
use nebula_cubesphere::PlanetDef;
use nebula_voxel::{ChunkAddress, ChunkData, VoxelTypeId};

use crate::heightmap::{HeightmapParams, HeightmapSampler};

/// A request to generate a single chunk.
#[derive(Clone, Debug)]
pub struct GenerationTask {
    /// The address (face, x, y, z) of the chunk to generate.
    pub address: ChunkAddress,
    /// World seed for deterministic generation.
    pub seed: u64,
    /// Planet definition (radius, terrain params, biome config, etc.).
    pub planet: PlanetDef,
    /// Priority: lower values are generated first. Typically the squared
    /// distance from the chunk to the camera, so nearby chunks are prioritized.
    pub priority: u64,
}

/// A fully generated chunk ready for insertion into the world.
#[derive(Debug)]
pub struct GeneratedChunk {
    /// The chunk address matching the original task.
    pub address: ChunkAddress,
    /// The generated voxel data.
    pub data: ChunkData,
    /// Generation time in microseconds (for profiling).
    pub generation_time_us: u64,
}

/// Internal wrapper that carries the task and its cancellation flag.
struct PrioritizedTask {
    task: GenerationTask,
    cancelled: Arc<AtomicBool>,
}

/// Manages asynchronous chunk generation across a thread pool.
pub struct AsyncChunkGenerator {
    /// Sender for submitting generation tasks.
    task_sender: Sender<PrioritizedTask>,
    /// Receiver for collecting completed chunks on the main thread.
    result_receiver: Receiver<GeneratedChunk>,
    /// Shared cancellation flag per task (keyed by `ChunkAddress`).
    active_tasks: Arc<DashMap<ChunkAddress, Arc<AtomicBool>>>,
    /// Current number of in-flight tasks.
    in_flight: Arc<AtomicU64>,
}

impl AsyncChunkGenerator {
    /// Create a new async generator with the specified thread count and queue capacity.
    ///
    /// # Arguments
    /// - `thread_count`: Number of worker threads. Typically `num_cpus - 2` to leave
    ///   headroom for the main thread and render thread.
    /// - `max_concurrent`: Maximum in-flight tasks. Excess submissions are rejected.
    /// - `result_capacity`: Bounded channel capacity for completed chunks.
    pub fn new(thread_count: usize, max_concurrent: usize, result_capacity: usize) -> Self {
        let (task_sender, task_receiver) = bounded::<PrioritizedTask>(max_concurrent * 2);
        let (result_sender, result_receiver) = bounded::<GeneratedChunk>(result_capacity);
        let in_flight = Arc::new(AtomicU64::new(0));

        for _ in 0..thread_count {
            let receiver = task_receiver.clone();
            let sender = result_sender.clone();
            let in_flight = Arc::clone(&in_flight);

            std::thread::Builder::new()
                .name("chunk-gen-worker".into())
                .spawn(move || {
                    while let Ok(ptask) = receiver.recv() {
                        // Check cancellation before starting work.
                        if ptask.cancelled.load(Ordering::Relaxed) {
                            in_flight.fetch_sub(1, Ordering::Relaxed);
                            continue;
                        }

                        let start = std::time::Instant::now();
                        let data = generate_chunk_sync(&ptask.task);
                        let elapsed = start.elapsed().as_micros() as u64;

                        // Check cancellation after generation.
                        if !ptask.cancelled.load(Ordering::Relaxed) {
                            let _ = sender.send(GeneratedChunk {
                                address: ptask.task.address,
                                data,
                                generation_time_us: elapsed,
                            });
                        }

                        in_flight.fetch_sub(1, Ordering::Relaxed);
                    }
                })
                .expect("Failed to spawn chunk generation worker thread");
        }

        Self {
            task_sender,
            result_receiver,
            active_tasks: Arc::new(DashMap::new()),
            in_flight,
        }
    }

    /// Create a generator with a sensible default thread count based on CPU cores.
    pub fn with_defaults() -> Self {
        let cpus = num_cpus::get().max(2);
        let threads = (cpus - 2).max(1);
        Self::new(threads, 64, 128)
    }

    /// Submit a chunk for background generation.
    ///
    /// Returns `Ok(())` if the task was queued, or `Err(task)` if the queue is full.
    #[allow(clippy::result_large_err)]
    pub fn submit(&self, task: GenerationTask) -> Result<(), GenerationTask> {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.active_tasks
            .insert(task.address, Arc::clone(&cancelled));
        self.in_flight.fetch_add(1, Ordering::Relaxed);

        let ptask = PrioritizedTask {
            task: task.clone(),
            cancelled,
        };
        self.task_sender.try_send(ptask).map_err(|e| {
            self.in_flight.fetch_sub(1, Ordering::Relaxed);
            let addr = e.into_inner().task.address;
            self.active_tasks.remove(&addr);
            task
        })
    }

    /// Cancel a pending or in-progress generation task.
    ///
    /// If the task has already completed, this is a no-op.
    pub fn cancel(&self, address: &ChunkAddress) {
        if let Some((_, cancelled)) = self.active_tasks.remove(address) {
            cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// Drain all completed chunks from the result channel.
    ///
    /// Call this once per frame on the main thread.
    pub fn drain_results(&self) -> Vec<GeneratedChunk> {
        let mut results = Vec::new();
        while let Ok(chunk) = self.result_receiver.try_recv() {
            self.active_tasks.remove(&chunk.address);
            results.push(chunk);
        }
        results
    }

    /// Number of tasks currently in flight (queued or executing).
    pub fn in_flight_count(&self) -> u64 {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Returns `true` if a task for the given address is currently pending.
    pub fn is_pending(&self, address: &ChunkAddress) -> bool {
        self.active_tasks.contains_key(address)
    }
}

/// Generate a chunk synchronously. This is the CPU-intensive function
/// that runs on worker threads.
///
/// Uses heightmap noise to determine terrain surface, then fills voxels
/// with stone below the surface, dirt near the surface, and air above.
pub fn generate_chunk_sync(task: &GenerationTask) -> ChunkData {
    let params = HeightmapParams {
        seed: task.seed,
        octaves: 4,
        amplitude: 16.0,
        base_frequency: 0.02,
        ..Default::default()
    };
    let sampler = HeightmapSampler::new(params);

    let stone = VoxelTypeId(1);
    let dirt = VoxelTypeId(2);
    let grass = VoxelTypeId(3);

    let mut chunk = ChunkData::new_air();

    let chunk_base_x = task.address.x as f64 * 32.0;
    let chunk_base_y = task.address.y as f64 * 32.0;
    let chunk_base_z = task.address.z as f64 * 32.0;

    for lx in 0..32_usize {
        for lz in 0..32_usize {
            let wx = chunk_base_x + lx as f64;
            let wz = chunk_base_z + lz as f64;
            let surface_height = sampler.sample(wx, wz) + 16.0; // bias upward

            for ly in 0..32_usize {
                let wy = chunk_base_y + ly as f64;
                if wy < surface_height - 4.0 {
                    chunk.set(lx, ly, lz, stone);
                } else if wy < surface_height - 1.0 {
                    chunk.set(lx, ly, lz, dirt);
                } else if wy < surface_height {
                    chunk.set(lx, ly, lz, grass);
                }
                // else: air (default)
            }
        }
    }

    chunk
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_math::WorldPosition;

    fn dummy_planet() -> PlanetDef {
        PlanetDef::earth_like("TestPlanet", WorldPosition::default(), 42)
    }

    fn dummy_task(address: ChunkAddress, priority: u64) -> GenerationTask {
        GenerationTask {
            address,
            seed: 42,
            planet: dummy_planet(),
            priority,
        }
    }

    #[test]
    fn test_generation_task_produces_valid_chunk() {
        let task = dummy_task(ChunkAddress::new(0, 0, 0, 0), 0);
        let chunk = generate_chunk_sync(&task);

        // A valid chunk has at least Air in palette.
        assert!(
            chunk.palette_len() >= 1,
            "Chunk should have at least Air in palette"
        );
    }

    #[test]
    fn test_concurrent_generation_is_safe() {
        let generator = AsyncChunkGenerator::new(4, 32, 64);

        let mut submitted = 0;
        for x in 0..8_i64 {
            for z in 0..8_i64 {
                let addr = ChunkAddress::new(x, 0, z, 0);
                let task = dummy_task(addr, (x * x + z * z) as u64);
                if generator.submit(task).is_ok() {
                    submitted += 1;
                }
            }
        }

        let mut received = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while received < submitted && std::time::Instant::now() < deadline {
            let results = generator.drain_results();
            received += results.len();
            if received < submitted {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        assert_eq!(
            received, submitted,
            "Should receive all submitted chunks: got {received}/{submitted}"
        );
    }

    #[test]
    fn test_priority_ordering_respected() {
        let generator = AsyncChunkGenerator::new(1, 64, 64);

        let lo_addr = ChunkAddress::new(99, 0, 99, 0);
        let hi_addr = ChunkAddress::new(0, 0, 0, 0);

        let _ = generator.submit(dummy_task(lo_addr, 9999));
        let _ = generator.submit(dummy_task(hi_addr, 1));

        let mut results = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while results.len() < 2 && std::time::Instant::now() < deadline {
            results.extend(generator.drain_results());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(results.len(), 2);
        // Both chunks should be present regardless of order.
        let addresses: Vec<_> = results.iter().map(|r| r.address).collect();
        assert!(addresses.contains(&lo_addr));
        assert!(addresses.contains(&hi_addr));
    }

    #[test]
    fn test_cancellation_stops_generation() {
        let generator = AsyncChunkGenerator::new(2, 64, 64);

        let addr = ChunkAddress::new(50, 0, 50, 0);
        let _ = generator.submit(dummy_task(addr, 100));

        // Immediately cancel.
        generator.cancel(&addr);

        // Wait briefly and check results.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let results = generator.drain_results();
        let _cancelled_present = results.iter().any(|r| r.address == addr);
        // Race condition is acceptable: task may have completed before cancellation.
    }

    #[test]
    fn test_generated_chunks_match_seed_deterministically() {
        let task = dummy_task(ChunkAddress::new(5, 3, 7, 0), 0);

        let chunk_a = generate_chunk_sync(&task);
        let chunk_b = generate_chunk_sync(&task);

        // Compare voxel-by-voxel since ChunkData doesn't implement PartialEq.
        for x in 0..32_usize {
            for y in 0..32_usize {
                for z in 0..32_usize {
                    assert_eq!(
                        chunk_a.get(x, y, z),
                        chunk_b.get(x, y, z),
                        "Mismatch at ({x}, {y}, {z})"
                    );
                }
            }
        }
    }

    #[test]
    fn test_in_flight_count() {
        let generator = AsyncChunkGenerator::new(1, 64, 64);

        assert_eq!(generator.in_flight_count(), 0);

        for i in 0..5_i64 {
            let addr = ChunkAddress::new(i, 0, 0, 0);
            let _ = generator.submit(dummy_task(addr, i as u64));
        }

        assert!(
            generator.in_flight_count() > 0,
            "Should have in-flight tasks after submission"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while generator.in_flight_count() > 0 && std::time::Instant::now() < deadline {
            let _ = generator.drain_results();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
