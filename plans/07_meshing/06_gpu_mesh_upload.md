# GPU Mesh Upload

## Problem

The meshing pipeline produces `ChunkMesh` instances on the CPU (story 05), but rendering requires the vertex and index data to live in GPU-accessible buffers. Each visible chunk needs its own pair of wgpu vertex and index buffers. Naively creating and destroying GPU buffers every time a chunk is meshed or remeshed causes allocation churn: `wgpu::Device::create_buffer` is not free — it involves driver-level memory allocation, and frequent small allocations fragment GPU memory. With thousands of chunks being loaded, remeshed (due to edits), and unloaded per second, the upload path must be efficient and reuse resources.

Additionally, GPU memory is a finite resource. Without tracking, the engine has no way to know how much VRAM is consumed by chunk meshes, making it impossible to enforce memory budgets or trigger LOD transitions to shed geometry.

## Solution

Implement a GPU mesh upload system and buffer pool in the `nebula_meshing` crate (or `nebula_render`, depending on crate boundaries), managing the lifecycle of GPU-resident chunk meshes.

### GpuChunkMesh

```rust
/// A chunk mesh that has been uploaded to the GPU.
/// Holds wgpu buffer handles and the metadata needed to issue draw calls.
pub struct GpuChunkMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub vertex_count: u32,
    /// Size of the vertex buffer in bytes (for memory tracking).
    vertex_buffer_size: u64,
    /// Size of the index buffer in bytes (for memory tracking).
    index_buffer_size: u64,
}

impl GpuChunkMesh {
    pub fn total_gpu_bytes(&self) -> u64 {
        self.vertex_buffer_size + self.index_buffer_size
    }
}
```

### Upload Path

```rust
impl GpuChunkMesh {
    /// Upload a ChunkMesh to the GPU, creating new buffers.
    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, mesh: &ChunkMesh) -> Self {
        let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
        let index_bytes: &[u8] = bytemuck::cast_slice(&mesh.indices);

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_vertex_buffer"),
            contents: vertex_bytes,
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_index_buffer"),
            contents: index_bytes,
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            vertex_count: mesh.vertices.len() as u32,
            vertex_buffer_size: vertex_bytes.len() as u64,
            index_buffer_size: index_bytes.len() as u64,
        }
    }
}
```

### Buffer Pool

To avoid per-chunk allocation overhead, maintain a pool of pre-allocated GPU buffers in common sizes. When a chunk mesh is uploaded, find a buffer from the pool that is large enough. When a chunk mesh is freed, return the buffer to the pool instead of destroying it.

```rust
pub struct GpuBufferPool {
    /// Free vertex buffers, bucketed by size class.
    vertex_pool: Vec<Vec<wgpu::Buffer>>,
    /// Free index buffers, bucketed by size class.
    index_pool: Vec<Vec<wgpu::Buffer>>,
    /// Size class thresholds in bytes (e.g., 4KB, 8KB, 16KB, 32KB, 64KB).
    size_classes: Vec<u64>,
    /// Total bytes currently allocated (in use + pooled).
    total_allocated: u64,
    /// Total bytes currently in use (uploaded, not pooled).
    in_use: u64,
}

impl GpuBufferPool {
    pub fn new() -> Self {
        Self {
            vertex_pool: vec![Vec::new(); 6],
            index_pool: vec![Vec::new(); 6],
            size_classes: vec![4096, 8192, 16384, 32768, 65536, 131072],
            total_allocated: 0,
            in_use: 0,
        }
    }

    /// Acquire a vertex buffer of at least `min_size` bytes.
    /// Returns a pooled buffer if available, or creates a new one.
    pub fn acquire_vertex_buffer(
        &mut self,
        device: &wgpu::Device,
        min_size: u64,
    ) -> wgpu::Buffer {
        let class = self.size_class_for(min_size);
        if let Some(buf) = self.vertex_pool[class].pop() {
            self.in_use += self.size_classes[class];
            return buf;
        }
        let size = self.size_classes[class];
        self.total_allocated += size;
        self.in_use += size;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pooled_chunk_vertex_buffer"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Return a buffer to the pool for reuse.
    pub fn release_vertex_buffer(&mut self, buffer: wgpu::Buffer, size_class: usize) {
        self.in_use -= self.size_classes[size_class];
        self.vertex_pool[size_class].push(buffer);
    }

    /// Current GPU memory in use by chunk meshes.
    pub fn gpu_memory_in_use(&self) -> u64 {
        self.in_use
    }

    /// Total GPU memory allocated (including pooled free buffers).
    pub fn gpu_memory_allocated(&self) -> u64 {
        self.total_allocated
    }
}
```

### Write-on-Remesh

When a chunk is remeshed, the new mesh data is written into the existing buffer using `queue.write_buffer()` if the new data fits. If the new mesh is larger, the old buffer is returned to the pool and a larger one is acquired. This avoids destroying and recreating buffers on every block edit.

### Memory Tracking

The pool tracks `total_allocated` (all GPU memory ever allocated for chunk meshes) and `in_use` (memory currently holding active mesh data). The engine can query these values to enforce memory budgets, trigger LOD increases for distant chunks, or display GPU memory usage in a debug overlay.

## Outcome

The `nebula_meshing` (or `nebula_render`) crate exports `GpuChunkMesh` and `GpuBufferPool`. Chunk meshes are uploaded with a single call, GPU buffers are reused via the pool, and memory usage is tracked. The render loop draws each chunk by binding its `GpuChunkMesh` vertex and index buffers and issuing an indexed draw call. Running `cargo test -p nebula_meshing` passes all GPU upload and pool tests (using wgpu's headless/software backend for CI).

## Demo Integration

**Demo crate:** `nebula-demo`

Terrain meshes are uploaded to the GPU and rendered from GPU-resident buffers. Frame time drops noticeably compared to CPU-side rendering.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | GPU buffer creation, queue writes, buffer descriptors |
| `bytemuck` | `1.21` | Zero-copy cast `&[ChunkVertex]` / `&[u32]` to `&[u8]` for upload |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::util::DeviceExt;

    /// Helper: create a test device and queue using wgpu's software backend.
    fn test_device() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: true,
            compatible_surface: None,
        }))
        .expect("no adapter");
        pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor::default(),
            None,
        ))
        .expect("no device")
    }

    /// Uploading a non-empty mesh should create buffers with the correct sizes.
    #[test]
    fn test_upload_creates_valid_buffers() {
        let (device, queue) = test_device();
        let mut mesh = ChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new([0, 0, 0], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([1, 0, 0], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([1, 0, 1], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([0, 0, 1], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );

        let gpu_mesh = GpuChunkMesh::upload(&device, &queue, &mesh);

        assert_eq!(gpu_mesh.vertex_count, 4);
        assert_eq!(gpu_mesh.index_count, 6);
        assert_eq!(gpu_mesh.vertex_buffer_size, 4 * 12); // 4 vertices * 12 bytes
        assert_eq!(gpu_mesh.index_buffer_size, 6 * 4);   // 6 indices * 4 bytes
    }

    /// Buffer size should match the vertex and index data exactly.
    #[test]
    fn test_buffer_size_matches_data() {
        let (device, queue) = test_device();
        let mut mesh = ChunkMesh::new();
        // Add 10 quads
        for i in 0..10u8 {
            mesh.push_quad(
                [
                    ChunkVertex::new([i, 0, 0], FaceDirection::PosY, 0, 1, [0, 0]),
                    ChunkVertex::new([i + 1, 0, 0], FaceDirection::PosY, 0, 1, [1, 0]),
                    ChunkVertex::new([i + 1, 0, 1], FaceDirection::PosY, 0, 1, [1, 1]),
                    ChunkVertex::new([i, 0, 1], FaceDirection::PosY, 0, 1, [0, 1]),
                ],
                false,
            );
        }

        let gpu_mesh = GpuChunkMesh::upload(&device, &queue, &mesh);

        assert_eq!(gpu_mesh.vertex_buffer_size, (10 * 4 * 12) as u64);
        assert_eq!(gpu_mesh.index_buffer_size, (10 * 6 * 4) as u64);
        assert_eq!(gpu_mesh.total_gpu_bytes(), (10 * 4 * 12 + 10 * 6 * 4) as u64);
    }

    /// The buffer pool should reuse freed buffers instead of allocating new ones.
    #[test]
    fn test_pool_reuses_freed_buffers() {
        let (device, _queue) = test_device();
        let mut pool = GpuBufferPool::new();

        // Acquire a buffer
        let buf1 = pool.acquire_vertex_buffer(&device, 1000);
        let allocated_after_first = pool.gpu_memory_allocated();

        // Release it
        pool.release_vertex_buffer(buf1, pool.size_class_for(1000));

        // Acquire again — should reuse, no new allocation
        let _buf2 = pool.acquire_vertex_buffer(&device, 1000);
        let allocated_after_second = pool.gpu_memory_allocated();

        assert_eq!(
            allocated_after_first, allocated_after_second,
            "Pool should reuse buffer, not allocate new"
        );
    }

    /// GPU memory tracking should increase on upload and decrease on free.
    #[test]
    fn test_gpu_memory_tracking() {
        let (device, _queue) = test_device();
        let mut pool = GpuBufferPool::new();

        assert_eq!(pool.gpu_memory_in_use(), 0);

        let buf = pool.acquire_vertex_buffer(&device, 2000);
        assert!(pool.gpu_memory_in_use() > 0);

        let in_use_after_acquire = pool.gpu_memory_in_use();
        pool.release_vertex_buffer(buf, pool.size_class_for(2000));

        assert!(
            pool.gpu_memory_in_use() < in_use_after_acquire,
            "Memory in use should decrease after release"
        );
    }
}
```
