# GPU Buffer Pool

## Problem

Every visible chunk on a cubesphere planet requires GPU-resident vertex and index buffers for rendering. As the player moves, chunks enter and leave the view distance, causing a constant cycle of mesh uploads and teardowns. Each `wgpu::Device::create_buffer` call triggers a driver-level GPU memory allocation, which is significantly more expensive than a CPU-side heap allocation -- it may involve kernel transitions, memory mapping, and page table updates. Destroying buffers is equally costly, and frequent create/destroy cycles fragment GPU memory, eventually leading to allocation failures or driver-level compaction stalls.

The GPU buffer pool (story 07_meshing/06 introduced the concept alongside mesh upload) deserves its own focused memory management story because it is a reusable subsystem: not just chunk meshes but also particle buffers, debug line buffers, UI geometry, and any other dynamically-sized GPU data can benefit from buffer reuse. This story provides the standalone, generic implementation.

## Solution

Implement a `GpuBufferPool` in the `nebula_memory` crate that manages a set of wgpu buffers organized by size class. Buffers are acquired and released rather than created and destroyed. Size classes use power-of-two buckets so that a requested size is rounded up to the nearest bucket, accepting a small amount of internal waste in exchange for high reuse rates.

### Size Class Buckets

```rust
/// Pre-defined buffer size classes.
/// Each bucket holds free buffers of exactly this size.
const SIZE_CLASSES: &[u64] = &[
    4 * 1024,       //   4 KB  — small chunks, debug geometry
    16 * 1024,      //  16 KB  — typical small chunk mesh
    64 * 1024,      //  64 KB  — medium chunk mesh
    256 * 1024,     // 256 KB  — large chunk mesh (fully exposed surfaces)
    1024 * 1024,    //   1 MB  — particle systems, large batches
];

/// Returns the index of the smallest size class that can hold `requested_bytes`.
/// Returns `None` if the requested size exceeds all size classes.
fn size_class_index(requested_bytes: u64) -> Option<usize> {
    SIZE_CLASSES.iter().position(|&sz| sz >= requested_bytes)
}
```

### GpuBufferPool

```rust
use wgpu;

/// A pool of reusable GPU buffers, bucketed by size class.
///
/// Buffers are created with `COPY_DST` usage so that data can be written
/// into them via `queue.write_buffer()` after acquisition.
pub struct GpuBufferPool {
    /// One free-list per size class, holding buffers available for reuse.
    buckets: Vec<Vec<wgpu::Buffer>>,
    /// The required usage flags for all buffers in this pool.
    usage: wgpu::BufferUsages,
    /// Total GPU memory allocated by this pool (active + free), in bytes.
    total_allocated_bytes: u64,
    /// GPU memory currently in active use (acquired, not yet released), in bytes.
    active_bytes: u64,
    /// Label prefix for wgpu debug labels.
    label_prefix: &'static str,
}

impl GpuBufferPool {
    /// Create a new buffer pool.
    ///
    /// `usage` specifies the wgpu buffer usage flags. Typically
    /// `BufferUsages::VERTEX | BufferUsages::COPY_DST` for vertex pools
    /// or `BufferUsages::INDEX | BufferUsages::COPY_DST` for index pools.
    pub fn new(usage: wgpu::BufferUsages, label_prefix: &'static str) -> Self {
        Self {
            buckets: vec![Vec::new(); SIZE_CLASSES.len()],
            usage,
            total_allocated_bytes: 0,
            active_bytes: 0,
            label_prefix,
        }
    }

    /// Acquire a buffer of at least `min_bytes` from the pool.
    ///
    /// If a free buffer exists in the appropriate size class, it is returned.
    /// Otherwise, a new buffer is created via `device.create_buffer()`.
    ///
    /// Returns `(buffer, size_class_index, actual_size)`.
    ///
    /// # Panics
    /// Panics if `min_bytes` exceeds the largest size class.
    pub fn acquire(
        &mut self,
        device: &wgpu::Device,
        min_bytes: u64,
    ) -> GpuPooledBuffer {
        let class_idx = size_class_index(min_bytes)
            .expect("requested buffer size exceeds maximum size class");
        let actual_size = SIZE_CLASSES[class_idx];

        let buffer = if let Some(buf) = self.buckets[class_idx].pop() {
            // Reuse an existing buffer.
            buf
        } else {
            // No free buffer in this class -- create a new one.
            self.total_allocated_bytes += actual_size;
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label_prefix),
                size: actual_size,
                usage: self.usage,
                mapped_at_creation: false,
            })
        };

        self.active_bytes += actual_size;

        GpuPooledBuffer {
            buffer,
            size_class: class_idx,
            size: actual_size,
        }
    }

    /// Release a buffer back to the pool for reuse.
    ///
    /// The buffer is placed in its size class bucket.  The GPU memory is not
    /// freed -- it remains allocated and ready for the next `acquire()`.
    pub fn release(&mut self, pooled: GpuPooledBuffer) {
        let size = SIZE_CLASSES[pooled.size_class];
        self.active_bytes -= size;
        self.buckets[pooled.size_class].push(pooled.buffer);
    }

    /// Total GPU memory allocated by this pool (active + pooled free), in bytes.
    pub fn total_allocated_bytes(&self) -> u64 {
        self.total_allocated_bytes
    }

    /// GPU memory currently in active use (acquired but not released), in bytes.
    pub fn active_bytes(&self) -> u64 {
        self.active_bytes
    }

    /// GPU memory sitting idle in the pool (allocated but not in use), in bytes.
    pub fn free_bytes(&self) -> u64 {
        self.total_allocated_bytes - self.active_bytes
    }

    /// Number of free buffers in each size class (for diagnostics).
    pub fn free_counts(&self) -> Vec<(u64, usize)> {
        SIZE_CLASSES
            .iter()
            .zip(self.buckets.iter())
            .map(|(&size, bucket)| (size, bucket.len()))
            .collect()
    }

    /// Drop all free buffers, releasing their GPU memory.
    /// Active (acquired) buffers are not affected.
    pub fn drain_free(&mut self) {
        for bucket in &mut self.buckets {
            let freed: u64 = bucket.len() as u64
                * SIZE_CLASSES[self.buckets.iter().position(|b| std::ptr::eq(b, bucket)).unwrap_or(0)];
            self.total_allocated_bytes -= freed;
            bucket.clear();
        }
    }

    /// Drop all free buffers, releasing GPU memory, with a cleaner implementation.
    pub fn drain_free_buffers(&mut self) {
        for (i, bucket) in self.buckets.iter_mut().enumerate() {
            let class_size = SIZE_CLASSES[i];
            let count = bucket.len() as u64;
            self.total_allocated_bytes -= count * class_size;
            bucket.clear();
        }
    }
}
```

### GpuPooledBuffer Handle

```rust
/// A GPU buffer acquired from a `GpuBufferPool`.
///
/// Holds the buffer handle and the metadata needed to return it to the pool.
pub struct GpuPooledBuffer {
    /// The underlying wgpu buffer.
    pub buffer: wgpu::Buffer,
    /// Which size class bucket this buffer belongs to.
    size_class: usize,
    /// Actual allocated size in bytes.
    pub size: u64,
}

impl GpuPooledBuffer {
    /// Write data into this buffer using the wgpu queue.
    ///
    /// # Panics
    /// Panics if `data.len()` exceeds `self.size`.
    pub fn write(&self, queue: &wgpu::Queue, data: &[u8]) {
        assert!(
            data.len() as u64 <= self.size,
            "data ({} bytes) exceeds buffer size ({} bytes)",
            data.len(),
            self.size,
        );
        queue.write_buffer(&self.buffer, 0, data);
    }
}
```

### Dual Pool for Mesh Rendering

For chunk mesh rendering, the engine maintains two pools -- one for vertex buffers, one for index buffers:

```rust
/// Combined vertex + index buffer pool for chunk mesh rendering.
pub struct MeshBufferPools {
    pub vertex_pool: GpuBufferPool,
    pub index_pool: GpuBufferPool,
}

impl MeshBufferPools {
    pub fn new() -> Self {
        Self {
            vertex_pool: GpuBufferPool::new(
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                "chunk_vertex",
            ),
            index_pool: GpuBufferPool::new(
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                "chunk_index",
            ),
        }
    }

    /// Total GPU memory used by both pools.
    pub fn total_gpu_memory(&self) -> u64 {
        self.vertex_pool.total_allocated_bytes() + self.index_pool.total_allocated_bytes()
    }
}
```

### Design Decisions

- **Five size classes**: The classes (4 KB, 16 KB, 64 KB, 256 KB, 1 MB) cover the range of chunk mesh sizes observed in practice. A fully exposed chunk (worst case, checkerboard pattern) generates approximately 200 KB of vertex data. The 256 KB bucket handles this with only 22% waste. Most chunks produce far less geometry and fit in the 16 KB or 64 KB buckets.
- **Separate vertex and index pools**: Vertex and index buffers have different wgpu usage flags. Keeping them in separate pools avoids mixing flags and simplifies the acquire/release logic.
- **No oversized fallback**: If a requested size exceeds 1 MB, the pool panics. This is intentional -- no single chunk mesh should ever be that large. If it is, there is a bug in the meshing pipeline. A future story could add an "oversized" path for non-chunk uses.
- **`COPY_DST` usage**: All pooled buffers include `COPY_DST` so that data can be written into them via `queue.write_buffer()` after acquisition. This enables reuse -- the same buffer can hold different mesh data across frames.

## Outcome

The `nebula_memory` crate exports `GpuBufferPool`, `GpuPooledBuffer`, and `MeshBufferPools`. The chunk meshing and rendering systems use these pools instead of creating and destroying wgpu buffers directly. GPU memory usage is tracked and queryable. Running `cargo test -p nebula_memory` passes all GPU buffer pool tests (using wgpu's headless/software backend for CI). The crate uses Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

GPU vertex and index buffers are recycled. Mesh re-generation reuses existing GPU allocations where possible. GPU memory fragmentation is reduced.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | GPU buffer creation, buffer descriptors, queue writes |
| `pollster` | `0.4` | Block on async adapter/device requests in tests |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a test device using wgpu's software fallback backend.
    fn test_device() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: true,
            compatible_surface: None,
        }))
        .expect("no software adapter available");
        pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor::default(),
            None,
        ))
        .expect("failed to create device")
    }

    /// Requesting a 10 KB buffer should acquire from the 16 KB bucket (the
    /// smallest bucket that fits 10 KB).
    #[test]
    fn test_buffer_acquired_from_correct_bucket() {
        let (device, _queue) = test_device();
        let mut pool = GpuBufferPool::new(
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "test",
        );

        let buf = pool.acquire(&device, 10 * 1024); // 10 KB requested

        assert_eq!(buf.size, 16 * 1024, "should round up to 16 KB bucket");
        assert_eq!(buf.size_class, 1, "16 KB is size class index 1");
    }

    /// A released buffer should be reused on the next acquire of the same
    /// size class, with no new GPU allocation.
    #[test]
    fn test_released_buffer_is_reused() {
        let (device, _queue) = test_device();
        let mut pool = GpuBufferPool::new(
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "test",
        );

        let buf1 = pool.acquire(&device, 5 * 1024);
        let total_after_first = pool.total_allocated_bytes();
        pool.release(buf1);

        let _buf2 = pool.acquire(&device, 5 * 1024);
        let total_after_second = pool.total_allocated_bytes();

        assert_eq!(
            total_after_first, total_after_second,
            "second acquire should reuse the released buffer, not allocate new GPU memory"
        );
    }

    /// Requesting different sizes should use different buckets.
    #[test]
    fn test_different_sizes_use_different_buckets() {
        let (device, _queue) = test_device();
        let mut pool = GpuBufferPool::new(
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "test",
        );

        let small = pool.acquire(&device, 2 * 1024);   // -> 4 KB bucket
        let medium = pool.acquire(&device, 50 * 1024);  // -> 64 KB bucket
        let large = pool.acquire(&device, 200 * 1024);  // -> 256 KB bucket

        assert_eq!(small.size, 4 * 1024);
        assert_eq!(medium.size, 64 * 1024);
        assert_eq!(large.size, 256 * 1024);

        assert_ne!(small.size_class, medium.size_class);
        assert_ne!(medium.size_class, large.size_class);
    }

    /// The total_allocated_bytes and active_bytes counters should accurately
    /// reflect the state of the pool.
    #[test]
    fn test_total_memory_tracking_is_accurate() {
        let (device, _queue) = test_device();
        let mut pool = GpuBufferPool::new(
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "test",
        );

        assert_eq!(pool.total_allocated_bytes(), 0);
        assert_eq!(pool.active_bytes(), 0);

        let buf_a = pool.acquire(&device, 4 * 1024);  // 4 KB
        assert_eq!(pool.total_allocated_bytes(), 4 * 1024);
        assert_eq!(pool.active_bytes(), 4 * 1024);

        let buf_b = pool.acquire(&device, 60 * 1024); // 64 KB
        assert_eq!(pool.total_allocated_bytes(), 4 * 1024 + 64 * 1024);
        assert_eq!(pool.active_bytes(), 4 * 1024 + 64 * 1024);

        pool.release(buf_a);
        assert_eq!(
            pool.total_allocated_bytes(),
            4 * 1024 + 64 * 1024,
            "total should not decrease on release"
        );
        assert_eq!(
            pool.active_bytes(),
            64 * 1024,
            "active should decrease on release"
        );
        assert_eq!(pool.free_bytes(), 4 * 1024, "free should be the released buffer");

        pool.release(buf_b);
        assert_eq!(pool.active_bytes(), 0);
        assert_eq!(pool.free_bytes(), 4 * 1024 + 64 * 1024);
    }

    /// If the pool is empty for a given size class, acquire should create a
    /// new buffer rather than failing.
    #[test]
    fn test_empty_pool_creates_new_buffer() {
        let (device, _queue) = test_device();
        let mut pool = GpuBufferPool::new(
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "test",
        );

        // Pool starts empty -- every acquire should succeed by creating a new buffer.
        let buf1 = pool.acquire(&device, 1024);
        let buf2 = pool.acquire(&device, 1024);
        let buf3 = pool.acquire(&device, 1024);

        assert_eq!(pool.total_allocated_bytes(), 3 * 4 * 1024);
        assert_eq!(pool.active_bytes(), 3 * 4 * 1024);

        pool.release(buf1);
        pool.release(buf2);
        pool.release(buf3);
    }

    /// The free_counts diagnostic should report correct counts per bucket.
    #[test]
    fn test_free_counts_per_bucket() {
        let (device, _queue) = test_device();
        let mut pool = GpuBufferPool::new(
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            "test",
        );

        let a = pool.acquire(&device, 3 * 1024);  // 4 KB bucket
        let b = pool.acquire(&device, 3 * 1024);  // 4 KB bucket
        let c = pool.acquire(&device, 50 * 1024); // 64 KB bucket

        pool.release(a);
        pool.release(b);
        pool.release(c);

        let counts = pool.free_counts();
        // 4 KB bucket should have 2 free buffers
        assert_eq!(counts[0], (4 * 1024, 2));
        // 64 KB bucket should have 1 free buffer
        assert_eq!(counts[2], (64 * 1024, 1));
        // Other buckets should be empty
        assert_eq!(counts[1].1, 0); // 16 KB
        assert_eq!(counts[3].1, 0); // 256 KB
        assert_eq!(counts[4].1, 0); // 1 MB
    }
}
```
