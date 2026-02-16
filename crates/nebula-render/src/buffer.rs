//! Vertex and index buffer management for GPU rendering.

use bytemuck::{Pod, Zeroable};

/// A complete mesh buffer containing vertex and index data ready for GPU rendering.
pub struct MeshBuffer {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub index_format: wgpu::IndexFormat,
}

impl MeshBuffer {
    /// Bind vertex and index buffers to a render pass.
    pub fn bind<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), self.index_format);
    }

    /// Draw the entire mesh using indexed rendering.
    pub fn draw(&self, render_pass: &mut wgpu::RenderPass) {
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

/// Index data that can be either u16 or u32 format.
pub enum IndexData<'a> {
    U16(&'a [u16]),
    U32(&'a [u32]),
}

impl IndexData<'_> {
    /// Get the appropriate wgpu index format for this data.
    pub fn format(&self) -> wgpu::IndexFormat {
        match self {
            IndexData::U16(_) => wgpu::IndexFormat::Uint16,
            IndexData::U32(_) => wgpu::IndexFormat::Uint32,
        }
    }

    /// Get the number of indices.
    pub fn count(&self) -> u32 {
        match self {
            IndexData::U16(data) => data.len() as u32,
            IndexData::U32(data) => data.len() as u32,
        }
    }

    /// Get the raw byte slice for buffer creation.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            IndexData::U16(data) => bytemuck::cast_slice(data),
            IndexData::U32(data) => bytemuck::cast_slice(data),
        }
    }
}

/// GPU buffer allocator for creating vertex and index buffers.
pub struct BufferAllocator<'a> {
    device: &'a wgpu::Device,
}

impl<'a> BufferAllocator<'a> {
    /// Create a new buffer allocator with the given device.
    pub fn new(device: &'a wgpu::Device) -> Self {
        Self { device }
    }

    /// Create a complete mesh buffer from vertex and index data.
    pub fn create_mesh(&self, label: &str, vertices: &[u8], indices: IndexData) -> MeshBuffer {
        let vertex_buffer = self.create_vertex_buffer(&format!("{}-vertices", label), vertices);

        let (index_buffer, index_format) = match indices {
            IndexData::U16(data) => {
                let buffer = self.create_index_buffer_u16(&format!("{}-indices", label), data);
                (buffer, wgpu::IndexFormat::Uint16)
            }
            IndexData::U32(data) => {
                let buffer = self.create_index_buffer_u32(&format!("{}-indices", label), data);
                (buffer, wgpu::IndexFormat::Uint32)
            }
        };

        MeshBuffer {
            vertex_buffer,
            index_buffer,
            index_count: indices.count(),
            index_format,
        }
    }

    /// Create a vertex buffer from raw byte data.
    pub fn create_vertex_buffer(&self, label: &str, data: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;

        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: data,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
    }

    /// Create a u16 index buffer.
    pub fn create_index_buffer_u16(&self, label: &str, data: &[u16]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;

        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            })
    }

    /// Create a u32 index buffer.
    pub fn create_index_buffer_u32(&self, label: &str, data: &[u32]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;

        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            })
    }
}

/// Standard vertex format with position and color.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct VertexPositionColor {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl VertexPositionColor {
    /// Get the vertex buffer layout for this vertex type.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        use wgpu::{VertexAttribute, VertexFormat};

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<VertexPositionColor>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: VertexFormat::Float32x3,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: VertexFormat::Float32x4,
                },
            ],
        }
    }
}

/// Standard vertex format with position, normal, and UV coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct VertexPositionNormalUv {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl VertexPositionNormalUv {
    /// Get the vertex buffer layout for this vertex type.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        use wgpu::{VertexAttribute, VertexFormat};

        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<VertexPositionNormalUv>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: VertexFormat::Float32x3,
                },
                VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: VertexFormat::Float32x3,
                },
                VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 3]>() * 2) as wgpu::BufferAddress,
                    shader_location: 2,
                    format: VertexFormat::Float32x2,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
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
                .request_device(&wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    experimental_features: Default::default(),
                    ..Default::default()
                })
                .await
                .ok()
        })
    }

    #[test]
    fn test_mesh_buffer_creation_u16() {
        let Some((device, _queue)) = create_test_device() else {
            return;
        };
        let allocator = BufferAllocator::new(&device);

        let vertices: &[VertexPositionColor] = &[
            VertexPositionColor {
                position: [0.0, 0.0, 0.0],
                color: [1.0; 4],
            },
            VertexPositionColor {
                position: [1.0, 0.0, 0.0],
                color: [1.0; 4],
            },
            VertexPositionColor {
                position: [0.0, 1.0, 0.0],
                color: [1.0; 4],
            },
        ];
        let indices: &[u16] = &[0, 1, 2];

        let mesh = allocator.create_mesh(
            "test-triangle",
            bytemuck::cast_slice(vertices),
            IndexData::U16(indices),
        );

        assert_eq!(mesh.index_count, 3);
        assert_eq!(mesh.index_format, wgpu::IndexFormat::Uint16);
    }

    #[test]
    fn test_mesh_buffer_creation_u32() {
        let Some((device, _queue)) = create_test_device() else {
            return;
        };
        let allocator = BufferAllocator::new(&device);

        let vertices = vec![0u8; 128]; // raw vertex data
        let indices: &[u32] = &[0, 1, 2, 2, 3, 0];

        let mesh = allocator.create_mesh("test-quad", &vertices, IndexData::U32(indices));

        assert_eq!(mesh.index_count, 6);
        assert_eq!(mesh.index_format, wgpu::IndexFormat::Uint32);
    }

    #[test]
    fn test_index_count_matches_input() {
        let Some((device, _queue)) = create_test_device() else {
            return;
        };
        let allocator = BufferAllocator::new(&device);
        let indices: &[u16] = &[0, 1, 2, 3, 4, 5, 6, 7, 8]; // 3 triangles

        let mesh = allocator.create_mesh("test", &[0u8; 64], IndexData::U16(indices));

        assert_eq!(mesh.index_count, 9);
    }

    #[test]
    fn test_u16_vs_u32_format_selection() {
        let u16_data = IndexData::U16(&[0, 1, 2]);
        let u32_data = IndexData::U32(&[0, 1, 2]);

        assert_eq!(u16_data.format(), wgpu::IndexFormat::Uint16);
        assert_eq!(u32_data.format(), wgpu::IndexFormat::Uint32);
    }

    #[test]
    fn test_empty_mesh_creates_zero_index_count() {
        let Some((device, _queue)) = create_test_device() else {
            return;
        };
        let allocator = BufferAllocator::new(&device);

        let mesh = allocator.create_mesh("empty", &[], IndexData::U16(&[]));

        assert_eq!(mesh.index_count, 0);
    }

    #[test]
    fn test_index_data_as_bytes() {
        let indices_u16: &[u16] = &[0, 1, 2];
        let data = IndexData::U16(indices_u16);
        assert_eq!(data.as_bytes().len(), 6); // 3 × 2 bytes

        let indices_u32: &[u32] = &[0, 1, 2];
        let data = IndexData::U32(indices_u32);
        assert_eq!(data.as_bytes().len(), 12); // 3 × 4 bytes
    }

    #[test]
    fn test_vertex_position_color_layout() {
        let layout = VertexPositionColor::layout();
        // position (f32×3) + color (f32×4) = 28 bytes stride
        assert_eq!(layout.array_stride, 28);
        assert_eq!(layout.attributes.len(), 2);
    }

    #[test]
    fn test_vertex_position_normal_uv_layout() {
        let layout = VertexPositionNormalUv::layout();
        // position (f32×3) + normal (f32×3) + uv (f32×2) = 32 bytes stride
        assert_eq!(layout.array_stride, 32);
        assert_eq!(layout.attributes.len(), 3);
    }
}
