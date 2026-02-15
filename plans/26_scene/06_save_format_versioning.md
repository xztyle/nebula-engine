# Save Format Versioning

## Problem

The save file format will inevitably change as the engine evolves. New component types are added, data layouts change, fields are renamed or removed. Without versioning, old saves silently produce corrupted state or crash on load. Players lose their worlds. The engine needs a migration system that can read old save files and upgrade them to the current format, preserving player data across engine updates.

The tricky part is that migrations are not just data transformations -- they can involve structural changes. Version 1 might store health as a single `u32`; version 2 might split it into `current: u32, max: u32`. Version 3 might add a `shield: u32` field. Each migration step must be a pure function from one version's data to the next, composable in sequence. A version 1 save must pass through `v1->v2` then `v2->v3` to reach version 3.

Some migrations may be impossible. If a format version is so old that critical data was stored in a now-unrecoverable layout, the engine should show a friendly error rather than silently corrupting the save.

## Solution

Implement a versioned migration pipeline in `nebula_save` that detects save file versions and applies sequential transformations.

### Version Constants

```rust
/// The oldest save version that can still be migrated to current.
/// Saves older than this are rejected with a friendly error.
pub const MIN_SUPPORTED_SAVE_VERSION: u32 = 1;

/// The current save format version. Increment when the format changes.
pub const CURRENT_SAVE_VERSION: u32 = 3;
```

### Migration Trait

```rust
use serde_json::Value as JsonValue;

/// A single migration step from version N to version N+1.
///
/// Migrations operate on the RON metadata (`world.ron`) and on the
/// entity data (as a `SceneData`). Chunk data has its own versioning
/// (see the voxel chunk serialization story) and is not handled here.
pub trait SaveMigration: Send + Sync {
    /// The version this migration upgrades FROM.
    fn from_version(&self) -> u32;
    /// The version this migration upgrades TO (must be from_version + 1).
    fn to_version(&self) -> u32;

    /// Migrate the world metadata RON. The input is a mutable RON Value
    /// that can be inspected and modified. Fields can be added, removed,
    /// or transformed.
    fn migrate_meta(&self, meta: &mut ron::Value) -> Result<(), MigrationError>;

    /// Migrate the scene data. Entity components can be added, removed,
    /// renamed, or have their data transformed.
    fn migrate_scene(&self, scene: &mut SceneData) -> Result<(), MigrationError>;
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("migration from v{from} to v{to} failed: {reason}")]
    Failed { from: u32, to: u32, reason: String },
    #[error("save version {version} is too old (minimum supported: {min_supported})")]
    TooOld { version: u32, min_supported: u32 },
    #[error("save version {version} is newer than the engine supports ({max_supported})")]
    TooNew { version: u32, max_supported: u32 },
    #[error("no migration path from v{from} to v{to}")]
    NoPath { from: u32, to: u32 },
}
```

### Concrete Migration Examples

```rust
/// Migration from save version 1 to version 2.
/// Change: Health component split from `health: u32` to `{ current: u32, max: u32 }`.
pub struct MigrateV1ToV2;

impl SaveMigration for MigrateV1ToV2 {
    fn from_version(&self) -> u32 { 1 }
    fn to_version(&self) -> u32 { 2 }

    fn migrate_meta(&self, meta: &mut ron::Value) -> Result<(), MigrationError> {
        // V1 -> V2: No metadata changes, just bump the version
        set_version(meta, 2);
        Ok(())
    }

    fn migrate_scene(&self, scene: &mut SceneData) -> Result<(), MigrationError> {
        for entity in &mut scene.entities {
            for comp in &mut entity.components {
                if comp.type_name == "Health" {
                    // Old format: Ron("100")  (just a u32)
                    // New format: Ron("(current: 100, max: 100)")
                    if let ComponentValue::Ron(old_val) = &comp.data {
                        if let Ok(hp) = old_val.trim().parse::<u32>() {
                            comp.data = ComponentValue::Ron(
                                format!("(current: {}, max: {})", hp, hp)
                            );
                        }
                    }
                }
            }
        }
        scene.version = 2;
        Ok(())
    }
}

/// Migration from save version 2 to version 3.
/// Change: Added `shield` field to Health component with default value 0.
pub struct MigrateV2ToV3;

impl SaveMigration for MigrateV2ToV3 {
    fn from_version(&self) -> u32 { 2 }
    fn to_version(&self) -> u32 { 3 }

    fn migrate_meta(&self, meta: &mut ron::Value) -> Result<(), MigrationError> {
        set_version(meta, 3);
        Ok(())
    }

    fn migrate_scene(&self, scene: &mut SceneData) -> Result<(), MigrationError> {
        for entity in &mut scene.entities {
            for comp in &mut entity.components {
                if comp.type_name == "Health" {
                    // Add shield: 0 to existing Health components
                    if let ComponentValue::Ron(val) = &comp.data {
                        // "(current: 80, max: 100)" -> "(current: 80, max: 100, shield: 0)"
                        let new_val = val.trim_end_matches(')').to_string()
                            + ", shield: 0)";
                        comp.data = ComponentValue::Ron(new_val);
                    }
                }
            }
        }
        scene.version = 3;
        Ok(())
    }
}
```

### Migration Registry and Pipeline

```rust
/// Registry of all available migrations, ordered by version.
pub struct MigrationPipeline {
    migrations: Vec<Box<dyn SaveMigration>>,
}

impl MigrationPipeline {
    pub fn new() -> Self {
        let mut pipeline = Self {
            migrations: Vec::new(),
        };
        // Register all known migrations
        pipeline.register(Box::new(MigrateV1ToV2));
        pipeline.register(Box::new(MigrateV2ToV3));
        pipeline
    }

    pub fn register(&mut self, migration: Box<dyn SaveMigration>) {
        self.migrations.push(migration);
        self.migrations.sort_by_key(|m| m.from_version());
    }

    /// Check the save version and apply all necessary migrations to
    /// bring it up to CURRENT_SAVE_VERSION.
    pub fn migrate(
        &self,
        meta: &mut ron::Value,
        scene: &mut SceneData,
        save_version: u32,
    ) -> Result<(), MigrationError> {
        // Reject versions that are too old
        if save_version < MIN_SUPPORTED_SAVE_VERSION {
            return Err(MigrationError::TooOld {
                version: save_version,
                min_supported: MIN_SUPPORTED_SAVE_VERSION,
            });
        }

        // Reject versions from the future
        if save_version > CURRENT_SAVE_VERSION {
            return Err(MigrationError::TooNew {
                version: save_version,
                max_supported: CURRENT_SAVE_VERSION,
            });
        }

        // Already current
        if save_version == CURRENT_SAVE_VERSION {
            return Ok(());
        }

        // Apply migrations sequentially
        let mut current = save_version;
        while current < CURRENT_SAVE_VERSION {
            let migration = self.migrations.iter()
                .find(|m| m.from_version() == current)
                .ok_or(MigrationError::NoPath {
                    from: current,
                    to: current + 1,
                })?;

            tracing::info!(
                "Migrating save from v{} to v{}",
                migration.from_version(),
                migration.to_version(),
            );

            migration.migrate_meta(meta)?;
            migration.migrate_scene(scene)?;
            current = migration.to_version();
        }

        Ok(())
    }
}

/// Helper to update the version field in a RON Value.
fn set_version(meta: &mut ron::Value, version: u32) {
    // Implementation depends on ron::Value API; conceptually:
    // meta["save_version"] = version;
}
```

### Integration with Load

The migration pipeline is invoked during `load_world()`:

```rust
pub fn load_world_with_migration(
    save_dir: &std::path::Path,
    world: &mut World,
    registry: &ComponentRegistry,
    pipeline: &MigrationPipeline,
) -> Result<LoadedWorld, SaveError> {
    // Read raw metadata to detect version
    let meta_str = fs::read_to_string(save_dir.join("world.ron"))?;
    let mut meta_value: ron::Value = ron::from_str(&meta_str)
        .map_err(|e| SaveError::Serialize(e.to_string()))?;

    let save_version = extract_version(&meta_value)
        .ok_or(SaveError::Serialize("missing save_version field".into()))?;

    // Load raw scene data
    let entity_bytes = fs::read(save_dir.join("entities.bin"))?;
    let mut scene = SceneData::from_binary(&entity_bytes)?;

    // Apply migrations if needed
    pipeline.migrate(&mut meta_value, &mut scene, save_version)
        .map_err(|e| SaveError::Serialize(e.to_string()))?;

    // Parse the migrated metadata into WorldMeta
    let meta: WorldMeta = ron::from_str(&ron::to_string(&meta_value).unwrap())
        .map_err(|e| SaveError::Serialize(e.to_string()))?;

    // Load entities from the migrated scene
    let spawned = load_scene(world, &scene, registry, LoadMode::Replace)?;

    // ... (discover chunk files as in story 05)

    Ok(LoadedWorld {
        meta,
        spawned_entities: spawned,
        saved_chunk_addresses: vec![],
    })
}
```

### Design Decisions

- **Sequential migrations over big-bang**: Each migration step transforms from version N to N+1. This means each migration is small and testable. The alternative -- a single migration from any version to the latest -- becomes unmaintainable as versions accumulate.
- **RON Value as intermediate representation**: Migrations operate on `ron::Value` rather than typed structs. This allows migrations to handle fields that do not exist in the current code (e.g., a removed field from version 1). Typed deserialization would fail on unknown fields.
- **Minimum supported version**: Rather than maintaining migrations all the way back to version 0, the engine declares a minimum supported version. Very old saves get a clear error message ("This save is from an earlier version that is no longer supported") rather than a cryptic crash.
- **Chunk data versioned separately**: Chunk files have their own version byte (story 05 of the voxel module). The world save version covers metadata and entity data layout, not chunk binary format.

## Outcome

A `MigrationPipeline` that detects save file versions and applies sequential migration functions to upgrade old saves to the current format. Each migration is a small, testable transformation from version N to N+1. Saves that are too old or too new are rejected with friendly error messages. The version number is stored in `world.ron` and checked on every load.

## Demo Integration

**Demo crate:** `nebula-demo`

Save files carry a version number. Loading an older save triggers automatic migration. A warning is shown, not a crash.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | `1.0` | Serialize/Deserialize for save data |
| `ron` | `0.12` | RON Value type for migration intermediate representation |
| `postcard` | `1.1` | Binary scene data deserialization |
| `thiserror` | `2.0` | Error type derivation for MigrationError |
| `tracing` | `0.1` | Logging migration steps |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_version_save_loads_directly() {
        let pipeline = MigrationPipeline::new();
        let mut meta = ron::from_str::<ron::Value>(
            &format!("(save_version: {})", CURRENT_SAVE_VERSION)
        ).unwrap();
        let mut scene = SceneData {
            version: CURRENT_SAVE_VERSION,
            entities: vec![],
            relationships: vec![],
            prefab_refs: vec![],
        };

        // No migration needed -- should succeed without changes
        let result = pipeline.migrate(&mut meta, &mut scene, CURRENT_SAVE_VERSION);
        assert!(result.is_ok());
        assert_eq!(scene.version, CURRENT_SAVE_VERSION);
    }

    #[test]
    fn test_old_version_triggers_migration() {
        let pipeline = MigrationPipeline::new();
        let mut meta = ron::from_str::<ron::Value>("(save_version: 1)").unwrap();
        let mut scene = SceneData {
            version: 1,
            entities: vec![EntityData {
                name: Some("Player".into()),
                stable_id: 1,
                components: vec![ComponentEntry {
                    type_name: "Health".into(),
                    data: ComponentValue::Ron("100".into()),
                }],
            }],
            relationships: vec![],
            prefab_refs: vec![],
        };

        let result = pipeline.migrate(&mut meta, &mut scene, 1);
        assert!(result.is_ok());
        // Scene should now be at CURRENT_SAVE_VERSION
        assert_eq!(scene.version, CURRENT_SAVE_VERSION);
    }

    #[test]
    fn test_migration_produces_valid_data() {
        let pipeline = MigrationPipeline::new();
        let mut meta = ron::from_str::<ron::Value>("(save_version: 1)").unwrap();
        let mut scene = SceneData {
            version: 1,
            entities: vec![EntityData {
                name: Some("Player".into()),
                stable_id: 1,
                components: vec![ComponentEntry {
                    type_name: "Health".into(),
                    data: ComponentValue::Ron("80".into()),
                }],
            }],
            relationships: vec![],
            prefab_refs: vec![],
        };

        pipeline.migrate(&mut meta, &mut scene, 1).unwrap();

        // After v1->v2: Health should be "(current: 80, max: 80)"
        // After v2->v3: Health should include shield: 0
        let health_data = &scene.entities[0].components[0].data;
        if let ComponentValue::Ron(val) = health_data {
            assert!(val.contains("current: 80"), "Should contain current HP");
            assert!(val.contains("max: 80"), "Should contain max HP");
            assert!(val.contains("shield: 0"), "Should contain shield field");
        } else {
            panic!("Expected Ron component value");
        }
    }

    #[test]
    fn test_too_old_version_shows_error() {
        let pipeline = MigrationPipeline::new();
        let mut meta = ron::from_str::<ron::Value>("(save_version: 0)").unwrap();
        let mut scene = SceneData {
            version: 0,
            entities: vec![],
            relationships: vec![],
            prefab_refs: vec![],
        };

        // Version 0 is below MIN_SUPPORTED_SAVE_VERSION (1)
        let result = pipeline.migrate(&mut meta, &mut scene, 0);
        assert!(matches!(
            result,
            Err(MigrationError::TooOld { version: 0, min_supported: 1 })
        ));
    }

    #[test]
    fn test_future_version_shows_error() {
        let pipeline = MigrationPipeline::new();
        let mut meta = ron::from_str::<ron::Value>("(save_version: 999)").unwrap();
        let mut scene = SceneData {
            version: 999,
            entities: vec![],
            relationships: vec![],
            prefab_refs: vec![],
        };

        let result = pipeline.migrate(&mut meta, &mut scene, 999);
        assert!(matches!(
            result,
            Err(MigrationError::TooNew { version: 999, .. })
        ));
    }

    #[test]
    fn test_version_is_stored_in_save_file() {
        // Verify that CURRENT_SAVE_VERSION is written into WorldMeta
        let meta = WorldMeta {
            save_version: CURRENT_SAVE_VERSION,
            seed: 0,
            world_time: 0.0,
            display_name: "Test".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            last_saved_at: "2026-01-01T00:00:00Z".into(),
            play_time: 0.0,
        };

        let ron_str = ron::ser::to_string_pretty(&meta, ron::ser::PrettyConfig::default())
            .unwrap();
        assert!(ron_str.contains(&format!("save_version: {}", CURRENT_SAVE_VERSION)));
    }

    #[test]
    fn test_single_step_migration_v2_to_v3() {
        let pipeline = MigrationPipeline::new();
        let mut meta = ron::from_str::<ron::Value>("(save_version: 2)").unwrap();
        let mut scene = SceneData {
            version: 2,
            entities: vec![EntityData {
                name: None,
                stable_id: 1,
                components: vec![ComponentEntry {
                    type_name: "Health".into(),
                    data: ComponentValue::Ron("(current: 50, max: 100)".into()),
                }],
            }],
            relationships: vec![],
            prefab_refs: vec![],
        };

        pipeline.migrate(&mut meta, &mut scene, 2).unwrap();
        assert_eq!(scene.version, CURRENT_SAVE_VERSION);

        if let ComponentValue::Ron(val) = &scene.entities[0].components[0].data {
            assert!(val.contains("shield: 0"));
        } else {
            panic!("Expected Ron value");
        }
    }
}
```
