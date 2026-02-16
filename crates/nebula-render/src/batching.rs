//! Draw call batching: collect, sort, and group draw calls to minimize GPU state changes.
//!
//! Draw calls are sorted by pipeline, then material, then mesh ID. Groups of calls
//! sharing the same pipeline and material are yielded together, and within each group,
//! calls sharing the same mesh can be drawn as instanced calls.

/// A single draw call description with opaque resource keys.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// A batch of draw calls that can be sorted and grouped for efficient rendering.
pub struct DrawBatch {
    draw_calls: Vec<DrawCall>,
    sorted: bool,
}

impl Default for DrawBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl DrawBatch {
    /// Create a new empty batch.
    pub fn new() -> Self {
        Self {
            draw_calls: Vec::new(),
            sorted: false,
        }
    }

    /// Create a new batch with preallocated capacity.
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
            a.pipeline_id
                .cmp(&b.pipeline_id)
                .then(a.material_id.cmp(&b.material_id))
                .then(a.mesh_id.cmp(&b.mesh_id))
        });
        self.sorted = true;
    }

    /// Clear the batch for reuse next frame, keeping allocated capacity.
    pub fn clear(&mut self) {
        self.draw_calls.clear();
        self.sorted = false;
    }

    /// Number of draw calls in the batch.
    pub fn len(&self) -> usize {
        self.draw_calls.len()
    }

    /// Whether the batch contains no draw calls.
    pub fn is_empty(&self) -> bool {
        self.draw_calls.is_empty()
    }

    /// Whether the batch has been sorted since the last modification.
    pub fn is_sorted(&self) -> bool {
        self.sorted
    }

    /// Iterate over sorted draw calls, yielding groups with the same pipeline and material.
    ///
    /// For correct grouping, call [`sort`](Self::sort) before calling this method.
    pub fn groups(&self) -> DrawGroupIter<'_> {
        DrawGroupIter {
            calls: &self.draw_calls,
            cursor: 0,
        }
    }
}

/// A group of draw calls sharing the same pipeline and material.
#[derive(Debug)]
pub struct DrawGroup<'a> {
    /// The pipeline ID shared by all calls in this group.
    pub pipeline_id: u64,
    /// The material ID shared by all calls in this group.
    pub material_id: u64,
    /// The draw calls in this group.
    pub calls: &'a [DrawCall],
}

impl<'a> DrawGroup<'a> {
    /// Yield sub-groups of calls with the same `mesh_id` for instanced drawing.
    ///
    /// Assumes the parent batch was sorted (so calls with the same mesh_id are contiguous).
    pub fn instanced_groups(&self) -> InstancedGroupIter<'a> {
        InstancedGroupIter {
            calls: self.calls,
            cursor: 0,
        }
    }
}

/// Iterator over [`DrawGroup`]s within a [`DrawBatch`].
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

/// A sub-group of draw calls sharing the same mesh, suitable for instanced drawing.
#[derive(Debug)]
pub struct InstancedDraw<'a> {
    /// The mesh ID shared by all calls in this sub-group.
    pub mesh_id: u64,
    /// The draw calls in this sub-group.
    pub calls: &'a [DrawCall],
}

impl InstancedDraw<'_> {
    /// Number of instances to draw.
    pub fn instance_count(&self) -> u32 {
        self.calls.len() as u32
    }
}

/// Iterator over [`InstancedDraw`] sub-groups within a [`DrawGroup`].
pub struct InstancedGroupIter<'a> {
    calls: &'a [DrawCall],
    cursor: usize,
}

impl<'a> Iterator for InstancedGroupIter<'a> {
    type Item = InstancedDraw<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.calls.len() {
            return None;
        }

        let start = self.cursor;
        let mesh_id = self.calls[start].mesh_id;

        while self.cursor < self.calls.len() && self.calls[self.cursor].mesh_id == mesh_id {
            self.cursor += 1;
        }

        Some(InstancedDraw {
            mesh_id,
            calls: &self.calls[start..self.cursor],
        })
    }
}

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
        batch.push(make_call(2, 1, 2, 0));
        batch.push(make_call(1, 1, 3, 0));
        batch.sort();

        let groups: Vec<_> = batch.groups().collect();
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
        assert_eq!(groups.len(), 4);
        assert_eq!((groups[0].pipeline_id, groups[0].material_id), (1, 1));
        assert_eq!((groups[1].pipeline_id, groups[1].material_id), (1, 2));
        assert_eq!((groups[2].pipeline_id, groups[2].material_id), (2, 1));
        assert_eq!((groups[3].pipeline_id, groups[3].material_id), (2, 2));
    }

    #[test]
    fn test_instance_count_is_correct() {
        let mut batch = DrawBatch::new();
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
        batch.push(make_call(1, 1, 100, 0));
        batch.push(make_call(1, 1, 200, 0));
        batch.sort();

        let groups: Vec<_> = batch.groups().collect();
        assert_eq!(groups.len(), 1);

        let instanced: Vec<_> = groups[0].instanced_groups().collect();
        assert_eq!(instanced.len(), 2);
    }

    #[test]
    fn test_with_capacity_preallocates() {
        let batch = DrawBatch::with_capacity(1000);
        assert!(batch.is_empty());
    }
}
