//! GPU buffer pool for reusing wgpu buffers across chunk mesh uploads.
//!
//! Instead of creating and destroying GPU buffers every time a chunk is
//! meshed/remeshed, the [`GpuBufferPool`] maintains buckets of pre-allocated
//! buffers in common size classes. This reduces driver-level allocation
//! overhead and GPU memory fragmentation.

/// Number of size classes in the pool.
const NUM_SIZE_CLASSES: usize = 6;

/// Size class thresholds in bytes: 4 KB, 8 KB, 16 KB, 32 KB, 64 KB, 128 KB.
const SIZE_CLASSES: [u64; NUM_SIZE_CLASSES] = [4096, 8192, 16384, 32768, 65536, 131072];

/// A pool of GPU buffers bucketed by size class.
///
/// Tracks total allocated and in-use GPU memory for chunk meshes.
pub struct GpuBufferPool {
    /// Free vertex buffers, bucketed by size class.
    vertex_pool: [Vec<wgpu::Buffer>; NUM_SIZE_CLASSES],
    /// Free index buffers, bucketed by size class.
    index_pool: [Vec<wgpu::Buffer>; NUM_SIZE_CLASSES],
    /// Total bytes currently allocated (in-use + pooled).
    total_allocated: u64,
    /// Total bytes currently in use (uploaded, not pooled).
    in_use: u64,
}

impl GpuBufferPool {
    /// Create a new empty buffer pool.
    pub fn new() -> Self {
        Self {
            vertex_pool: Default::default(),
            index_pool: Default::default(),
            total_allocated: 0,
            in_use: 0,
        }
    }

    /// Find the size class index for a given minimum byte size.
    ///
    /// Returns the index of the smallest size class that can hold `min_size`
    /// bytes. If `min_size` exceeds the largest class, returns the last index.
    pub fn size_class_for(&self, min_size: u64) -> usize {
        SIZE_CLASSES
            .iter()
            .position(|&s| s >= min_size)
            .unwrap_or(NUM_SIZE_CLASSES - 1)
    }

    /// The actual byte size of a given size class.
    pub fn class_size(class: usize) -> u64 {
        SIZE_CLASSES[class.min(NUM_SIZE_CLASSES - 1)]
    }

    /// Acquire a vertex buffer of at least `min_size` bytes.
    ///
    /// Returns a pooled buffer if available, or creates a new one.
    pub fn acquire_vertex_buffer(
        &mut self,
        device: &wgpu::Device,
        min_size: u64,
    ) -> (wgpu::Buffer, usize) {
        let class = self.size_class_for(min_size);
        let size = SIZE_CLASSES[class];

        if let Some(buf) = self.vertex_pool[class].pop() {
            self.in_use += size;
            return (buf, class);
        }

        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pooled_chunk_vertex_buffer"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.total_allocated += size;
        self.in_use += size;
        (buf, class)
    }

    /// Acquire an index buffer of at least `min_size` bytes.
    ///
    /// Returns a pooled buffer if available, or creates a new one.
    pub fn acquire_index_buffer(
        &mut self,
        device: &wgpu::Device,
        min_size: u64,
    ) -> (wgpu::Buffer, usize) {
        let class = self.size_class_for(min_size);
        let size = SIZE_CLASSES[class];

        if let Some(buf) = self.index_pool[class].pop() {
            self.in_use += size;
            return (buf, class);
        }

        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pooled_chunk_index_buffer"),
            size,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.total_allocated += size;
        self.in_use += size;
        (buf, class)
    }

    /// Return a vertex buffer to the pool for reuse.
    pub fn release_vertex_buffer(&mut self, buffer: wgpu::Buffer, size_class: usize) {
        let class = size_class.min(NUM_SIZE_CLASSES - 1);
        self.in_use = self.in_use.saturating_sub(SIZE_CLASSES[class]);
        self.vertex_pool[class].push(buffer);
    }

    /// Return an index buffer to the pool for reuse.
    pub fn release_index_buffer(&mut self, buffer: wgpu::Buffer, size_class: usize) {
        let class = size_class.min(NUM_SIZE_CLASSES - 1);
        self.in_use = self.in_use.saturating_sub(SIZE_CLASSES[class]);
        self.index_pool[class].push(buffer);
    }

    /// Current GPU memory in use by active chunk meshes.
    pub fn gpu_memory_in_use(&self) -> u64 {
        self.in_use
    }

    /// Total GPU memory allocated (including pooled free buffers).
    pub fn gpu_memory_allocated(&self) -> u64 {
        self.total_allocated
    }

    /// Number of free vertex buffers across all size classes.
    pub fn free_vertex_buffer_count(&self) -> usize {
        self.vertex_pool.iter().map(Vec::len).sum()
    }

    /// Number of free index buffers across all size classes.
    pub fn free_index_buffer_count(&self) -> usize {
        self.index_pool.iter().map(Vec::len).sum()
    }
}

impl Default for GpuBufferPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
                .ok()?;
            adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .ok()
        })
    }

    #[test]
    fn test_size_class_selection() {
        let pool = GpuBufferPool::new();
        assert_eq!(pool.size_class_for(100), 0); // < 4KB → class 0
        assert_eq!(pool.size_class_for(4096), 0); // exactly 4KB
        assert_eq!(pool.size_class_for(4097), 1); // just over 4KB → 8KB class
        assert_eq!(pool.size_class_for(131072), 5); // exactly 128KB
        assert_eq!(pool.size_class_for(200_000), 5); // over max → last class
    }

    #[test]
    fn test_pool_reuses_freed_vertex_buffers() {
        let Some((device, _queue)) = test_device() else {
            return;
        };
        let mut pool = GpuBufferPool::new();

        let (buf, class) = pool.acquire_vertex_buffer(&device, 1000);
        let allocated_after_first = pool.gpu_memory_allocated();

        pool.release_vertex_buffer(buf, class);
        assert_eq!(pool.free_vertex_buffer_count(), 1);

        let (_buf2, _class2) = pool.acquire_vertex_buffer(&device, 1000);
        let allocated_after_second = pool.gpu_memory_allocated();

        assert_eq!(
            allocated_after_first, allocated_after_second,
            "Pool should reuse buffer, not allocate new"
        );
        assert_eq!(pool.free_vertex_buffer_count(), 0);
    }

    #[test]
    fn test_pool_reuses_freed_index_buffers() {
        let Some((device, _queue)) = test_device() else {
            return;
        };
        let mut pool = GpuBufferPool::new();

        let (buf, class) = pool.acquire_index_buffer(&device, 2000);
        pool.release_index_buffer(buf, class);

        let allocated_before = pool.gpu_memory_allocated();
        let (_buf2, _) = pool.acquire_index_buffer(&device, 2000);
        assert_eq!(pool.gpu_memory_allocated(), allocated_before);
    }

    #[test]
    fn test_gpu_memory_tracking() {
        let Some((device, _queue)) = test_device() else {
            return;
        };
        let mut pool = GpuBufferPool::new();

        assert_eq!(pool.gpu_memory_in_use(), 0);

        let (buf, class) = pool.acquire_vertex_buffer(&device, 2000);
        assert!(pool.gpu_memory_in_use() > 0);

        let in_use_after_acquire = pool.gpu_memory_in_use();
        pool.release_vertex_buffer(buf, class);

        assert!(
            pool.gpu_memory_in_use() < in_use_after_acquire,
            "Memory in use should decrease after release"
        );
    }

    #[test]
    fn test_different_size_classes_dont_mix() {
        let Some((device, _queue)) = test_device() else {
            return;
        };
        let mut pool = GpuBufferPool::new();

        // Acquire and release a small buffer (class 0 = 4KB)
        let (buf, class) = pool.acquire_vertex_buffer(&device, 100);
        assert_eq!(class, 0);
        pool.release_vertex_buffer(buf, class);

        // Acquire a larger buffer — should NOT reuse the class-0 buffer
        let allocated_before = pool.gpu_memory_allocated();
        let (_buf2, class2) = pool.acquire_vertex_buffer(&device, 5000);
        assert_eq!(class2, 1); // 8KB class
        assert!(pool.gpu_memory_allocated() > allocated_before);
    }
}
