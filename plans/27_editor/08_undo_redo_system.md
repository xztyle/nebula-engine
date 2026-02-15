# Undo/Redo System

## Problem

Every editor operation is potentially destructive — placing a voxel overwrites the previous one, moving an entity changes its position, deleting an entity removes it entirely. Without undo, a single misclick can cost minutes or hours of manual work, and designers become afraid to experiment. Redo is equally important: undoing too far should not lose the work that was undone. Together, undo and redo form the safety net that makes creative experimentation viable.

The system must handle heterogeneous operations: voxel batch edits (which may touch thousands of cells), entity transform changes, entity spawn/despawn, property edits in the inspector, terrain brush strokes, and reparenting in the hierarchy. Each operation type has different storage requirements — voxel changes need old and new type IDs for every affected cell, while a transform change only needs the old and new position/rotation/scale. The undo stack must have a configurable depth limit to prevent unbounded memory growth during long editing sessions.

A naive implementation that clones entire world snapshots would consume gigabytes of memory. Instead, each operation must store only the minimal inverse delta needed to reverse it.

## Solution

Implement an `UndoStack` resource and `UndoAction` enum in the `nebula_editor` crate.

### Action Types

```rust
use bevy_ecs::prelude::*;

#[derive(Clone, Debug)]
pub enum UndoAction {
    /// A batch of voxel type changes. Each entry stores the coordinate,
    /// the old voxel type, and the new voxel type.
    VoxelBatch(VoxelBatchOp),

    /// A change to an entity's transform (position, rotation, scale).
    TransformChange {
        entity: Entity,
        old: TransformSnapshot,
        new: TransformSnapshot,
    },

    /// An entity was spawned. Undoing despawns it; redoing respawns it.
    SpawnEntity {
        entity: Entity,
        /// Serialized component bundle for respawning on redo.
        components: SerializedBundle,
    },

    /// An entity was despawned. Undoing respawns it; redoing despawns it.
    DespawnEntity {
        entity: Entity,
        /// Serialized component bundle for respawning on undo.
        components: SerializedBundle,
    },

    /// A single component property was edited in the inspector.
    PropertyEdit {
        entity: Entity,
        component_name: String,
        field_name: String,
        old_value: PropertyValue,
        new_value: PropertyValue,
    },

    /// An entity was reparented in the hierarchy.
    Reparent {
        entity: Entity,
        old_parent: Option<Entity>,
        new_parent: Option<Entity>,
    },

    /// A group of actions that should be undone/redone as a single unit.
    Group(Vec<UndoAction>),
}
```

### Voxel Batch Storage

```rust
#[derive(Clone, Debug)]
pub struct VoxelBatchOp {
    pub entries: Vec<VoxelChangeEntry>,
}

#[derive(Clone, Debug)]
pub struct VoxelChangeEntry {
    pub coord: VoxelCoord,
    pub old_type: VoxelTypeId,
    pub new_type: VoxelTypeId,
}

impl VoxelBatchOp {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn push(&mut self, coord: VoxelCoord, old: VoxelTypeId, new: VoxelTypeId) {
        self.entries.push(VoxelChangeEntry {
            coord,
            old_type: old,
            new_type: new,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
```

### Property Value Storage

```rust
#[derive(Clone, Debug)]
pub enum PropertyValue {
    F32(f32),
    F64(f64),
    I32(i32),
    I64(i64),
    I128(i128),
    Bool(bool),
    String(String),
    Vec3(glam::Vec3),
    Quat(glam::Quat),
}
```

### Undo Stack

```rust
#[derive(Resource)]
pub struct UndoStack {
    /// Actions that can be undone, most recent last.
    undo: Vec<UndoAction>,
    /// Actions that can be redone, most recent last.
    redo: Vec<UndoAction>,
    /// Maximum number of actions stored in the undo stack.
    pub max_depth: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            max_depth: 100,
        }
    }
}

impl UndoStack {
    pub fn push(&mut self, action: UndoAction) {
        self.undo.push(action);
        // Any new action invalidates the redo history
        self.redo.clear();
        // Enforce depth limit by removing the oldest action
        if self.undo.len() > self.max_depth {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn pop_undo(&mut self) -> Option<UndoAction> {
        let action = self.undo.pop()?;
        self.redo.push(action.clone());
        Some(action)
    }

    pub fn pop_redo(&mut self) -> Option<UndoAction> {
        let action = self.redo.pop()?;
        self.undo.push(action.clone());
        Some(action)
    }

    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
}
```

### Undo/Redo Systems

```rust
pub fn undo_system(
    keyboard: Res<KeyboardState>,
    mut undo_stack: ResMut<UndoStack>,
    mut chunks: ResMut<ChunkManager>,
    mut commands: Commands,
    mut transforms: Query<(&mut WorldPos, &mut Rotation, &mut Scale)>,
) {
    let ctrl = keyboard.is_pressed(PhysicalKey::Code(KeyCode::ControlLeft))
            || keyboard.is_pressed(PhysicalKey::Code(KeyCode::ControlRight));
    let shift = keyboard.is_pressed(PhysicalKey::Code(KeyCode::ShiftLeft))
             || keyboard.is_pressed(PhysicalKey::Code(KeyCode::ShiftRight));
    let z = keyboard.just_pressed(PhysicalKey::Code(KeyCode::KeyZ));

    if ctrl && !shift && z {
        if let Some(action) = undo_stack.pop_undo() {
            apply_inverse(&action, &mut chunks, &mut commands, &mut transforms);
        }
    }

    if ctrl && shift && z {
        if let Some(action) = undo_stack.pop_redo() {
            apply_forward(&action, &mut chunks, &mut commands, &mut transforms);
        }
    }
}

fn apply_inverse(
    action: &UndoAction,
    chunks: &mut ChunkManager,
    commands: &mut Commands,
    transforms: &mut Query<(&mut WorldPos, &mut Rotation, &mut Scale)>,
) {
    match action {
        UndoAction::VoxelBatch(batch) => {
            for entry in &batch.entries {
                chunks.set_voxel(entry.coord, entry.old_type);
            }
        }
        UndoAction::TransformChange { entity, old, .. } => {
            if let Ok((mut pos, mut rot, mut scl)) = transforms.get_mut(*entity) {
                *pos = old.position;
                rot.0 = old.rotation;
                scl.0 = old.scale;
            }
        }
        UndoAction::SpawnEntity { entity, .. } => {
            commands.entity(*entity).despawn();
        }
        UndoAction::DespawnEntity { entity, components, .. } => {
            respawn_entity(commands, *entity, components);
        }
        UndoAction::PropertyEdit { entity, old_value, .. } => {
            apply_property_value(transforms, *entity, old_value);
        }
        UndoAction::Reparent { entity, old_parent, .. } => {
            match old_parent {
                Some(parent) => commands.entity(*entity).set_parent(*parent),
                None => commands.entity(*entity).remove_parent(),
            };
        }
        UndoAction::Group(actions) => {
            for action in actions.iter().rev() {
                apply_inverse(action, chunks, commands, transforms);
            }
        }
    }
}

fn apply_forward(
    action: &UndoAction,
    chunks: &mut ChunkManager,
    commands: &mut Commands,
    transforms: &mut Query<(&mut WorldPos, &mut Rotation, &mut Scale)>,
) {
    match action {
        UndoAction::VoxelBatch(batch) => {
            for entry in &batch.entries {
                chunks.set_voxel(entry.coord, entry.new_type);
            }
        }
        UndoAction::TransformChange { entity, new, .. } => {
            if let Ok((mut pos, mut rot, mut scl)) = transforms.get_mut(*entity) {
                *pos = new.position;
                rot.0 = new.rotation;
                scl.0 = new.scale;
            }
        }
        UndoAction::SpawnEntity { entity, components, .. } => {
            respawn_entity(commands, *entity, components);
        }
        UndoAction::DespawnEntity { entity, .. } => {
            commands.entity(*entity).despawn();
        }
        UndoAction::PropertyEdit { entity, new_value, .. } => {
            apply_property_value(transforms, *entity, new_value);
        }
        UndoAction::Reparent { entity, new_parent, .. } => {
            match new_parent {
                Some(parent) => commands.entity(*entity).set_parent(*parent),
                None => commands.entity(*entity).remove_parent(),
            };
        }
        UndoAction::Group(actions) => {
            for action in actions {
                apply_forward(action, chunks, commands, transforms);
            }
        }
    }
}
```

### Memory Considerations

Each `VoxelBatchOp` stores `VoxelChangeEntry` structs at roughly 24 bytes each (coordinate + two u16 IDs + padding). A large brush stroke affecting 1000 voxels costs ~24 KB. With a stack depth of 100, the worst case (100 large brush strokes) uses ~2.4 MB — well within budget. Transform changes are ~100 bytes each. Entity spawn/despawn operations serialize the component bundle, which varies but is typically under 1 KB per entity.

The `Group` variant allows compound operations (e.g., "duplicate entity and reparent it") to be undone as a single step, preventing the user from reaching an inconsistent intermediate state.

## Outcome

An `undo_redo.rs` module in `crates/nebula_editor/src/` exporting `UndoStack`, `UndoAction`, `VoxelBatchOp`, `VoxelChangeEntry`, `PropertyValue`, `undo_system`, `apply_inverse`, and `apply_forward`. Ctrl+Z undoes the most recent action, Ctrl+Shift+Z redoes. All editor tools (voxel painting, terrain brushes, entity spawner, transform gizmos, inspector, hierarchy) push their operations onto the shared `UndoStack`. The stack depth is capped at 100 by default and is configurable.

## Demo Integration

**Demo crate:** `nebula-demo`

Ctrl+Z undoes the last editor action. Ctrl+Y redoes. All editor operations (voxel edits, entity changes, transforms) are tracked.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | Resource storage for undo stack, `Commands` for entity spawn/despawn, queries for transform access |
| `glam` | `0.32` | `Vec3`, `Quat` types in `TransformSnapshot` and `PropertyValue` |
| `serde` | `1.0` | Serialize/deserialize component bundles for entity respawn on undo |
| `winit` | `0.30` | Key codes for Ctrl+Z and Ctrl+Shift+Z detection |

Rust edition 2024. Depends on `nebula_voxel` (for `ChunkManager`, `VoxelTypeId`, `VoxelCoord`), `nebula_input` (for `KeyboardState`), `nebula_ecs` (for `WorldPos`, `Rotation`, `Scale`), and `nebula_math`.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_reverses_last_action() {
        let mut stack = UndoStack::default();
        let action = UndoAction::TransformChange {
            entity: Entity::from_raw(1),
            old: TransformSnapshot { position: WorldPos::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE },
            new: TransformSnapshot { position: WorldPos::new(10, 0, 0), rotation: Quat::IDENTITY, scale: Vec3::ONE },
        };
        stack.push(action);
        assert!(stack.can_undo());
        let undone = stack.pop_undo().unwrap();
        // The undone action should restore the old transform
        match &undone {
            UndoAction::TransformChange { old, .. } => {
                assert_eq!(old.position, WorldPos::ZERO);
            }
            _ => panic!("Wrong action type"),
        }
    }

    #[test]
    fn test_redo_reapplies_undone_action() {
        let mut stack = UndoStack::default();
        let action = UndoAction::VoxelBatch(VoxelBatchOp {
            entries: vec![VoxelChangeEntry {
                coord: VoxelCoord::new(0, 0, 0),
                old_type: VoxelTypeId(0),
                new_type: VoxelTypeId(1),
            }],
        });
        stack.push(action);
        stack.pop_undo(); // undo
        assert!(stack.can_redo());
        let redone = stack.pop_redo().unwrap();
        match &redone {
            UndoAction::VoxelBatch(batch) => {
                assert_eq!(batch.entries[0].new_type, VoxelTypeId(1));
            }
            _ => panic!("Wrong action type"),
        }
    }

    #[test]
    fn test_undo_stack_depth_limited() {
        let mut stack = UndoStack::default();
        stack.max_depth = 5;
        for i in 0..10 {
            stack.push(UndoAction::VoxelBatch(VoxelBatchOp::new()));
        }
        assert_eq!(stack.undo_depth(), 5);
    }

    #[test]
    fn test_new_action_clears_redo_stack() {
        let mut stack = UndoStack::default();
        stack.push(UndoAction::VoxelBatch(VoxelBatchOp::new()));
        stack.push(UndoAction::VoxelBatch(VoxelBatchOp::new()));
        stack.pop_undo(); // undo one action, redo stack has 1
        assert!(stack.can_redo());
        stack.push(UndoAction::VoxelBatch(VoxelBatchOp::new())); // new action
        assert!(!stack.can_redo()); // redo stack cleared
    }

    #[test]
    fn test_multiple_undos_chain_correctly() {
        let mut stack = UndoStack::default();
        stack.push(UndoAction::VoxelBatch(VoxelBatchOp::new()));
        stack.push(UndoAction::VoxelBatch(VoxelBatchOp::new()));
        stack.push(UndoAction::VoxelBatch(VoxelBatchOp::new()));
        assert_eq!(stack.undo_depth(), 3);
        stack.pop_undo();
        assert_eq!(stack.undo_depth(), 2);
        stack.pop_undo();
        assert_eq!(stack.undo_depth(), 1);
        stack.pop_undo();
        assert_eq!(stack.undo_depth(), 0);
        assert!(!stack.can_undo());
    }

    #[test]
    fn test_undo_works_for_all_operation_types() {
        let mut stack = UndoStack::default();

        // VoxelBatch
        stack.push(UndoAction::VoxelBatch(VoxelBatchOp::new()));
        assert!(matches!(stack.pop_undo().unwrap(), UndoAction::VoxelBatch(_)));

        // TransformChange
        stack.push(UndoAction::TransformChange {
            entity: Entity::from_raw(1),
            old: TransformSnapshot { position: WorldPos::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE },
            new: TransformSnapshot { position: WorldPos::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE },
        });
        assert!(matches!(stack.pop_undo().unwrap(), UndoAction::TransformChange { .. }));

        // Reparent
        stack.push(UndoAction::Reparent {
            entity: Entity::from_raw(1),
            old_parent: None,
            new_parent: Some(Entity::from_raw(2)),
        });
        assert!(matches!(stack.pop_undo().unwrap(), UndoAction::Reparent { .. }));
    }

    #[test]
    fn test_group_undo_reverses_all_sub_actions() {
        let mut stack = UndoStack::default();
        let group = UndoAction::Group(vec![
            UndoAction::VoxelBatch(VoxelBatchOp::new()),
            UndoAction::VoxelBatch(VoxelBatchOp::new()),
        ]);
        stack.push(group);
        let undone = stack.pop_undo().unwrap();
        match undone {
            UndoAction::Group(actions) => assert_eq!(actions.len(), 2),
            _ => panic!("Expected Group"),
        }
    }

    #[test]
    fn test_empty_stack_undo_returns_none() {
        let mut stack = UndoStack::default();
        assert!(stack.pop_undo().is_none());
    }

    #[test]
    fn test_empty_stack_redo_returns_none() {
        let mut stack = UndoStack::default();
        assert!(stack.pop_redo().is_none());
    }
}
```
