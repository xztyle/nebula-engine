//! GPU-resident chunk mesh: holds wgpu buffer handles for vertex/index data.
//!
//! [`GpuChunkMesh`] wraps the GPU buffers produced by uploading a
//! [`PackedChunkMesh`](nebula_mesh::PackedChunkMesh) and exposes the
//! metadata needed to issue indexed draw calls.

use nebula_mesh::PackedChunkMesh;
use wgpu::util::DeviceExt;

/// A chunk mesh that has been uploaded to the GPU.
///
/// Holds wgpu buffer handles and the metadata needed to issue draw calls.
pub struct GpuChunkMesh {
    /// Vertex buffer on the GPU.
    pub vertex_buffer: wgpu::Buffer,
    /// Index buffer on the GPU.
    pub index_buffer: wgpu::Buffer,
    /// Number of indices (used in `draw_indexed`).
    pub index_count: u32,
    /// Number of vertices.
    pub vertex_count: u32,
    /// Size of the vertex buffer in bytes (for memory tracking).
    vertex_buffer_size: u64,
    /// Size of the index buffer in bytes (for memory tracking).
    index_buffer_size: u64,
}

impl GpuChunkMesh {
    /// Upload a [`PackedChunkMesh`] to the GPU, creating new buffers.
    pub fn upload(device: &wgpu::Device, mesh: &PackedChunkMesh) -> Self {
        let vertex_bytes = mesh.vertex_bytes();
        let index_bytes = mesh.index_bytes();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_vertex_buffer"),
            contents: vertex_bytes,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("chunk_index_buffer"),
            contents: index_bytes,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
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

    /// Re-upload mesh data into existing buffers if they fit, or create new ones.
    ///
    /// Returns `true` if the existing buffers were reused (write-on-remesh).
    pub fn reupload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &PackedChunkMesh,
    ) -> bool {
        let vertex_bytes = mesh.vertex_bytes();
        let index_bytes = mesh.index_bytes();

        let vb_fits = vertex_bytes.len() as u64 <= self.vertex_buffer_size;
        let ib_fits = index_bytes.len() as u64 <= self.index_buffer_size;

        if vb_fits && ib_fits {
            // Reuse existing buffers via queue.write_buffer
            queue.write_buffer(&self.vertex_buffer, 0, vertex_bytes);
            queue.write_buffer(&self.index_buffer, 0, index_bytes);
            self.vertex_count = mesh.vertices.len() as u32;
            self.index_count = mesh.indices.len() as u32;
            true
        } else {
            // Buffers too small — recreate
            *self = Self::upload(device, mesh);
            false
        }
    }

    /// Total GPU memory consumed by this mesh's buffers in bytes.
    pub fn total_gpu_bytes(&self) -> u64 {
        self.vertex_buffer_size + self.index_buffer_size
    }

    /// Size of the vertex buffer in bytes.
    pub fn vertex_buffer_size(&self) -> u64 {
        self.vertex_buffer_size
    }

    /// Size of the index buffer in bytes.
    pub fn index_buffer_size(&self) -> u64 {
        self.index_buffer_size
    }

    /// Bind this mesh's buffers to a render pass.
    pub fn bind<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
    }

    /// Issue an indexed draw call for this mesh.
    pub fn draw(&self, render_pass: &mut wgpu::RenderPass) {
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_mesh::{ChunkVertex, FaceDirection};

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

    fn make_quad_mesh(count: usize) -> PackedChunkMesh {
        let mut mesh = PackedChunkMesh::new();
        for i in 0..count {
            let x = (i % 32) as u8;
            mesh.push_quad(
                [
                    ChunkVertex::new([x, 0, 0], FaceDirection::PosY, 0, 1, [0, 0]),
                    ChunkVertex::new([x + 1, 0, 0], FaceDirection::PosY, 0, 1, [1, 0]),
                    ChunkVertex::new([x + 1, 0, 1], FaceDirection::PosY, 0, 1, [1, 1]),
                    ChunkVertex::new([x, 0, 1], FaceDirection::PosY, 0, 1, [0, 1]),
                ],
                false,
            );
        }
        mesh
    }

    #[test]
    fn test_upload_creates_valid_buffers() {
        let Some((device, _queue)) = test_device() else {
            return; // graceful skip when no GPU
        };
        let mesh = make_quad_mesh(1);
        let gpu_mesh = GpuChunkMesh::upload(&device, &mesh);

        assert_eq!(gpu_mesh.vertex_count, 4);
        assert_eq!(gpu_mesh.index_count, 6);
        assert_eq!(gpu_mesh.vertex_buffer_size, 4 * 12); // 4 vertices × 12 bytes
        assert_eq!(gpu_mesh.index_buffer_size, 6 * 4); // 6 indices × 4 bytes
    }

    #[test]
    fn test_buffer_size_matches_data() {
        let Some((device, _queue)) = test_device() else {
            return;
        };
        let mesh = make_quad_mesh(10);
        let gpu_mesh = GpuChunkMesh::upload(&device, &mesh);

        assert_eq!(gpu_mesh.vertex_buffer_size, (10 * 4 * 12) as u64);
        assert_eq!(gpu_mesh.index_buffer_size, (10 * 6 * 4) as u64);
        assert_eq!(
            gpu_mesh.total_gpu_bytes(),
            (10 * 4 * 12 + 10 * 6 * 4) as u64
        );
    }

    #[test]
    fn test_reupload_reuses_buffers_when_fits() {
        let Some((device, queue)) = test_device() else {
            return;
        };
        let big_mesh = make_quad_mesh(10);
        let mut gpu_mesh = GpuChunkMesh::upload(&device, &big_mesh);

        // Re-upload a smaller mesh — should reuse buffers
        let small_mesh = make_quad_mesh(5);
        let reused = gpu_mesh.reupload(&device, &queue, &small_mesh);

        assert!(reused, "smaller mesh should fit in existing buffers");
        assert_eq!(gpu_mesh.vertex_count, 20); // 5 quads × 4 verts
        assert_eq!(gpu_mesh.index_count, 30); // 5 quads × 6 indices
    }

    #[test]
    fn test_reupload_recreates_when_too_large() {
        let Some((device, queue)) = test_device() else {
            return;
        };
        let small_mesh = make_quad_mesh(1);
        let mut gpu_mesh = GpuChunkMesh::upload(&device, &small_mesh);

        let big_mesh = make_quad_mesh(10);
        let reused = gpu_mesh.reupload(&device, &queue, &big_mesh);

        assert!(!reused, "larger mesh should not fit");
        assert_eq!(gpu_mesh.vertex_count, 40);
        assert_eq!(gpu_mesh.index_count, 60);
    }
}
