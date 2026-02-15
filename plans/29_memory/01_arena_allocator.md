# Arena Allocator

## Problem

Every frame, the engine creates thousands of short-lived temporary objects: intermediate mesh buffers during greedy meshing, query result lists from the spatial hash, scratch vectors for frustum culling, LOD priority lists, and per-system working sets. These allocations follow a predictable lifecycle -- they are created during a frame, used briefly, and then all discarded together at the frame boundary. The standard global allocator (`malloc`/`free`) is not optimized for this pattern. Each allocation requires a search for a free block, and each deallocation updates free lists, leading to overhead that scales linearly with the number of allocations. Worse, many small allocations fragment the heap, causing cache misses and increasing page faults over long play sessions.

A bump allocator (arena) eliminates per-allocation overhead by advancing a single pointer into a pre-allocated memory block. At frame end, the entire arena is "freed" by resetting the pointer to the start -- an O(1) operation regardless of how many allocations were made. This is the standard approach in game engines where per-frame temporary data dominates allocation counts.

Additionally, in a multi-threaded ECS engine (Bevy), multiple systems run in parallel. A single arena shared across threads would require synchronization. Thread-local arenas avoid contention entirely, giving each worker thread its own bump region.

## Solution

Implement a `FrameArena` allocator in the `nebula_memory` crate. The arena pre-allocates a contiguous block of memory at startup and hands out sub-allocations by bumping a pointer.

### Core Arena

```rust
use std::alloc::Layout;
use std::cell::Cell;
use std::ptr::NonNull;

/// A bump/arena allocator for short-lived per-frame allocations.
///
/// All allocations are freed together when `reset()` is called.
/// Individual deallocation is not supported.
pub struct FrameArena {
    /// Start of the pre-allocated memory block.
    base: NonNull<u8>,
    /// Total capacity in bytes.
    capacity: usize,
    /// Current allocation offset from `base`. Advances on each allocation.
    offset: Cell<usize>,
    /// Layout used for the original allocation (needed for dealloc on drop).
    layout: Layout,
    /// High watermark: maximum `offset` value ever observed (for diagnostics).
    high_watermark: Cell<usize>,
}

impl FrameArena {
    /// Create a new arena with the given capacity in bytes.
    /// The memory is allocated once from the global allocator.
    ///
    /// # Panics
    /// Panics if `capacity` is 0 or if the allocation fails.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "Arena capacity must be > 0");
        let layout = Layout::from_size_align(capacity, 16)
            .expect("invalid layout");
        // SAFETY: layout has non-zero size.
        let ptr = unsafe { std::alloc::alloc(layout) };
        let base = NonNull::new(ptr).expect("arena allocation failed");
        Self {
            base,
            capacity,
            offset: Cell::new(0),
            layout,
            high_watermark: Cell::new(0),
        }
    }

    /// Allocate `size` bytes with the given alignment from the arena.
    /// Returns `None` if the arena does not have enough remaining capacity.
    ///
    /// The returned pointer is valid until the next call to `reset()`.
    pub fn alloc(&self, layout: Layout) -> Option<NonNull<u8>> {
        let current = self.offset.get();

        // Align the current offset up to the required alignment.
        let aligned = (current + layout.align() - 1) & !(layout.align() - 1);
        let new_offset = aligned + layout.size();

        if new_offset > self.capacity {
            return None; // Out of space
        }

        self.offset.set(new_offset);

        // Update high watermark
        if new_offset > self.high_watermark.get() {
            self.high_watermark.set(new_offset);
        }

        // SAFETY: `aligned` is within bounds (checked above) and the base pointer
        // is valid for `capacity` bytes.
        let ptr = unsafe { self.base.as_ptr().add(aligned) };
        Some(unsafe { NonNull::new_unchecked(ptr) })
    }

    /// Allocate and initialize a value of type `T` in the arena.
    /// Returns a reference that lives until the next `reset()`.
    pub fn alloc_val<T>(&self, value: T) -> Option<&mut T> {
        let layout = Layout::new::<T>();
        let ptr = self.alloc(layout)?;
        let typed = ptr.as_ptr() as *mut T;
        // SAFETY: pointer is aligned and within arena bounds.
        unsafe {
            typed.write(value);
            Some(&mut *typed)
        }
    }

    /// Allocate a slice of `count` elements of type `T`, initialized to the given value.
    pub fn alloc_slice<T: Copy>(&self, count: usize, value: T) -> Option<&mut [T]> {
        let layout = Layout::array::<T>(count).ok()?;
        let ptr = self.alloc(layout)?;
        let slice_ptr = ptr.as_ptr() as *mut T;
        // SAFETY: pointer is within bounds and properly aligned for T.
        unsafe {
            for i in 0..count {
                slice_ptr.add(i).write(value);
            }
            Some(std::slice::from_raw_parts_mut(slice_ptr, count))
        }
    }

    /// Reset the arena, reclaiming all allocated memory.
    /// This is O(1) -- it simply resets the bump pointer.
    ///
    /// # Safety
    /// All previously returned pointers and references become invalid after this call.
    /// The caller must ensure no references into the arena are held.
    pub unsafe fn reset(&self) {
        self.offset.set(0);
    }

    /// Number of bytes currently allocated in this arena.
    pub fn bytes_used(&self) -> usize {
        self.offset.get()
    }

    /// Total capacity of this arena in bytes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The maximum number of bytes ever allocated at once (across all frames).
    pub fn high_watermark(&self) -> usize {
        self.high_watermark.get()
    }

    /// Remaining bytes available for allocation.
    pub fn bytes_remaining(&self) -> usize {
        self.capacity - self.offset.get()
    }
}

impl Drop for FrameArena {
    fn drop(&mut self) {
        // SAFETY: `base` was allocated with `self.layout` in `new()`.
        unsafe {
            std::alloc::dealloc(self.base.as_ptr(), self.layout);
        }
    }
}
```

### Thread-Local Arenas

For parallel ECS systems, each worker thread gets its own arena via `thread_local!`:

```rust
use std::cell::RefCell;

/// Default per-thread arena capacity: 16 MB.
const DEFAULT_THREAD_ARENA_CAPACITY: usize = 16 * 1024 * 1024;

thread_local! {
    static THREAD_ARENA: RefCell<FrameArena> = RefCell::new(
        FrameArena::new(DEFAULT_THREAD_ARENA_CAPACITY)
    );
}

/// Allocate from the current thread's frame arena.
/// Returns `None` if the thread arena is out of space.
pub fn thread_arena_alloc(layout: Layout) -> Option<NonNull<u8>> {
    THREAD_ARENA.with(|arena| arena.borrow().alloc(layout))
}

/// Reset the current thread's frame arena.
/// Must be called at the end of each frame on every worker thread.
///
/// # Safety
/// No references into the arena may be held when this is called.
pub unsafe fn thread_arena_reset() {
    THREAD_ARENA.with(|arena| arena.borrow().reset());
}
```

### ECS Integration

A Bevy system resets all thread-local arenas at the start or end of each frame:

```rust
/// System that resets the frame arena at the end of each frame.
/// Runs in the `Last` schedule after all other systems have completed.
pub fn reset_frame_arenas_system() {
    // SAFETY: This system runs after all other systems in the frame, so no
    // references into the arena should be held.
    unsafe { thread_arena_reset(); }
}
```

### Design Decisions

- **`Cell<usize>` for offset**: The offset uses interior mutability so that `alloc()` takes `&self` instead of `&mut self`. This allows multiple allocations from the same arena reference without exclusive borrow -- important for ergonomic use within a single system.
- **No individual free**: Arena allocators do not support freeing individual allocations. This is by design -- the entire point is that all memory is freed at once via `reset()`. This makes the allocator trivially simple and extremely fast.
- **16-byte base alignment**: The base block is aligned to 16 bytes, which satisfies alignment requirements for most types including SIMD types. Individual allocations respect the alignment specified in the `Layout`.
- **Unsafe reset**: `reset()` is marked `unsafe` because it invalidates all outstanding pointers. The frame boundary system is responsible for calling this at the correct time.
- **High watermark tracking**: The high watermark enables the engine to monitor how much arena space is actually used per frame, informing capacity tuning. If the watermark consistently stays below 2 MB in a 16 MB arena, the capacity can be reduced.

## Outcome

The `nebula_memory` crate exports `FrameArena`, `thread_arena_alloc()`, `thread_arena_reset()`, and `reset_frame_arenas_system()`. Per-frame temporary allocations throughout the engine use the arena instead of the global allocator, eliminating per-allocation overhead and heap fragmentation. Running `cargo test -p nebula_memory` passes all arena tests. The crate uses Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Temporary per-frame allocations (meshing scratch buffers, sort arrays) use an arena that resets each frame. Frame time variance drops because allocation is O(1).

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.15` | ECS system registration for the frame-end reset system (workspace dependency) |
| `bevy_app` | `0.15` | `Last` schedule for system ordering (workspace dependency) |

No external allocator crates are required. The arena is implemented directly using `std::alloc::alloc` and `std::alloc::dealloc`.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::Layout;

    /// Allocating from a fresh arena should return a valid, non-null pointer.
    #[test]
    fn test_alloc_returns_valid_pointer() {
        let arena = FrameArena::new(4096);
        let layout = Layout::from_size_align(64, 8).unwrap();

        let ptr = arena.alloc(layout);

        assert!(ptr.is_some(), "allocation should succeed");
        let ptr = ptr.unwrap();
        assert!(!ptr.as_ptr().is_null());
        // The pointer should be properly aligned.
        assert_eq!(ptr.as_ptr() as usize % 8, 0);
    }

    /// Multiple allocations should return non-overlapping memory regions.
    #[test]
    fn test_multiple_allocations_dont_overlap() {
        let arena = FrameArena::new(4096);
        let layout = Layout::from_size_align(128, 8).unwrap();

        let ptr_a = arena.alloc(layout).expect("first alloc");
        let ptr_b = arena.alloc(layout).expect("second alloc");
        let ptr_c = arena.alloc(layout).expect("third alloc");

        let a = ptr_a.as_ptr() as usize;
        let b = ptr_b.as_ptr() as usize;
        let c = ptr_c.as_ptr() as usize;

        // Each region is 128 bytes. They must not overlap.
        assert!(b >= a + 128, "B ({b:#x}) overlaps A ({a:#x})");
        assert!(c >= b + 128, "C ({c:#x}) overlaps B ({b:#x})");
    }

    /// After reset, bytes_used should return to 0 and new allocations
    /// should start from the beginning of the arena.
    #[test]
    fn test_reset_frees_all_memory() {
        let arena = FrameArena::new(4096);
        let layout = Layout::from_size_align(256, 8).unwrap();

        arena.alloc(layout).expect("alloc before reset");
        assert!(arena.bytes_used() > 0);

        // SAFETY: no references into the arena are held.
        unsafe { arena.reset(); }

        assert_eq!(arena.bytes_used(), 0, "bytes_used should be 0 after reset");
        assert_eq!(
            arena.bytes_remaining(),
            arena.capacity(),
            "full capacity should be available after reset"
        );
    }

    /// Allocations that would exceed arena capacity should return None.
    #[test]
    fn test_arena_respects_capacity() {
        let arena = FrameArena::new(256);

        // Allocate exactly 256 bytes (may succeed depending on alignment).
        let layout_full = Layout::from_size_align(256, 1).unwrap();
        let first = arena.alloc(layout_full);
        assert!(first.is_some(), "should fit in 256-byte arena");

        // Now the arena is full. Another allocation should fail.
        let layout_extra = Layout::from_size_align(1, 1).unwrap();
        let second = arena.alloc(layout_extra);
        assert!(second.is_none(), "arena should be full");
    }

    /// After reset, allocations should reuse the same memory region
    /// (bump pointer starts from the beginning again).
    #[test]
    fn test_allocation_after_reset_reuses_memory() {
        let arena = FrameArena::new(4096);
        let layout = Layout::from_size_align(64, 8).unwrap();

        let ptr_before = arena.alloc(layout).expect("alloc before reset");

        // SAFETY: no references into the arena are held after this point.
        unsafe { arena.reset(); }

        let ptr_after = arena.alloc(layout).expect("alloc after reset");

        // Both allocations should return the same address since the bump
        // pointer was reset and alignment is the same.
        assert_eq!(
            ptr_before.as_ptr(),
            ptr_after.as_ptr(),
            "after reset, the first allocation should reuse the same address"
        );
    }

    /// The high watermark should track the maximum bytes ever allocated.
    #[test]
    fn test_high_watermark_tracks_peak_usage() {
        let arena = FrameArena::new(4096);
        let layout = Layout::from_size_align(100, 8).unwrap();

        arena.alloc(layout).unwrap();
        arena.alloc(layout).unwrap();
        arena.alloc(layout).unwrap();
        let peak = arena.high_watermark();
        assert!(peak >= 300, "watermark should be at least 300 bytes");

        // SAFETY: no references held.
        unsafe { arena.reset(); }

        // Watermark should persist after reset.
        assert_eq!(
            arena.high_watermark(),
            peak,
            "high watermark should survive reset"
        );
    }

    /// alloc_val should store and return the correct value.
    #[test]
    fn test_alloc_val_stores_value() {
        let arena = FrameArena::new(4096);

        let val = arena.alloc_val(42u64).expect("alloc_val");
        assert_eq!(*val, 42);

        *val = 99;
        assert_eq!(*val, 99);
    }

    /// alloc_slice should return a properly initialized slice.
    #[test]
    fn test_alloc_slice_initializes_values() {
        let arena = FrameArena::new(4096);

        let slice = arena.alloc_slice::<u32>(10, 0xDEAD).expect("alloc_slice");
        assert_eq!(slice.len(), 10);
        for &val in slice.iter() {
            assert_eq!(val, 0xDEAD);
        }
    }
}
```
