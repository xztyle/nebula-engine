//! GPU rendering context resource with type erasure.

use bevy_ecs::prelude::*;

/// GPU rendering context. Wraps wgpu objects and frame state.
/// Written only by PreRender and Render stages.
///
/// The actual wgpu types (Device, Queue, Surface) are defined in
/// nebula-render. This resource type in nebula-ecs is a placeholder
/// that nebula-render will extend with the concrete GPU state.
/// The ECS crate defines the resource slot; the render crate fills it.
#[derive(Resource)]
pub struct RenderContext {
    /// Opaque handle to the GPU context. The concrete type is defined
    /// in nebula-render and stored as a type-erased box here to avoid
    /// a dependency cycle (nebula-ecs cannot depend on nebula-render).
    inner: Box<dyn std::any::Any + Send + Sync>,
}

impl RenderContext {
    /// Creates a new [`RenderContext`] wrapping the given concrete GPU context.
    pub fn new<T: Send + Sync + 'static>(context: T) -> Self {
        Self {
            inner: Box::new(context),
        }
    }

    /// Downcast to the concrete GPU context type.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }

    /// Downcast to the concrete GPU context type (mutable).
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.inner.downcast_mut::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_context_type_erasure() {
        struct MockGpuContext {
            device_name: String,
        }

        let ctx = RenderContext::new(MockGpuContext {
            device_name: "Test GPU".to_string(),
        });

        let gpu = ctx.get::<MockGpuContext>().unwrap();
        assert_eq!(gpu.device_name, "Test GPU");

        // Wrong type returns None
        assert!(ctx.get::<String>().is_none());
    }
}
