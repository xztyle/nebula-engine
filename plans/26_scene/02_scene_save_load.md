# Scene Save/Load

## Problem

The engine can define a scene format (story 01), but there is no mechanism to extract the live ECS world state into that format, or to spawn entities from a scene file back into the world. Without save/load, every game session starts from scratch and levels cannot be loaded from data files. The save system must handle selective serialization -- only entities explicitly marked as persistent should be written to disk, while transient entities (particles, debug overlays, one-frame events) are excluded. The load system must handle missing or unknown component types gracefully rather than crashing, because scene files created by a newer version of the engine may reference components that the current build does not have.

A gameplay scene might contain 1,000 persistent entities out of 5,000 total (the rest are particles, projectiles, and transient effects). Saving should complete in under 10 ms for the persistent subset. Loading should spawn all entities and wire up parent-child relationships in a single frame.

## Solution

Implement save and load functions in `nebula_scene` that bridge the live `bevy_ecs::World` and the `SceneData` format from story 01.

### Marker Component

```rust
use bevy_ecs::prelude::*;

/// Marker component. Only entities with `Persistent` are included when
/// saving the scene. Entities without it (particles, debug shapes, etc.)
/// are considered transient and skipped.
#[derive(Component, Default, Debug, Clone)]
pub struct Persistent;
```

### Component Registry

A registry maps Rust types to string names and provides serialize/deserialize closures. This allows the save system to iterate over all registered component types and extract them from the world without static knowledge of every type at compile time.

```rust
use std::collections::HashMap;

/// A registered component type with its serialization hooks.
pub struct RegisteredComponent {
    /// Fully qualified type name (e.g. "nebula_ecs::WorldPos").
    pub type_name: String,
    /// Extract this component from an entity and serialize it to a
    /// `ComponentValue`. Returns `None` if the entity lacks this component.
    pub extract: Box<dyn Fn(&World, Entity) -> Option<ComponentValue> + Send + Sync>,
    /// Deserialize a `ComponentValue` and insert the component onto an
    /// entity. Returns `Err` if deserialization fails.
    pub insert: Box<dyn Fn(&mut World, Entity, &ComponentValue) -> Result<(), String> + Send + Sync>,
}

/// Registry of all saveable component types.
#[derive(Resource, Default)]
pub struct ComponentRegistry {
    components: Vec<RegisteredComponent>,
    name_index: HashMap<String, usize>,
}

impl ComponentRegistry {
    /// Register a component type for serialization.
    pub fn register(&mut self, component: RegisteredComponent) {
        let idx = self.components.len();
        self.name_index.insert(component.type_name.clone(), idx);
        self.components.push(component);
    }

    pub fn get_by_name(&self, name: &str) -> Option<&RegisteredComponent> {
        self.name_index.get(name).map(|&i| &self.components[i])
    }

    pub fn iter(&self) -> impl Iterator<Item = &RegisteredComponent> {
        self.components.iter()
    }
}
```

### Save Function

```rust
use tracing::warn;

/// Save all persistent entities from the world into a `SceneData`.
///
/// Only entities with the `Persistent` component are included. For each
/// persistent entity, every registered component type is checked and
/// serialized if present.
pub fn save_scene(world: &World, registry: &ComponentRegistry) -> SceneData {
    let mut entities = Vec::new();
    let mut entity_index_map: HashMap<Entity, usize> = HashMap::new();
    let mut stable_id_counter: u64 = 0;

    // Query all entities with the Persistent marker
    let mut query = world.query_filtered::<Entity, With<Persistent>>();
    for entity in query.iter(world) {
        let idx = entities.len();
        entity_index_map.insert(entity, idx);

        let mut components = Vec::new();
        for reg in registry.iter() {
            if let Some(value) = (reg.extract)(world, entity) {
                components.push(ComponentEntry {
                    type_name: reg.type_name.clone(),
                    data: value,
                });
            }
        }

        stable_id_counter += 1;
        entities.push(EntityData {
            name: None, // Name component extracted via registry if registered
            stable_id: stable_id_counter,
            components,
        });
    }

    // Build parent-child relationships from bevy_ecs hierarchy
    let mut relationships = Vec::new();
    for (entity, &parent_idx) in &entity_index_map {
        // Check if entity has a Parent component pointing to another
        // persistent entity
        if let Some(parent_entity) = world.get::<bevy_ecs::hierarchy::ChildOf>(*entity) {
            if let Some(&child_idx) = entity_index_map.get(&parent_entity.get()) {
                // Note: parent_idx is the child in the scene, child_idx is the parent
                relationships.push(ParentChild {
                    parent: child_idx,
                    child: parent_idx,
                });
            }
        }
    }

    SceneData {
        version: SCENE_FORMAT_VERSION,
        entities,
        relationships,
        prefab_refs: vec![],
    }
}
```

### Load Function

```rust
/// Controls how loading interacts with the existing world state.
pub enum LoadMode {
    /// Remove all existing entities before loading. Clean slate.
    Replace,
    /// Merge loaded entities into the existing world. Existing entities
    /// are left untouched.
    Merge,
}

/// Load a `SceneData` into the world, spawning entities and wiring
/// parent-child relationships.
///
/// Unknown component types are logged as warnings and skipped rather
/// than causing a hard error. This allows scenes from newer engine
/// versions to be partially loaded.
pub fn load_scene(
    world: &mut World,
    scene: &SceneData,
    registry: &ComponentRegistry,
    mode: LoadMode,
) -> Result<Vec<Entity>, SceneLoadError> {
    // Optionally clear the world
    if matches!(mode, LoadMode::Replace) {
        world.clear_entities();
    }

    let mut spawned: Vec<Entity> = Vec::with_capacity(scene.entities.len());

    // Spawn entities and insert components
    for entity_data in &scene.entities {
        let entity = world.spawn(Persistent).id();

        for comp in &entity_data.components {
            match registry.get_by_name(&comp.type_name) {
                Some(reg) => {
                    if let Err(e) = (reg.insert)(world, entity, &comp.data) {
                        warn!(
                            "Failed to deserialize component '{}' for entity {}: {}",
                            comp.type_name, entity_data.stable_id, e
                        );
                    }
                }
                None => {
                    warn!(
                        "Unknown component type '{}' on entity {} -- skipping",
                        comp.type_name, entity_data.stable_id
                    );
                }
            }
        }

        spawned.push(entity);
    }

    // Wire parent-child relationships
    for rel in &scene.relationships {
        if rel.parent < spawned.len() && rel.child < spawned.len() {
            let parent = spawned[rel.parent];
            let child = spawned[rel.child];
            world.entity_mut(child).set_parent(parent);
        } else {
            warn!(
                "Parent-child relationship ({} -> {}) references out-of-bounds entity index",
                rel.parent, rel.child
            );
        }
    }

    Ok(spawned)
}

#[derive(Debug, thiserror::Error)]
pub enum SceneLoadError {
    #[error("scene format error: {0}")]
    Format(#[from] SceneSerError),
    #[error("entity spawn failed: {0}")]
    SpawnFailed(String),
}
```

### Design Decisions

- **Persistent marker over opt-out**: An opt-in `Persistent` component is safer than an opt-out `Transient` marker. Forgetting to mark a particle emitter as transient would bloat save files; forgetting to mark a gameplay entity as persistent is caught quickly in playtesting.
- **Component registry over reflection**: `bevy_reflect` could discover components at runtime, but it requires all components to derive `Reflect` and introduces a dynamic dispatch overhead. An explicit registry keeps serialization fast and predictable, and avoids coupling the save system to Bevy's reflection infrastructure.
- **Warn-and-skip for unknown types**: Rather than failing on unknown component types, the loader logs a warning and continues. This is critical for forward compatibility -- a save file from a newer engine version can be loaded by an older version with minimal data loss.
- **Replace vs Merge**: The `LoadMode` enum gives callers explicit control. Level transitions use `Replace` (clean slate). Editor features like "import entities from file" use `Merge`.

## Outcome

A `save_scene()` function that extracts persistent entities from the ECS world into a `SceneData`, and a `load_scene()` function that spawns entities from a `SceneData` back into the world. The system uses an explicit `ComponentRegistry` for type-safe serialization and handles unknown component types gracefully with warnings. Both replace and merge loading modes are supported.

## Demo Integration

**Demo crate:** `nebula-demo`

F5 saves the current world state. F9 loads it back. Player position, voxel modifications, and entity state all restore correctly.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS World, Entity, Component, Query, hierarchy |
| `serde` | `1.0` | Serialize/Deserialize for component data |
| `ron` | `0.12` | RON serialization for human-readable component values |
| `postcard` | `1.1` | Binary serialization for production component values |
| `tracing` | `0.1` | Warning logs for unknown/failed component types |
| `thiserror` | `2.0` | Error type derivation |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    #[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Health {
        current: u32,
        max: u32,
    }

    #[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Position {
        x: i128,
        y: i128,
        z: i128,
    }

    fn test_registry() -> ComponentRegistry {
        let mut reg = ComponentRegistry::default();
        reg.register(RegisteredComponent {
            type_name: "Health".into(),
            extract: Box::new(|world, entity| {
                world.get::<Health>(entity).map(|h| {
                    ComponentValue::Ron(ron::to_string(h).unwrap())
                })
            }),
            insert: Box::new(|world, entity, value| {
                if let ComponentValue::Ron(s) = value {
                    let h: Health = ron::from_str(s).map_err(|e| e.to_string())?;
                    world.entity_mut(entity).insert(h);
                }
                Ok(())
            }),
        });
        reg.register(RegisteredComponent {
            type_name: "Position".into(),
            extract: Box::new(|world, entity| {
                world.get::<Position>(entity).map(|p| {
                    ComponentValue::Ron(ron::to_string(p).unwrap())
                })
            }),
            insert: Box::new(|world, entity, value| {
                if let ComponentValue::Ron(s) = value {
                    let p: Position = ron::from_str(s).map_err(|e| e.to_string())?;
                    world.entity_mut(entity).insert(p);
                }
                Ok(())
            }),
        });
        reg
    }

    #[test]
    fn test_save_then_load_produces_equivalent_state() {
        let mut world = World::new();
        let registry = test_registry();

        // Spawn a persistent entity
        world.spawn((
            Persistent,
            Health { current: 80, max: 100 },
            Position { x: 10, y: 20, z: 30 },
        ));

        let scene = save_scene(&world, &registry);
        assert_eq!(scene.entities.len(), 1);

        // Load into a fresh world
        let mut new_world = World::new();
        let spawned = load_scene(&mut new_world, &scene, &registry, LoadMode::Replace)
            .expect("load failed");

        assert_eq!(spawned.len(), 1);
        let health = new_world.get::<Health>(spawned[0]).unwrap();
        assert_eq!(health.current, 80);
        assert_eq!(health.max, 100);
        let pos = new_world.get::<Position>(spawned[0]).unwrap();
        assert_eq!(pos.x, 10);
    }

    #[test]
    fn test_only_persistent_entities_are_saved() {
        let mut world = World::new();
        let registry = test_registry();

        // Persistent entity
        world.spawn((Persistent, Health { current: 50, max: 50 }));
        // Transient entity (no Persistent marker)
        world.spawn(Health { current: 1, max: 1 });

        let scene = save_scene(&world, &registry);
        // Only the persistent entity should appear
        assert_eq!(scene.entities.len(), 1);
    }

    #[test]
    fn test_load_creates_entities_with_correct_components() {
        let scene = SceneData {
            version: SCENE_FORMAT_VERSION,
            entities: vec![EntityData {
                name: Some("TestEntity".into()),
                stable_id: 42,
                components: vec![ComponentEntry {
                    type_name: "Health".into(),
                    data: ComponentValue::Ron("(current: 75, max: 100)".into()),
                }],
            }],
            relationships: vec![],
            prefab_refs: vec![],
        };

        let mut world = World::new();
        let registry = test_registry();
        let spawned = load_scene(&mut world, &scene, &registry, LoadMode::Replace).unwrap();

        assert_eq!(spawned.len(), 1);
        let health = world.get::<Health>(spawned[0]).unwrap();
        assert_eq!(health.current, 75);
        assert_eq!(health.max, 100);
    }

    #[test]
    fn test_missing_component_type_warns_and_skips() {
        let scene = SceneData {
            version: SCENE_FORMAT_VERSION,
            entities: vec![EntityData {
                name: None,
                stable_id: 1,
                components: vec![ComponentEntry {
                    type_name: "NonexistentComponent".into(),
                    data: ComponentValue::Ron("()".into()),
                }],
            }],
            relationships: vec![],
            prefab_refs: vec![],
        };

        let mut world = World::new();
        let registry = test_registry(); // Does not include NonexistentComponent
        // Should not panic -- unknown types are warned and skipped
        let spawned = load_scene(&mut world, &scene, &registry, LoadMode::Replace).unwrap();
        assert_eq!(spawned.len(), 1);
        // Entity exists but has no extra components beyond Persistent
        assert!(world.get::<Health>(spawned[0]).is_none());
    }

    #[test]
    fn test_empty_scene_produces_empty_world() {
        let scene = SceneData {
            version: SCENE_FORMAT_VERSION,
            entities: vec![],
            relationships: vec![],
            prefab_refs: vec![],
        };

        let mut world = World::new();
        let registry = test_registry();
        let spawned = load_scene(&mut world, &scene, &registry, LoadMode::Replace).unwrap();
        assert_eq!(spawned.len(), 0);
        assert_eq!(world.entities().len(), 0);
    }

    #[test]
    fn test_replace_mode_clears_existing_entities() {
        let mut world = World::new();
        world.spawn(Health { current: 1, max: 1 });
        world.spawn(Health { current: 2, max: 2 });
        assert_eq!(world.entities().len(), 2);

        let scene = SceneData {
            version: SCENE_FORMAT_VERSION,
            entities: vec![],
            relationships: vec![],
            prefab_refs: vec![],
        };

        let registry = test_registry();
        load_scene(&mut world, &scene, &registry, LoadMode::Replace).unwrap();
        assert_eq!(world.entities().len(), 0);
    }

    #[test]
    fn test_merge_mode_preserves_existing_entities() {
        let mut world = World::new();
        world.spawn(Health { current: 1, max: 1 });

        let scene = SceneData {
            version: SCENE_FORMAT_VERSION,
            entities: vec![EntityData {
                name: None,
                stable_id: 1,
                components: vec![],
            }],
            relationships: vec![],
            prefab_refs: vec![],
        };

        let registry = test_registry();
        load_scene(&mut world, &scene, &registry, LoadMode::Merge).unwrap();
        // Original entity + 1 new entity from scene
        assert_eq!(world.entities().len(), 2);
    }
}
```
