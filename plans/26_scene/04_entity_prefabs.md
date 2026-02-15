# Entity Prefabs

## Problem

Game worlds are built from many copies of the same entity configurations: trees, rocks, enemies, light fixtures, spaceships. Without a prefab system, every instance must be constructed manually in code or duplicated verbatim in scene files. This leads to massive duplication -- if a tree has 6 components, placing 500 trees means 3,000 component entries in the scene file. Changing the tree's collision shape means editing all 500 instances. Prefabs solve this by defining a template once and instantiating it many times, with optional per-instance overrides.

Prefabs must also support nesting: a spaceship prefab contains engine prefabs, which contain particle-emitter prefabs. The nesting must be resolved at spawn time, producing a flat entity hierarchy in the ECS world with correct parent-child relationships.

## Solution

Implement a prefab system in `nebula_scene` that reuses the `SceneData` format from story 01 as the prefab file format (a prefab is just a small scene), and provides a `spawn_prefab()` function that instantiates it into the world.

### Prefab Asset

```rust
use serde::{Serialize, Deserialize};

/// A prefab is a reusable entity template stored as a scene file.
/// It contains one or more entities with preset components and
/// parent-child relationships, plus optional nested prefab references.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefab {
    /// The underlying scene data defining the prefab's entities.
    pub scene: SceneData,
    /// Index of the root entity in `scene.entities`. Components can be
    /// overridden on this entity at spawn time.
    pub root_entity_index: usize,
}

impl Prefab {
    /// Load a prefab from a RON string.
    pub fn from_ron(input: &str) -> Result<Self, SceneSerError> {
        ron::from_str(input).map_err(|e| SceneSerError::RonDeserialize(e.to_string()))
    }

    /// Load a prefab from binary data.
    pub fn from_binary(data: &[u8]) -> Result<Self, SceneSerError> {
        if data.len() < 4 || &data[0..4] != b"NPFB" {
            return Err(SceneSerError::InvalidMagic);
        }
        postcard::from_bytes(&data[4..])
            .map_err(|e| SceneSerError::BinaryDeserialize(e.to_string()))
    }
}
```

### Component Overrides

```rust
/// Per-instance overrides applied when spawning a prefab. These replace
/// or add component values on the root entity (or any entity by index).
#[derive(Debug, Clone, Default)]
pub struct PrefabOverrides {
    /// Map from entity index (within the prefab) to component overrides.
    /// Each override is a list of (type_name, value) pairs.
    pub overrides: HashMap<usize, Vec<ComponentEntry>>,
}

impl PrefabOverrides {
    /// Override a component on the root entity (index 0 by convention).
    pub fn set_root_component(&mut self, type_name: &str, value: ComponentValue) {
        self.overrides
            .entry(0)
            .or_default()
            .push(ComponentEntry {
                type_name: type_name.to_string(),
                data: value,
            });
    }

    /// Override a component on any entity by its index in the prefab.
    pub fn set_component(
        &mut self,
        entity_index: usize,
        type_name: &str,
        value: ComponentValue,
    ) {
        self.overrides
            .entry(entity_index)
            .or_default()
            .push(ComponentEntry {
                type_name: type_name.to_string(),
                data: value,
            });
    }
}
```

### Spawn Function

```rust
use bevy_ecs::prelude::*;
use std::collections::HashMap;
use tracing::warn;

/// Spawn all entities defined in a prefab into the world.
///
/// Returns the root entity and a list of all spawned entities. Component
/// overrides are applied after the base components from the prefab
/// template, so overrides win.
///
/// Nested prefabs (referenced via `prefab_refs` in the scene data) are
/// resolved recursively. Each nested prefab's entities become children
/// of the referencing entity.
pub fn spawn_prefab(
    world: &mut World,
    prefab: &Prefab,
    registry: &ComponentRegistry,
    overrides: &PrefabOverrides,
    prefab_cache: &PrefabCache,
) -> Result<SpawnedPrefab, SceneLoadError> {
    let mut spawned_entities: Vec<Entity> = Vec::new();
    let mut prefab_index_to_entity: HashMap<usize, Entity> = HashMap::new();

    // Phase 1: Spawn entities and insert base components
    for (idx, entity_data) in prefab.scene.entities.iter().enumerate() {
        let entity = world.spawn_empty().id();
        spawned_entities.push(entity);
        prefab_index_to_entity.insert(idx, entity);

        // Insert base components from the template
        for comp in &entity_data.components {
            if let Some(reg) = registry.get_by_name(&comp.type_name) {
                if let Err(e) = (reg.insert)(world, entity, &comp.data) {
                    warn!("Failed to insert component '{}': {}", comp.type_name, e);
                }
            } else {
                warn!("Unknown component type '{}' in prefab", comp.type_name);
            }
        }

        // Phase 2: Apply overrides for this entity
        if let Some(entity_overrides) = overrides.overrides.get(&idx) {
            for comp in entity_overrides {
                if let Some(reg) = registry.get_by_name(&comp.type_name) {
                    if let Err(e) = (reg.insert)(world, entity, &comp.data) {
                        warn!("Failed to apply override '{}': {}", comp.type_name, e);
                    }
                }
            }
        }
    }

    // Phase 3: Wire parent-child relationships
    for rel in &prefab.scene.relationships {
        if let (Some(&parent), Some(&child)) = (
            prefab_index_to_entity.get(&rel.parent),
            prefab_index_to_entity.get(&rel.child),
        ) {
            world.entity_mut(child).set_parent(parent);
        }
    }

    // Phase 4: Resolve nested prefab references
    for prefab_ref in &prefab.scene.prefab_refs {
        if let Some(nested_prefab) = prefab_cache.get(&prefab_ref.path) {
            let nested_result = spawn_prefab(
                world,
                nested_prefab,
                registry,
                &PrefabOverrides::default(),
                prefab_cache,
            )?;

            // Parent the nested prefab's root under the referencing entity
            if let Some(&parent_entity) = prefab_index_to_entity.get(&prefab_ref.root_entity) {
                world.entity_mut(nested_result.root).set_parent(parent_entity);
            }

            spawned_entities.extend(nested_result.all_entities);
        } else {
            warn!("Nested prefab '{}' not found in cache", prefab_ref.path);
        }
    }

    let root = prefab_index_to_entity[&prefab.root_entity_index];
    Ok(SpawnedPrefab {
        root,
        all_entities: spawned_entities,
    })
}

/// Result of spawning a prefab.
pub struct SpawnedPrefab {
    /// The root entity of the prefab instance.
    pub root: Entity,
    /// All entities spawned (including nested prefab entities).
    pub all_entities: Vec<Entity>,
}

/// Cache of loaded prefab assets, keyed by asset path.
#[derive(Resource, Default)]
pub struct PrefabCache {
    prefabs: HashMap<String, Prefab>,
}

impl PrefabCache {
    pub fn insert(&mut self, path: String, prefab: Prefab) {
        self.prefabs.insert(path, prefab);
    }

    pub fn get(&self, path: &str) -> Option<&Prefab> {
        self.prefabs.get(path)
    }
}
```

### Design Decisions

- **Prefab = SceneData**: Rather than inventing a separate prefab format, a prefab is a `SceneData` with a designated root entity index. This means the same serialization code, the same component registry, and the same tooling works for both scenes and prefabs.
- **Overrides as a separate map**: Overrides are passed at spawn time rather than baked into the prefab. This keeps the prefab template immutable and allows the same template to be instantiated with different configurations.
- **Recursive nesting**: Nested prefab references are resolved recursively at spawn time. This avoids pre-flattening and allows changes to a nested prefab to propagate automatically.
- **PrefabCache resource**: Prefabs are loaded once and cached. Multiple spawn calls for the same prefab share the same template data. The cache is an ECS resource so it can be managed by the asset system.

## Outcome

A `Prefab` type backed by `SceneData`, a `spawn_prefab()` function that instantiates prefab templates into the ECS world with component overrides and nested prefab resolution, and a `PrefabCache` resource for managing loaded prefab assets. Prefabs use the same RON/binary serialization as scene files.

## Demo Integration

**Demo crate:** `nebula-demo`

A "house" prefab (voxel structure with furniture entities) can be instantiated via script or editor, spawning all child entities.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bevy_ecs` | `0.18` | ECS World, Entity, Component, hierarchy |
| `serde` | `1.0` | Serialize/Deserialize for Prefab and SceneData |
| `ron` | `0.12` | RON deserialization for prefab files |
| `postcard` | `1.1` | Binary deserialization for prefab files |
| `tracing` | `0.1` | Warnings for missing components and nested prefabs |
| `thiserror` | `2.0` | Error type derivation |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    #[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Position { x: i128, y: i128, z: i128 }

    #[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Health { current: u32, max: u32 }

    #[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Mesh { model: String }

    fn test_registry() -> ComponentRegistry {
        let mut reg = ComponentRegistry::default();
        // Register Position, Health, Mesh with extract/insert closures
        // (same pattern as story 02 tests)
        reg.register(RegisteredComponent {
            type_name: "Position".into(),
            extract: Box::new(|w, e| w.get::<Position>(e).map(|p| {
                ComponentValue::Ron(ron::to_string(p).unwrap())
            })),
            insert: Box::new(|w, e, v| {
                if let ComponentValue::Ron(s) = v {
                    let p: Position = ron::from_str(s).map_err(|e| e.to_string())?;
                    w.entity_mut(e).insert(p);
                }
                Ok(())
            }),
        });
        reg.register(RegisteredComponent {
            type_name: "Health".into(),
            extract: Box::new(|w, e| w.get::<Health>(e).map(|h| {
                ComponentValue::Ron(ron::to_string(h).unwrap())
            })),
            insert: Box::new(|w, e, v| {
                if let ComponentValue::Ron(s) = v {
                    let h: Health = ron::from_str(s).map_err(|e| e.to_string())?;
                    w.entity_mut(e).insert(h);
                }
                Ok(())
            }),
        });
        reg.register(RegisteredComponent {
            type_name: "Mesh".into(),
            extract: Box::new(|w, e| w.get::<Mesh>(e).map(|m| {
                ComponentValue::Ron(ron::to_string(m).unwrap())
            })),
            insert: Box::new(|w, e, v| {
                if let ComponentValue::Ron(s) = v {
                    let m: Mesh = ron::from_str(s).map_err(|e| e.to_string())?;
                    w.entity_mut(e).insert(m);
                }
                Ok(())
            }),
        });
        reg
    }

    fn tree_prefab() -> Prefab {
        Prefab {
            scene: SceneData {
                version: SCENE_FORMAT_VERSION,
                entities: vec![EntityData {
                    name: Some("Tree".into()),
                    stable_id: 1,
                    components: vec![
                        ComponentEntry {
                            type_name: "Position".into(),
                            data: ComponentValue::Ron("(x: 0, y: 0, z: 0)".into()),
                        },
                        ComponentEntry {
                            type_name: "Mesh".into(),
                            data: ComponentValue::Ron("(model: \"oak_tree\")".into()),
                        },
                        ComponentEntry {
                            type_name: "Health".into(),
                            data: ComponentValue::Ron("(current: 50, max: 50)".into()),
                        },
                    ],
                }],
                relationships: vec![],
                prefab_refs: vec![],
            },
            root_entity_index: 0,
        }
    }

    #[test]
    fn test_prefab_spawns_correct_entity() {
        let mut world = World::new();
        let registry = test_registry();
        let prefab = tree_prefab();
        let cache = PrefabCache::default();

        let result = spawn_prefab(
            &mut world, &prefab, &registry,
            &PrefabOverrides::default(), &cache,
        ).unwrap();

        assert_eq!(result.all_entities.len(), 1);
        assert!(world.get::<Position>(result.root).is_some());
        assert!(world.get::<Mesh>(result.root).is_some());
        assert!(world.get::<Health>(result.root).is_some());
    }

    #[test]
    fn test_components_match_template() {
        let mut world = World::new();
        let registry = test_registry();
        let prefab = tree_prefab();
        let cache = PrefabCache::default();

        let result = spawn_prefab(
            &mut world, &prefab, &registry,
            &PrefabOverrides::default(), &cache,
        ).unwrap();

        let health = world.get::<Health>(result.root).unwrap();
        assert_eq!(health.current, 50);
        assert_eq!(health.max, 50);

        let mesh = world.get::<Mesh>(result.root).unwrap();
        assert_eq!(mesh.model, "oak_tree");
    }

    #[test]
    fn test_position_override_works() {
        let mut world = World::new();
        let registry = test_registry();
        let prefab = tree_prefab();
        let cache = PrefabCache::default();

        let mut overrides = PrefabOverrides::default();
        overrides.set_root_component(
            "Position",
            ComponentValue::Ron("(x: 100, y: 200, z: 300)".into()),
        );

        let result = spawn_prefab(
            &mut world, &prefab, &registry, &overrides, &cache,
        ).unwrap();

        let pos = world.get::<Position>(result.root).unwrap();
        assert_eq!(pos.x, 100);
        assert_eq!(pos.y, 200);
        assert_eq!(pos.z, 300);
    }

    #[test]
    fn test_nested_prefab_spawns_all_children() {
        let mut world = World::new();
        let registry = test_registry();

        // Create an engine prefab
        let engine_prefab = Prefab {
            scene: SceneData {
                version: SCENE_FORMAT_VERSION,
                entities: vec![EntityData {
                    name: Some("Engine".into()),
                    stable_id: 10,
                    components: vec![ComponentEntry {
                        type_name: "Mesh".into(),
                        data: ComponentValue::Ron("(model: \"engine\")".into()),
                    }],
                }],
                relationships: vec![],
                prefab_refs: vec![],
            },
            root_entity_index: 0,
        };

        // Create a ship prefab that references the engine prefab
        let ship_prefab = Prefab {
            scene: SceneData {
                version: SCENE_FORMAT_VERSION,
                entities: vec![EntityData {
                    name: Some("Ship".into()),
                    stable_id: 20,
                    components: vec![ComponentEntry {
                        type_name: "Mesh".into(),
                        data: ComponentValue::Ron("(model: \"ship_hull\")".into()),
                    }],
                }],
                relationships: vec![],
                prefab_refs: vec![PrefabRef {
                    path: "prefabs/engine.prefab.ron".into(),
                    root_entity: 0,
                }],
            },
            root_entity_index: 0,
        };

        let mut cache = PrefabCache::default();
        cache.insert("prefabs/engine.prefab.ron".into(), engine_prefab);

        let result = spawn_prefab(
            &mut world, &ship_prefab, &registry,
            &PrefabOverrides::default(), &cache,
        ).unwrap();

        // Ship entity + Engine entity = 2 total
        assert_eq!(result.all_entities.len(), 2);
    }

    #[test]
    fn test_multiple_instances_from_same_prefab_are_independent() {
        let mut world = World::new();
        let registry = test_registry();
        let prefab = tree_prefab();
        let cache = PrefabCache::default();

        let result1 = spawn_prefab(
            &mut world, &prefab, &registry,
            &PrefabOverrides::default(), &cache,
        ).unwrap();
        let result2 = spawn_prefab(
            &mut world, &prefab, &registry,
            &PrefabOverrides::default(), &cache,
        ).unwrap();

        // Two distinct entities
        assert_ne!(result1.root, result2.root);

        // Modifying one does not affect the other
        world.get_mut::<Health>(result1.root).unwrap().current = 10;
        let health2 = world.get::<Health>(result2.root).unwrap();
        assert_eq!(health2.current, 50); // unchanged
    }
}
```
