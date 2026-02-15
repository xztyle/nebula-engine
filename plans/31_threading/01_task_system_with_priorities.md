# Task System with Priorities

## Problem

A voxel-based planetary engine generates enormous amounts of CPU-bound work every frame: greedy meshing of chunks, procedural terrain generation, physics broadphase preparation, LOD recalculation, and ambient occlusion computation. If this work runs on the main thread, the frame rate collapses. If it runs on worker threads without prioritization, critical work (the mesh the player is staring at) competes with speculative work (a chunk 500 meters away), causing visible pop-in and hitching. The engine needs a task system that understands urgency: some work must finish this frame to avoid visual artifacts, some work should finish within a few frames to maintain smoothness, and some work can trickle in whenever cores are idle.

Without a per-frame execution budget, an explosion of terrain generation requests during rapid camera movement can monopolize all worker threads for hundreds of milliseconds, starving the main thread of results and creating a stall spiral where the frame waits for tasks that themselves generate more tasks.

## Solution

### Priority Levels

Define four priority tiers, each with clear semantics:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskPriority {
    /// Must complete before the current frame ends.
    /// Examples: re-mesh a chunk the player is standing inside,
    /// physics collision recalculation after teleport.
    Critical = 3,

    /// Should complete within the next 2-3 frames.
    /// Examples: mesh chunks entering the near view frustum,
    /// terrain generation for adjacent sectors.
    High = 2,

    /// Background work with no urgency.
    /// Examples: terrain generation for distant chunks,
    /// LOD downgrades for far-away geometry.
    Normal = 1,

    /// Only execute when all other queues are empty and cores are idle.
    /// Examples: pre-caching speculative chunks, analytics aggregation,
    /// optional AO refinement passes.
    Low = 0,
}
```

### Task Descriptor

Each submitted task carries metadata alongside its closure:

```rust
pub struct TaskDescriptor {
    pub priority: TaskPriority,
    pub label: &'static str,
    pub frame_tag: u64,
    pub cancellation_token: CancellationToken,
}

pub struct Task {
    pub descriptor: TaskDescriptor,
    pub work: Box<dyn FnOnce() -> Box<dyn std::any::Any + Send> + Send>,
}
```

The `frame_tag` allows the frame sync system (story 03) to identify which tasks belong to the current frame. The `label` is used for profiling and debug overlays.

### Priority Queue

A thread-safe priority queue backed by `crossbeam` 0.8 concurrent data structures feeds tasks into the rayon pool. Tasks are dequeued in strict priority order: all Critical tasks before any High, all High before any Normal, and so on.

```rust
use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};

pub struct TaskQueue {
    heap: Arc<Mutex<BinaryHeap<PrioritizedTask>>>,
}

impl TaskQueue {
    pub fn push(&self, task: Task) {
        let mut heap = self.heap.lock().expect("task queue lock poisoned");
        heap.push(PrioritizedTask {
            priority: task.descriptor.priority,
            sequence: next_sequence(),
            task,
        });
    }

    pub fn pop(&self) -> Option<Task> {
        let mut heap = self.heap.lock().expect("task queue lock poisoned");
        heap.pop().map(|pt| pt.task)
    }
}
```

Within the same priority level, tasks execute in FIFO order (ensured by a monotonic sequence number).

### Rayon Work-Stealing Execution

The task system submits work to a `rayon` 1.10 thread pool. When the queue is drained into the pool, rayon's work-stealing scheduler distributes tasks across all available cores, preventing any single core from becoming a bottleneck.

```rust
use rayon::ThreadPool;

pub struct TaskSystem {
    pool: ThreadPool,
    queue: TaskQueue,
    budget_ms: f64,
}

impl TaskSystem {
    pub fn dispatch_frame(&self, frame_tag: u64) {
        let start = std::time::Instant::now();

        while let Some(task) = self.queue.pop() {
            // Check budget: if we have exceeded the per-frame time limit
            // and no more Critical tasks remain, stop dispatching.
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            if elapsed_ms > self.budget_ms
                && task.descriptor.priority != TaskPriority::Critical
            {
                // Re-enqueue the task we just popped and stop.
                self.queue.push(task);
                break;
            }

            let token = task.descriptor.cancellation_token.clone();
            let work = task.work;

            self.pool.spawn(move || {
                if !token.is_cancelled() {
                    let _result = work();
                }
            });
        }
    }
}
```

### Per-Frame Budget

The engine configures a per-frame dispatch budget in milliseconds (default: 2ms). During `dispatch_frame`, the system continuously pops tasks from the priority queue and submits them to rayon. If the budget is exceeded and no Critical tasks remain, dispatching stops. Critical tasks always dispatch regardless of budget, ensuring frame-critical work never starves.

The budget does not limit how long tasks run once dispatched -- it limits how much time the main thread spends submitting tasks. Tasks already in-flight on worker threads continue independently.

### Completion Callbacks

Each task can register an optional completion callback that fires on the main thread when the task's result is ready. Results are collected through a crossbeam channel (see story 02) and drained each frame:

```rust
pub struct TaskHandle<T> {
    receiver: crossbeam::channel::Receiver<T>,
}

impl<T> TaskHandle<T> {
    pub fn try_recv(&self) -> Option<T> {
        self.receiver.try_recv().ok()
    }

    pub fn is_complete(&self) -> bool {
        !self.receiver.is_empty()
    }
}
```

## Outcome

A `TaskSystem` struct in the `nebula-threading` crate that accepts prioritized tasks, dispatches them to a rayon work-stealing thread pool, respects a per-frame time budget, and delivers results through typed handles. The system guarantees that Critical tasks are always dispatched immediately, High tasks dispatch before Normal tasks, and Low tasks only execute when cores are idle. Debug labels on every task enable profiling integration with the `nebula-debug` crate.

## Demo Integration

**Demo crate:** `nebula-demo`

A task scheduler distributes terrain generation and meshing across CPU cores. The profiler flame graph shows multiple concurrent worker threads processing tasks.

## Crates & Dependencies

- **`rayon`** = `"1.10"` -- work-stealing thread pool for parallel CPU-bound execution
- **`crossbeam`** = `"0.8"` -- concurrent data structures and channels for result delivery
- **`log`** = `"0.4"` -- logging budget overruns and task lifecycle events
- Rust edition **2024**

## Unit Tests

- **`test_critical_tasks_complete_before_normal`** -- Submit 100 Normal tasks and 10 Critical tasks. Attach sequence-stamped completion markers. Assert that all 10 Critical tasks complete before any Normal task begins execution. Use an `AtomicU64` counter incremented by each task to verify ordering.

- **`test_work_stealing_distributes_evenly`** -- Submit 1000 trivial tasks (each records which thread executed it via `rayon::current_thread_index()`). After all complete, verify that no single thread handled more than 60% of the total work, confirming that rayon's work-stealing is active and distributing load.

- **`test_budget_limits_dispatch_count`** -- Set the per-frame budget to 0.1ms. Submit 10,000 Normal tasks. Call `dispatch_frame` and measure how many tasks were actually submitted to the pool. Assert that the count is significantly less than 10,000 (the budget cut off dispatching). Verify that un-dispatched tasks remain in the queue for the next frame.

- **`test_task_completion_callback_fires`** -- Submit a task that returns a specific value (e.g., `42u64`). Poll the `TaskHandle` in a loop with a timeout. Assert that `try_recv()` eventually returns `Some(42)`. Assert that `is_complete()` returns `true` once the result is available.

- **`test_pool_uses_all_available_cores`** -- Create a task system with a pool sized to 4 threads. Submit 4 tasks that each sleep for 50ms and record their start time. Assert that all 4 start times are within 5ms of each other, proving all 4 threads were utilized concurrently rather than serialized.

- **`test_priority_ordering_fifo_within_level`** -- Submit 5 Normal tasks labeled A through E in order. Assert that they execute in A, B, C, D, E order (FIFO within the same priority level), verified by an ordered log of execution.

- **`test_empty_queue_dispatch_is_noop`** -- Call `dispatch_frame` on an empty queue. Assert it returns immediately without panic and no tasks are submitted to the pool.

- **`test_cancelled_task_does_not_execute`** -- Submit a task with a pre-cancelled `CancellationToken`. Assert that the task's closure body never runs (verify with an `AtomicBool` that remains `false`).
