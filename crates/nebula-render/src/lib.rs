//! wgpu rendering pipeline: surface management, render passes, shader loading, and frame graph orchestration.

pub mod buffer;
pub mod gpu;
pub mod pass;
pub mod pipeline;
pub mod shader;

// Re-export the main types from the plan
pub use buffer::{
    BufferAllocator, IndexData, MeshBuffer, VertexPositionColor, VertexPositionNormalUv,
};
pub use gpu::{RenderContext, RenderContextError, SurfaceError, init_render_context_blocking};
pub use pass::{DepthAttachmentConfig, FrameEncoder, RenderPassBuilder, SKY_BLUE};
pub use pipeline::{CameraUniform, UNLIT_SHADER_SOURCE, UnlitPipeline, draw_unlit};
pub use shader::{ShaderError, ShaderLibrary};
