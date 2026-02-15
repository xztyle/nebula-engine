# Work-Stealing Thread Pools

## Problem

Rayon's default global thread pool is convenient but unsuitable for an engine with strict thread allocation requirements. The default pool claims all available CPU cores, leaving nothing for the main game thread or the tokio async runtime. Thread names default to anonymous identifiers, making profiling tools like Tracy or Perf useless for diagnosing which pool a bottleneck belongs to. The default stack size may be insufficient for deep recursion in terrain generation (multi-octave noise chains) or excessive for simple meshing tasks, wasting virtual address space.

Additionally, rayon's unscoped `spawn` does not guarantee that work completes before a reference becomes invalid, so frame-local work that borrows data from the current frame needs scoped parallelism with lifetime guarantees.

The engine needs a carefully configured rayon pool with explicit thread count, named threads, appropriate stack size, scoped parallelism support, and utilization tracking for the profiling overlay.

## Solution

### Pool Configuration

Create a custom rayon `ThreadPool` at engine startup, separate from rayon's global pool:

```rust
use rayon::ThreadPool;

pub struct WorkerPool {
    pool: ThreadPool,
    thread_count: usize,
    utilization: Arc<PoolUtilization>,
}

impl WorkerPool {
    pub fn new(config: &WorkerPoolConfig) -> Self {
        let thread_count = config.thread_count();
        let utilization = Arc::new(PoolUtilization::new(thread_count));
        let util_clone = utilization.clone();

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(thread_count)
            .thread_name(|index| format!("nebula-worker-{}", index))
            .stack_size(config.stack_size_bytes)
            .panic_handler(move |panic_info| {
                log::error!(
                    "Worker thread panicked: {:?}",
                    panic_info.downcast_ref::<&str>()
                );
            })
            .start_handler({
                let util = util_clone.clone();
                move |index| {
                    log::debug!("Worker thread {} started", index);
                    util.mark_idle(index);
                }
            })
            .build()
            .expect("failed to build rayon thread pool");

        Self {
            pool,
            thread_count,
            utilization,
        }
    }
}
```

### Thread Count Calculation

The engine reserves cores for the main thread and tokio, giving the rest to rayon:

```rust
pub struct WorkerPoolConfig {
    /// Override thread count. If None, auto-detect.
    pub thread_count_override: Option<usize>,
    /// Cores reserved for main thread + tokio. Default: 2.
    pub reserved_cores: usize,
    /// Stack size per worker thread in bytes. Default: 2MB.
    pub stack_size_bytes: usize,
}

impl WorkerPoolConfig {
    pub fn thread_count(&self) -> usize {
        if let Some(count) = self.thread_count_override {
            return count.max(1);
        }

        let available = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        // Reserve cores for main thread and tokio runtime.
        // Minimum 1 worker thread.
        (available.saturating_sub(self.reserved_cores)).max(1)
    }
}

impl Default for WorkerPoolConfig {
    fn default() -> Self {
        Self {
            thread_count_override: None,
            reserved_cores: 2,
            stack_size_bytes: 2 * 1024 * 1024, // 2 MB
        }
    }
}
```

On a typical 8-core machine: 8 - 2 = 6 rayon worker threads, leaving 1 core for the main thread and 1 core for tokio (which uses 2 threads but they are mostly idle, waiting on I/O).

On a 4-core machine: 4 - 2 = 2 rayon worker threads. Still functional but with lower throughput for meshing and terrain generation.

### Named Threads

Every rayon thread is named `"nebula-worker-{index}"` (e.g., `nebula-worker-0`, `nebula-worker-1`). This makes threads identifiable in:

- **Tracy** profiler thread lanes
- **htop** / `ps -T` output
- **Log messages** that include thread name via the `log` crate
- **Panic messages** that show which worker panicked

### Stack Size

The stack size is set to 2MB per thread (the Linux default is 8MB, which is excessive for most engine work). 2MB is sufficient for:

- Greedy meshing with moderate recursion depth
- Multi-octave noise evaluation (6-8 octaves, no stack-heavy recursion)
- LOD tree traversal (quadtree depth ~20)

If a subsystem needs deep recursion, it should use `stacker` or iterative algorithms instead of relying on large stack allocations.

### Scoped Parallelism

Rayon's `scope` function provides frame-local parallelism where spawned work is guaranteed to complete before the scope exits. This is essential for work that borrows data from the current frame's ECS resources:

```rust
impl WorkerPool {
    /// Execute scoped parallel work. All tasks spawned within the scope
    /// are guaranteed to complete before this function returns.
    pub fn scope<'scope, F, R>(&self, f: F) -> R
    where
        F: FnOnce(&rayon::Scope<'scope>) -> R + Send,
        R: Send,
    {
        self.pool.scope(|s| f(s))
    }

    /// Parallel iteration over a slice using the engine's pool.
    pub fn par_iter<'data, T: Sync + 'data>(
        &self,
        data: &'data [T],
    ) -> rayon::slice::Iter<'data, T> {
        self.pool.install(|| data.par_iter())
    }
}
```

Usage for frame-local work:

```rust
let chunks_to_mesh: Vec<&ChunkData> = gather_visible_dirty_chunks();

worker_pool.scope(|s| {
    for chunk in &chunks_to_mesh {
        s.spawn(|_| {
            let mesh = greedy_mesh(chunk);
            mesh_sender.send(mesh).ok();
        });
    }
});
// All meshing work is guaranteed complete here.
```

### Utilization Tracking

The pool tracks how busy each thread is for the profiling debug overlay:

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub struct PoolUtilization {
    /// Per-thread busy time in microseconds (rolling window).
    busy_us: Vec<AtomicU64>,
    /// Per-thread total time in microseconds (rolling window).
    total_us: Vec<AtomicU64>,
    thread_count: usize,
}

impl PoolUtilization {
    pub fn new(thread_count: usize) -> Self {
        Self {
            busy_us: (0..thread_count).map(|_| AtomicU64::new(0)).collect(),
            total_us: (0..thread_count).map(|_| AtomicU64::new(0)).collect(),
            thread_count,
        }
    }

    pub fn mark_idle(&self, _thread_index: usize) {}

    /// Returns utilization as a fraction [0.0, 1.0] per thread.
    pub fn per_thread_utilization(&self) -> Vec<f64> {
        (0..self.thread_count)
            .map(|i| {
                let busy = self.busy_us[i].load(Ordering::Relaxed) as f64;
                let total = self.total_us[i].load(Ordering::Relaxed) as f64;
                if total > 0.0 { busy / total } else { 0.0 }
            })
            .collect()
    }

    /// Returns average utilization across all threads.
    pub fn average_utilization(&self) -> f64 {
        let per_thread = self.per_thread_utilization();
        if per_thread.is_empty() {
            return 0.0;
        }
        per_thread.iter().sum::<f64>() / per_thread.len() as f64
    }
}
```

The debug overlay (from `nebula-debug`) reads `average_utilization()` each frame to display a percentage showing how much compute capacity the engine is using.

### Configuration

```toml
[threading.worker_pool]
# thread_count = 6    # Uncomment to override auto-detection
reserved_cores = 2
stack_size_bytes = 2097152  # 2 MB
```

## Outcome

A `WorkerPool` struct in the `nebula-threading` crate that wraps a custom rayon `ThreadPool` with engine-appropriate configuration: thread count set to CPU cores minus 2 (reserving capacity for the main thread and tokio), threads named `"nebula-worker-{n}"` for profiling, 2MB stack size, panic handling that logs instead of aborting, scoped parallelism for frame-local work, and per-thread utilization metrics exposed for the debug overlay.

## Demo Integration

**Demo crate:** `nebula-demo`

Worker threads steal tasks from each other when idle. CPU utilization approaches 94% on all cores during heavy chunk loading sequences.

## Crates & Dependencies

- **`rayon`** = `"1.10"` -- work-stealing thread pool with scoped parallelism and custom configuration
- **`log`** = `"0.4"` -- logging thread lifecycle and panic events
- Rust edition **2024**

## Unit Tests

- **`test_thread_count_matches_config`** -- Create a `WorkerPool` with `thread_count_override = Some(4)`. Spawn a task that calls `rayon::current_num_threads()` from within the pool. Assert the result is 4.

- **`test_thread_names_are_set`** -- Create a `WorkerPool` with 3 threads. Spawn 3 tasks that each record `std::thread::current().name()`. Collect the names and assert they are `"nebula-worker-0"`, `"nebula-worker-1"`, and `"nebula-worker-2"` (order may vary, but all three names must be present).

- **`test_scoped_work_completes_within_scope`** -- Create an `AtomicU32` counter initialized to 0. Call `worker_pool.scope(|s| { ... })` and spawn 10 tasks that each increment the counter. After `scope` returns, assert the counter equals 10. This verifies that all scoped work completes before the scope exits.

- **`test_utilization_metrics_available`** -- Create a `WorkerPool` with 2 threads. Call `per_thread_utilization()` and assert it returns a `Vec` with exactly 2 elements. Assert each element is in the range `[0.0, 1.0]`. Call `average_utilization()` and assert it returns a value in `[0.0, 1.0]`.

- **`test_pool_handles_task_panic_gracefully`** -- Spawn a task in the pool that calls `panic!("test panic")`. Assert the pool remains functional by successfully spawning and completing a subsequent non-panicking task. Assert the panic was caught by the custom panic handler (verify via a log capture or an `AtomicBool` set in the panic handler).

- **`test_auto_thread_count_reserves_cores`** -- Create a `WorkerPoolConfig` with `reserved_cores = 2` and no override. Call `thread_count()`. Assert the result equals `available_parallelism() - 2` (or 1, whichever is larger). This test may be skipped on single-core CI machines.

- **`test_minimum_one_thread`** -- Create a `WorkerPoolConfig` with `reserved_cores = 100` (more than any machine has). Call `thread_count()`. Assert the result is 1 (the minimum).

- **`test_scoped_parallelism_borrows_data`** -- Create a local `Vec<u32>` on the stack. Use `worker_pool.scope` to spawn tasks that read from the Vec (proving the borrow is valid within the scope). Assert the tasks read the correct values. This test validates that rayon's lifetime guarantees work with borrowed data.
