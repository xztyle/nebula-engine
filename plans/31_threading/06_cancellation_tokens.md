# Cancellation Tokens

## Problem

Long-running background tasks in the engine frequently become irrelevant before they complete:

- **Camera movement** -- The player flies across the planet at high speed. Dozens of chunk terrain generation tasks were dispatched for the previous camera position. By the time they complete, those chunks are behind the camera and will be immediately discarded. The CPU cycles are wasted.
- **Chunk unloading** -- A chunk leaves the loaded radius. Its meshing task is still running on a worker thread, generating vertices for geometry that will never be rendered. Meanwhile, the newly visible chunks in front of the camera wait in the queue.
- **Engine shutdown** -- The player quits the game. Hundreds of pending terrain and meshing tasks should abort immediately rather than running to completion while the player stares at a "shutting down" screen for 10 seconds.

Forcible thread termination (`pthread_cancel`, `TerminateThread`) is unsafe and unsupported in Rust. The engine needs cooperative cancellation: tasks periodically check whether they should stop, and if so, return early. This must be cheap (a single atomic load), hierarchical (cancelling a region cancels all chunks within it), and immediate (no polling delay beyond the task's own check interval).

## Solution

### CancellationToken

A cancellation token is a thin wrapper around a shared `AtomicBool`. Checking it costs a single atomic load with `Relaxed` ordering -- no memory barriers, no syscalls, no contention.

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

struct CancellationInner {
    cancelled: AtomicBool,
    children: std::sync::Mutex<Vec<CancellationToken>>,
    label: &'static str,
}

impl CancellationToken {
    /// Create a new uncancelled token.
    pub fn new(label: &'static str) -> Self {
        Self {
            inner: Arc::new(CancellationInner {
                cancelled: AtomicBool::new(false),
                children: std::sync::Mutex::new(Vec::new()),
                label,
            }),
        }
    }

    /// Check if this token has been cancelled.
    /// This is the hot-path call -- a single atomic load.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Relaxed)
    }

    /// Cancel this token and all its children, recursively.
    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Relaxed);
        let children = self.inner.children.lock()
            .expect("cancellation children lock poisoned");
        for child in children.iter() {
            child.cancel();
        }
        log::trace!("Cancelled token '{}'", self.inner.label);
    }

    /// Create a child token. When this token is cancelled,
    /// the child is also cancelled. The child can be independently
    /// cancelled without affecting the parent.
    pub fn child(&self, label: &'static str) -> CancellationToken {
        // If the parent is already cancelled, create an already-cancelled child.
        let child = CancellationToken::new(label);
        if self.is_cancelled() {
            child.cancel();
        }
        let mut children = self.inner.children.lock()
            .expect("cancellation children lock poisoned");
        children.push(child.clone());
        child
    }

    /// Create a pre-cancelled token (useful for testing).
    pub fn cancelled(label: &'static str) -> Self {
        let token = Self::new(label);
        token.cancel();
        token
    }

    pub fn label(&self) -> &'static str {
        self.inner.label
    }
}
```

### Usage in Tasks

Tasks check the cancellation token at natural breakpoints in their computation -- places where abandoning work is safe and the overhead of a single atomic load is negligible compared to the surrounding computation:

```rust
fn generate_terrain(
    chunk_id: ChunkId,
    params: &TerrainParams,
    token: &CancellationToken,
) -> Option<ChunkData> {
    let mut data = ChunkData::new();

    for y in 0..CHUNK_SIZE {
        // Check cancellation once per Y layer (32 checks per chunk).
        if token.is_cancelled() {
            log::debug!(
                "Terrain gen for chunk {:?} cancelled at layer {}",
                chunk_id, y
            );
            return None;
        }

        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let density = sample_noise(x, y, z, params);
                data.set(x, y, z, density_to_block(density));
            }
        }
    }

    Some(data)
}
```

For meshing, check once per face direction (6 checks per chunk). For physics broadphase, check once per spatial partition. The rule of thumb: check every 0.1 - 1.0 ms of CPU work.

### Hierarchical Cancellation

Cancellation tokens form a tree mirroring the engine's spatial hierarchy:

```
Global shutdown token
    |
    +-- Planet token (cancel all work for this planet)
    |       |
    |       +-- Face token (cancel all work on cube-sphere face 3)
    |       |       |
    |       |       +-- Chunk A terrain gen token
    |       |       +-- Chunk A meshing token
    |       |       +-- Chunk B terrain gen token
    |       |
    |       +-- Face token (face 5)
    |               |
    |               +-- Chunk C terrain gen token
    |
    +-- Network session token (cancel all network tasks)
```

Cancelling the "Face 3" token immediately cancels all terrain generation and meshing for chunks on that face, without affecting Face 5's work. Cancelling the global shutdown token cancels everything.

### Integration Points

The cancellation token integrates with the task system (story 01) and frame fence (story 03):

```rust
// When the camera moves, cancel outdated chunk work:
fn on_camera_sector_change(
    old_sector: SectorId,
    new_sector: SectorId,
    chunk_tokens: &mut HashMap<ChunkId, CancellationToken>,
) {
    for (chunk_id, token) in chunk_tokens.iter() {
        if !is_within_load_radius(*chunk_id, new_sector) {
            token.cancel();
        }
    }
}

// During engine shutdown:
fn shutdown(global_token: &CancellationToken) {
    global_token.cancel(); // Cancels everything in the hierarchy.
}
```

### Cheap Cloning

`CancellationToken` is `Clone` and cloning it is a single `Arc::clone` (atomic reference count increment). This makes it free to pass tokens into closures, across threads, and into task descriptors without copying the underlying state.

### Memory Cleanup

Cancelled tokens and their children remain in memory until all `Arc` references are dropped. The children `Vec` in a parent token holds strong references, so completed tasks should drop their token clones promptly. Periodically (e.g., once per second), the engine prunes the children lists by removing tokens whose `Arc` strong count is 1 (meaning no task holds a reference anymore):

```rust
impl CancellationToken {
    /// Remove children that are no longer referenced by any task.
    pub fn prune_children(&self) {
        let mut children = self.inner.children.lock()
            .expect("cancellation children lock poisoned");
        children.retain(|child| Arc::strong_count(&child.inner) > 1);
    }
}
```

## Outcome

A `CancellationToken` struct in the `nebula-threading` crate that provides cooperative, hierarchical cancellation for long-running tasks. Checking the token costs a single `Relaxed` atomic load. Cancelling a parent token recursively cancels all children. Tokens are cheap to clone (`Arc::clone`) and safe to share across threads. The token integrates with the task system, frame fence, and chunk lifecycle systems to eliminate wasted CPU work on outdated tasks.

## Demo Integration

**Demo crate:** `nebula-demo`

When the player teleports, in-flight terrain generation tasks for the old location are cancelled immediately. The console logs `Cancelled 47 stale tasks`.

## Crates & Dependencies

- **`log`** = `"0.4"` -- trace-level logging of cancellation events for debugging
- No additional external crates -- uses `std::sync::Arc`, `AtomicBool`, and `Mutex`
- Rust edition **2024**

## Unit Tests

- **`test_uncancelled_token_returns_false`** -- Create a `CancellationToken::new("test")`. Assert `is_cancelled()` returns `false`.

- **`test_cancelled_token_returns_true`** -- Create a `CancellationToken::new("test")`. Call `cancel()`. Assert `is_cancelled()` returns `true`.

- **`test_child_cancelled_when_parent_cancelled`** -- Create a parent token. Create a child via `parent.child("child")`. Cancel the parent. Assert the child's `is_cancelled()` returns `true`.

- **`test_parent_not_cancelled_when_child_cancelled`** -- Create a parent token. Create a child via `parent.child("child")`. Cancel the child only. Assert the parent's `is_cancelled()` returns `false`. Assert the child's `is_cancelled()` returns `true`.

- **`test_task_checks_and_exits_early`** -- Create a token and cancel it. Run a simulated task loop that checks `is_cancelled()` at the start of each iteration and returns early. Use an `AtomicU32` to count iterations. Assert the iteration count is 0 (or 1 if checked at the top of the loop), confirming the task exited immediately.

- **`test_cancellation_is_immediate`** -- Create a token. Spawn a thread that cancels the token after 1ms. In another thread, spin-loop on `is_cancelled()`. Measure the time between the cancel call and the spin-loop observing `true`. Assert the delay is less than 1ms (effectively immediate, limited only by atomic visibility which is near-instant on x86 and ARM).

- **`test_deep_hierarchy_cancellation`** -- Create a root token. Create 3 levels of children (root -> A -> B -> C). Cancel the root. Assert all tokens at every level return `is_cancelled() == true`.

- **`test_clone_shares_state`** -- Create a token. Clone it. Cancel the original. Assert the clone's `is_cancelled()` returns `true`. This confirms cloning shares the same underlying `AtomicBool`.

- **`test_pre_cancelled_child_of_cancelled_parent`** -- Create a parent token. Cancel the parent. Then create a child of the already-cancelled parent. Assert the child is immediately cancelled (its `is_cancelled()` returns `true` without an explicit `cancel()` call on the child).

- **`test_prune_removes_dead_children`** -- Create a parent token. Create 5 children and drop 3 of them (let them go out of scope). Call `prune_children()` on the parent. Assert the parent's internal children list has 2 entries remaining.
