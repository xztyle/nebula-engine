//! wgpu rendering pipeline: surface management, render passes, shader loading, and frame graph orchestration.

pub mod batching;
pub mod bloom;
pub mod buffer;
pub mod camera;
pub mod depth;
pub mod frustum;
pub mod gpu;
pub mod gpu_buffer_pool;
pub mod gpu_chunk_mesh;
pub mod lens_flare;
pub mod lit_pipeline;
pub mod pass;
pub mod pipeline;
pub mod shader;
pub mod surface;
pub mod texture;
pub mod textured_pipeline;

pub use batching::{
    DrawBatch, DrawCall, DrawGroup, DrawGroupIter, InstancedDraw, InstancedGroupIter,
};
pub use bloom::{BloomConfig, BloomPipeline};
pub use lens_flare::{FlareElement, FlareShape, LensFlareConfig, LensFlareRenderer};
// Re-export the main types from the plan
pub use buffer::{
    BufferAllocator, IndexData, MeshBuffer, VertexPositionColor, VertexPositionNormalUv,
};
pub use camera::{Camera, Projection};
pub use depth::DepthBuffer;
pub use frustum::{Aabb, Frustum, FrustumCuller};
pub use gpu::{RenderContext, RenderContextError, SurfaceError, init_render_context_blocking};
pub use gpu_buffer_pool::GpuBufferPool;
pub use gpu_chunk_mesh::GpuChunkMesh;
pub use lit_pipeline::{LIT_SHADER_SOURCE, LitPipeline, draw_lit};
pub use pass::{DepthAttachmentConfig, FrameEncoder, RenderPassBuilder, SKY_BLUE};
pub use pipeline::{CameraUniform, UNLIT_SHADER_SOURCE, UnlitPipeline, draw_unlit};
pub use shader::{ShaderError, ShaderLibrary};
pub use surface::{MIN_SURFACE_DIMENSION, PhysicalSize, SurfaceResizeEvent, SurfaceWrapper};
pub use texture::{
    ManagedTexture, TextureError, TextureLayerData, TextureManager, mip_level_count,
};
pub use textured_pipeline::{TEXTURED_SHADER_SOURCE, TexturedPipeline, draw_textured};
