# Frame Sync Barriers

## Problem

The game loop runs at a fixed timestep (see `01_setup/06_main_game_loop_with_fixed_timestep.md`), and each frame dispatches work to the worker thread pool (story 01). Some of that work is Critical and must complete before the frame can render -- if the player's chunk was just re-meshed, the renderer needs the new vertex buffer this frame, not next frame. Other work is speculative or low-priority and can span multiple frames without visual artifacts.

Without explicit synchronization at the frame boundary, the engine faces two failure modes:

1. **Rendering stale data** -- The renderer draws the scene before critical mesh results have arrived, causing a one-frame pop-in artifact visible as flickering geometry.
2. **Blocking on everything** -- A naive barrier that waits for all tasks (including Low-priority ones) forces the frame to wait for the slowest background task, destroying frame rate.

The engine needs a frame synchronization mechanism that distinguishes between tasks that must complete this frame and tasks that can float across frames.

## Solution

### Frame Fence

A `FrameFence` tracks outstanding critical tasks for a given frame. It uses an `AtomicU32` counter that increments when a critical task is dispatched and decrements when it completes. The main thread waits (spins briefly, then parks) until the counter reaches zero.

```rust
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

pub struct FrameFence {
    frame_counter: AtomicU64,
    pending_critical: AtomicU32,
    condvar: std::sync::Condvar,
    mutex: std::sync::Mutex<()>,
}

impl FrameFence {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            frame_counter: AtomicU64::new(0),
            pending_critical: AtomicU32::new(0),
            condvar: std::sync::Condvar::new(),
            mutex: std::sync::Mutex::new(()),
        })
    }

    /// Called at the start of each frame. Advances the frame counter
    /// and resets the critical task count.
    pub fn begin_frame(&self) -> u64 {
        let frame = self.frame_counter.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending_critical.store(0, Ordering::SeqCst);
        frame
    }

    /// Called when a Critical task is dispatched for this frame.
    pub fn register_critical(&self) {
        self.pending_critical.fetch_add(1, Ordering::SeqCst);
    }

    /// Called when a Critical task completes.
    pub fn complete_critical(&self) {
        let prev = self.pending_critical.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(prev > 0, "critical task completed but none were pending");
        if prev == 1 {
            // Last critical task finished -- wake the main thread.
            self.condvar.notify_one();
        }
    }

    /// Blocks until all critical tasks for this frame have completed.
    /// Called at the end of the frame, before rendering.
    pub fn wait_for_critical(&self) {
        let guard = self.mutex.lock().expect("fence mutex poisoned");
        let timeout = std::time::Duration::from_millis(100);

        let _guard = self.condvar.wait_timeout_while(
            guard,
            timeout,
            |_| self.pending_critical.load(Ordering::SeqCst) > 0,
        ).expect("fence condvar wait failed");

        let remaining = self.pending_critical.load(Ordering::SeqCst);
        if remaining > 0 {
            log::warn!(
                "Frame fence timed out with {} critical tasks still pending",
                remaining
            );
        }
    }

    pub fn current_frame(&self) -> u64 {
        self.frame_counter.load(Ordering::SeqCst)
    }
}
```

### Frame Lifecycle

Each frame follows this sequence:

```
begin_frame()
    |
    +-- dispatch Critical tasks (each calls register_critical)
    +-- dispatch High/Normal/Low tasks (no registration needed)
    |
    +-- [main thread does other CPU work: ECS schedules, input, etc.]
    |
    +-- wait_for_critical()  <-- blocks until all Critical tasks finish
    |
    +-- drain result channels (collect mesh results, chunk data, etc.)
    |
    +-- render()
```

### Task Integration

When the task system (story 01) dispatches a Critical task, it registers it with the frame fence:

```rust
if task.descriptor.priority == TaskPriority::Critical {
    fence.register_critical();
    let fence_clone = fence.clone();
    pool.spawn(move || {
        let result = (task.work)();
        fence_clone.complete_critical();
        result
    });
} else {
    pool.spawn(move || {
        (task.work)();
    });
}
```

### Non-Critical Task Spanning

High, Normal, and Low tasks have no fence registration. They execute whenever a worker thread picks them up and deliver results through channels (story 02). The main thread collects whatever results are available each frame via `drain_channel`. If a Normal task's result arrives two frames after dispatch, that is acceptable -- the chunk will simply appear two frames later, which is invisible to the player for distant geometry.

### Multi-Frame Task Tracking

For tasks that intentionally span multiple frames (e.g., a large terrain generation job), the task carries the `frame_tag` of the frame that dispatched it. The result includes this tag, allowing the consumer to understand how stale the result is:

```rust
pub struct TaskResult<T> {
    pub value: T,
    pub dispatched_frame: u64,
    pub completed_frame: u64,
}
```

If `completed_frame - dispatched_frame > threshold`, the consumer may discard the result as outdated (e.g., the camera moved away and the chunk is no longer needed).

### Timeout Safety

The `wait_for_critical` call includes a 100ms timeout. If critical tasks do not complete within this window, the frame proceeds anyway with a logged warning. This prevents a single misbehaving task from freezing the entire engine. The rendering system handles missing data gracefully by reusing the previous frame's buffers.

## Outcome

A `FrameFence` struct in the `nebula-threading` crate that provides per-frame synchronization for critical tasks. The fence tracks outstanding critical work using atomic counters and wakes the main thread via condvar when all critical tasks complete. Non-critical tasks execute asynchronously across frame boundaries. A timeout prevents deadlocks from misbehaving tasks. Frame tagging enables staleness detection for multi-frame results.

## Demo Integration

**Demo crate:** `nebula-demo`

A sync point at the end of each frame ensures all worker results are collected before rendering. No torn reads or partial updates are visible.

## Crates & Dependencies

- **`log`** = `"0.4"` -- logging timeout warnings and fence state for debugging
- No additional external crates -- uses `std::sync` atomics, `Condvar`, and `Mutex`
- Rust edition **2024**

## Unit Tests

- **`test_barrier_waits_for_all_critical`** -- Create a `FrameFence`. Register 5 critical tasks. Spawn 5 threads that each sleep for 10ms then call `complete_critical`. Call `wait_for_critical` on the main thread. Assert that `wait_for_critical` returns only after all 5 threads have called `complete_critical` (verify with an `AtomicU32` counter that equals 5 after the wait).

- **`test_non_critical_tasks_continue_past_barrier`** -- Create a `FrameFence`. Register 1 critical task and spawn a non-critical task that sleeps for 200ms and sets an `AtomicBool`. Complete the critical task immediately. Call `wait_for_critical` -- it should return promptly. Assert the `AtomicBool` is still `false` (the non-critical task is still running). Wait for the non-critical task to finish and assert it eventually sets the bool to `true`.

- **`test_frame_counter_increments`** -- Create a `FrameFence`. Call `begin_frame` three times. Assert `current_frame()` returns 3.

- **`test_tasks_from_different_frames_independent`** -- Call `begin_frame` (frame 1). Register 2 critical tasks. Call `begin_frame` (frame 2), which resets the critical count. Register 1 critical task. Complete the 1 frame-2 task. Call `wait_for_critical` -- it should return immediately because frame 2 has no outstanding critical tasks, even though the 2 frame-1 tasks are still pending.

- **`test_wait_with_no_critical_tasks_returns_immediately`** -- Create a `FrameFence`. Call `begin_frame` without registering any critical tasks. Call `wait_for_critical`. Assert it returns within 1ms (effectively immediately).

- **`test_fence_timeout_does_not_deadlock`** -- Create a `FrameFence`. Register 1 critical task but never complete it. Call `wait_for_critical`. Assert it returns after approximately 100ms (the timeout) rather than blocking forever. Verify the warning was logged.

- **`test_task_result_staleness_detection`** -- Create a `TaskResult` with `dispatched_frame = 10` and `completed_frame = 15`. Assert that `completed_frame - dispatched_frame == 5`. Verify that a threshold check (e.g., `> 3 frames`) correctly identifies this result as stale.
