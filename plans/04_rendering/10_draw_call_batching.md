# Draw Call Batching

## Problem

Every draw call on the GPU has overhead: the driver must validate state, the GPU must flush pipelines on state changes, and the CPU must encode commands. In a voxel planet engine with thousands of visible chunks, each using the same material (pipeline + textures), issuing draw calls in arbitrary order causes excessive GPU state changes. For example, if chunks alternate between terrain and water materials, the GPU pipeline switches back and forth thousands of times per frame. Sorting draw calls so that all terrain chunks draw together, then all water chunks draw together, reduces pipeline switches from thousands to two. This is one of the most impactful performance optimizations in any rendering engine.

Additionally, many chunks share the same mesh but are drawn at different positions (instanced rendering). Without batching, each instance is a separate draw call. With batching, hundreds of instances of the same mesh can be drawn in a single `draw_indexed_indirect` or `draw_indexed` call with an instance range.

## Solution

### DrawCall

A single draw call description:

```rust
#[derive(Clone, Debug)]
pub struct DrawCall {
    /// Opaque key identifying the render pipeline.
    pub pipeline_id: u64,
    /// Opaque key identifying the material (bind group).
    pub material_id: u64,
    /// Reference to the mesh buffer (vertex + index).
    pub mesh_id: u64,
    /// Per-instance data index (e.g., transform buffer offset).
    pub instance_index: u32,
}
```

The IDs are opaque keys (hashes or indices) that enable sorting without holding wgpu references directly. A separate lookup table maps IDs to actual wgpu objects at draw time.

### DrawBatch

```rust
pub struct DrawBatch {
    draw_calls: Vec<DrawCall>,
    sorted: bool,
}

impl DrawBatch {
    pub fn new() -> Self {
        Self {
            draw_calls: Vec::new(),
            sorted: false,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            draw_calls: Vec::with_capacity(capacity),
            sorted: false,
        }
    }

    /// Add a draw call to the batch.
    pub fn push(&mut self, call: DrawCall) {
        self.draw_calls.push(call);
        self.sorted = false;
    }

    /// Sort draw calls to minimize state changes.
    /// Sort order: pipeline_id first, then material_id, then mesh_id.
    pub fn sort(&mut self) {
        self.draw_calls.sort_unstable_by(|a, b| {
            a.pipeline_id.cmp(&b.pipeline_id)
                .then(a.material_id.cmp(&b.material_id))
                .then(a.mesh_id.cmp(&b.mesh_id))
        });
        self.sorted = true;
    }

    /// Clear the batch for reuse next frame.
    pub fn clear(&mut self) {
        self.draw_calls.clear();
        self.sorted = false;
    }

    /// Number of draw calls in the batch.
    pub fn len(&self) -> usize {
        self.draw_calls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.draw_calls.is_empty()
    }

    /// Iterate over sorted draw calls, yielding groups with the same pipeline and material.
    pub fn groups(&self) -> DrawGroupIter<'_> { ... }
}
```

### DrawGroupIter

An iterator that yields groups of draw calls sharing the same pipeline and material:

```rust
pub struct DrawGroup<'a> {
    pub pipeline_id: u64,
    pub material_id: u64,
    pub calls: &'a [DrawCall],
}

pub struct DrawGroupIter<'a> {
    calls: &'a [DrawCall],
    cursor: usize,
}

impl<'a> Iterator for DrawGroupIter<'a> {
    type Item = DrawGroup<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.calls.len() {
            return None;
        }

        let start = self.cursor;
        let pipeline_id = self.calls[start].pipeline_id;
        let material_id = self.calls[start].material_id;

        // Advance cursor while pipeline and material match.
        while self.cursor < self.calls.len()
            && self.calls[self.cursor].pipeline_id == pipeline_id
            && self.calls[self.cursor].material_id == material_id
        {
            self.cursor += 1;
        }

        Some(DrawGroup {
            pipeline_id,
            material_id,
            calls: &self.calls[start..self.cursor],
        })
    }
}
```

### Instanced Drawing

Within a `DrawGroup`, draw calls that share the same `mesh_id` can be merged into a single instanced draw call. The `DrawGroup` provides a helper:

```rust
impl<'a> DrawGroup<'a> {
    /// Yield sub-groups of calls with the same mesh_id for instanced drawing.
    pub fn instanced_groups(&self) -> impl Iterator<Item = InstancedDraw<'a>> { ... }
}

pub struct InstancedDraw<'a> {
    pub mesh_id: u64,
    pub calls: &'a [DrawCall],
}

impl InstancedDraw<'_> {
    pub fn instance_count(&self) -> u32 {
        self.calls.len() as u32
    }
}
```

### Rendering Loop Integration

```rust
fn execute_draw_batch(
    render_pass: &mut wgpu::RenderPass,
    batch: &DrawBatch,
    resources: &RenderResources, // maps IDs to wgpu objects
) {
    let mut current_pipeline = u64::MAX;
    let mut current_material = u64::MAX;

    for group in batch.groups() {
        if group.pipeline_id != current_pipeline {
            render_pass.set_pipeline(resources.get_pipeline(group.pipeline_id));
            current_pipeline = group.pipeline_id;
        }
        if group.material_id != current_material {
            render_pass.set_bind_group(1, resources.get_material(group.material_id), &[]);
            current_material = group.material_id;
        }

        for instanced in group.instanced_groups() {
            let mesh = resources.get_mesh(instanced.mesh_id);
            mesh.bind(render_pass);
            render_pass.draw_indexed(
                0..mesh.index_count,
                0,
                0..instanced.instance_count(),
            );
        }
    }
}
```

### Per-Frame Lifecycle

The `DrawBatch` is reused across frames to avoid allocation:

1. `batch.clear()` — reset at frame start, keeping the allocated Vec capacity.
2. Frustum culling adds visible objects with `batch.push(...)`.
3. `batch.sort()` — sort before rendering.
4. `execute_draw_batch(...)` — issue sorted draw calls.

## Outcome

A `DrawBatch` that collects, sorts, and groups draw calls by pipeline and material, minimizing GPU state changes. Instanced drawing merges multiple draw calls for the same mesh into a single draw call with an instance range. The batch is reused across frames without reallocation. The `DrawGroupIter` provides a zero-allocation iteration pattern for the rendering loop.

## Demo Integration

**Demo crate:** `nebula-demo`

The 100 cubes are batched into fewer draw calls by pipeline and material. The console logs `Draw calls: 3 (was 42)`. No visual change, but rendering is more efficient.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Draw call execution (used by the rendering loop, not by DrawBatch itself) |

The `DrawBatch` and `DrawCall` types are pure CPU-side data structures with no GPU dependencies. They use only `std`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_call(pipeline: u64, material: u64, mesh: u64, instance: u32) -> DrawCall {
        DrawCall {
            pipeline_id: pipeline,
            material_id: material,
            mesh_id: mesh,
            instance_index: instance,
        }
    }

    #[test]
    fn test_empty_batch_produces_zero_draw_calls() {
        let batch = DrawBatch::new();
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
        assert_eq!(batch.groups().count(), 0);
    }

    #[test]
    fn test_batching_groups_same_pipeline_together() {
        let mut batch = DrawBatch::new();
        batch.push(make_call(1, 1, 1, 0));
        batch.push(make_call(2, 1, 2, 0)); // different pipeline
        batch.push(make_call(1, 1, 3, 0)); // same pipeline as first
        batch.sort();

        let groups: Vec<_> = batch.groups().collect();
        // After sorting: pipeline 1 calls grouped, then pipeline 2
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].pipeline_id, 1);
        assert_eq!(groups[0].calls.len(), 2);
        assert_eq!(groups[1].pipeline_id, 2);
        assert_eq!(groups[1].calls.len(), 1);
    }

    #[test]
    fn test_sort_order_is_pipeline_then_material() {
        let mut batch = DrawBatch::new();
        batch.push(make_call(2, 2, 1, 0));
        batch.push(make_call(1, 2, 2, 0));
        batch.push(make_call(1, 1, 3, 0));
        batch.push(make_call(2, 1, 4, 0));
        batch.sort();

        let groups: Vec<_> = batch.groups().collect();
        // Expected order: (1,1), (1,2), (2,1), (2,2)
        assert_eq!(groups.len(), 4);
        assert_eq!((groups[0].pipeline_id, groups[0].material_id), (1, 1));
        assert_eq!((groups[1].pipeline_id, groups[1].material_id), (1, 2));
        assert_eq!((groups[2].pipeline_id, groups[2].material_id), (2, 1));
        assert_eq!((groups[3].pipeline_id, groups[3].material_id), (2, 2));
    }

    #[test]
    fn test_instance_count_is_correct() {
        let mut batch = DrawBatch::new();
        // Three instances of the same mesh with the same pipeline and material
        batch.push(make_call(1, 1, 42, 0));
        batch.push(make_call(1, 1, 42, 1));
        batch.push(make_call(1, 1, 42, 2));
        batch.sort();

        let groups: Vec<_> = batch.groups().collect();
        assert_eq!(groups.len(), 1);

        let instanced: Vec<_> = groups[0].instanced_groups().collect();
        assert_eq!(instanced.len(), 1);
        assert_eq!(instanced[0].mesh_id, 42);
        assert_eq!(instanced[0].instance_count(), 3);
    }

    #[test]
    fn test_clear_resets_batch() {
        let mut batch = DrawBatch::new();
        batch.push(make_call(1, 1, 1, 0));
        batch.push(make_call(2, 2, 2, 0));
        assert_eq!(batch.len(), 2);

        batch.clear();
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
    }

    #[test]
    fn test_single_draw_call_produces_one_group() {
        let mut batch = DrawBatch::new();
        batch.push(make_call(5, 10, 20, 0));
        batch.sort();

        let groups: Vec<_> = batch.groups().collect();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].pipeline_id, 5);
        assert_eq!(groups[0].material_id, 10);
        assert_eq!(groups[0].calls.len(), 1);
    }

    #[test]
    fn test_different_meshes_same_pipeline_material_form_one_group() {
        let mut batch = DrawBatch::new();
        batch.push(make_call(1, 1, 100, 0)); // mesh 100
        batch.push(make_call(1, 1, 200, 0)); // mesh 200, same pipeline/material
        batch.sort();

        let groups: Vec<_> = batch.groups().collect();
        assert_eq!(groups.len(), 1); // one group (same pipeline + material)

        let instanced: Vec<_> = groups[0].instanced_groups().collect();
        assert_eq!(instanced.len(), 2); // but two instanced draws (different meshes)
    }

    #[test]
    fn test_with_capacity_preallocates() {
        let batch = DrawBatch::with_capacity(1000);
        assert!(batch.is_empty());
        // The internal vec should have capacity >= 1000
        // (not directly testable, but ensures no realloc for first 1000 pushes)
    }
}
```
