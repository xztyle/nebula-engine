//! wgpu rendering pipeline: surface management, render passes, shader loading, and frame graph orchestration.

pub mod gpu;

// Re-export the main types from the plan
pub use gpu::{RenderContext, RenderContextError, SurfaceError, init_render_context_blocking};
