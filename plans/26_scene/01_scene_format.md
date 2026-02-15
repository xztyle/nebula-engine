# Scene Format

## Problem

The engine needs a standard file format for describing entity hierarchies with their component data. Without a defined scene format, there is no way to author levels, save game state, or build prefab templates -- everything must be constructed programmatically. The format must serve two audiences: human developers who need to read, edit, and diff scene files during development, and the production runtime where parsing speed and file size matter. It must capture entity-component data, parent-child relationships, and references to prefab templates while remaining forward-compatible as the engine evolves and new component types are added.

A scene file for a modest gameplay area might contain 500-2,000 entities, each with 3-8 components. The human-readable format should be editable in any text editor. The binary format should parse a 2,000-entity scene in under 1 ms on desktop hardware.

## Solution

Define the scene format in the `nebula_scene` crate, with RON (Rusty Object Notation) as the human-readable format and `postcard` as the compact binary format. Both formats serialize the same `SceneData` structure.

### Core Data Types

```rust
use serde::{Serialize, Deserialize};

/// Current scene format version. Increment when the schema changes.
pub const SCENE_FORMAT_VERSION: u32 = 1;

/// Top-level scene container. Every scene file (RON or binary) serializes this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SceneData {
    /// Format version for forward-compatibility checks.
    pub version: u32,
    /// All entities in the scene with their component data.
    pub entities: Vec<EntityData>,
    /// Parent-child relationships between entities (by index into `entities`).
    pub relationships: Vec<ParentChild>,
    /// References to external prefab files that this scene instantiates.
    pub prefab_refs: Vec<PrefabRef>,
}

/// A single entity and its serialized components.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityData {
    /// Optional human-readable name for debugging and editor display.
    pub name: Option<String>,
    /// Stable identifier persisted across save/load cycles. Used to
    /// reconnect relationships and external references after deserialization.
    pub stable_id: u64,
    /// Component data keyed by the component type name (e.g. "WorldPos",
    /// "Health"). Values are RON-encoded strings in the human-readable
    /// format and opaque byte blobs in the binary format.
    pub components: Vec<ComponentEntry>,
}

/// A named component with its serialized value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComponentEntry {
    /// Fully qualified component type name (e.g. "nebula_ecs::WorldPos").
    pub type_name: String,
    /// Serialized component data. In RON files this is a RON string.
    /// In binary files this is postcard-encoded bytes.
    pub data: ComponentValue,
}

/// Component payload -- either a RON string or raw bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ComponentValue {
    Ron(String),
    Binary(Vec<u8>),
}

/// A parent-child edge, using indices into `SceneData::entities`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParentChild {
    /// Index of the parent entity in `SceneData::entities`.
    pub parent: usize,
    /// Index of the child entity in `SceneData::entities`.
    pub child: usize,
}

/// A reference to an external prefab file, instantiated at a position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrefabRef {
    /// Path to the prefab file, relative to the assets directory.
    pub path: String,
    /// Index of the root entity in `SceneData::entities` that this
    /// prefab instance corresponds to.
    pub root_entity: usize,
}
```

### RON Format (Development)

RON is used for `.scene.ron` files. These are human-readable, diffable in version control, and editable in any text editor.

```rust
use ron::ser::PrettyConfig;

impl SceneData {
    /// Serialize to a pretty-printed RON string.
    pub fn to_ron(&self) -> Result<String, SceneSerError> {
        let config = PrettyConfig::new()
            .depth_limit(8)
            .struct_names(true);
        ron::ser::to_string_pretty(self, config)
            .map_err(|e| SceneSerError::RonSerialize(e.to_string()))
    }

    /// Deserialize from a RON string.
    pub fn from_ron(input: &str) -> Result<Self, SceneSerError> {
        let scene: SceneData = ron::from_str(input)
            .map_err(|e| SceneSerError::RonDeserialize(e.to_string()))?;
        if scene.version > SCENE_FORMAT_VERSION {
            return Err(SceneSerError::FutureVersion {
                found: scene.version,
                max_supported: SCENE_FORMAT_VERSION,
            });
        }
        Ok(scene)
    }
}
```

Example `.scene.ron` file:

```ron
SceneData(
    version: 1,
    entities: [
        EntityData(
            name: Some("Player"),
            stable_id: 1001,
            components: [
                ComponentEntry(
                    type_name: "nebula_ecs::WorldPos",
                    data: Ron("(x: 0, y: 6400000000, z: 0)"),
                ),
                ComponentEntry(
                    type_name: "nebula_gameplay::Health",
                    data: Ron("(current: 100, max: 100)"),
                ),
            ],
        ),
        EntityData(
            name: Some("PlayerCamera"),
            stable_id: 1002,
            components: [
                ComponentEntry(
                    type_name: "nebula_ecs::LocalPos",
                    data: Ron("(x: 0.0, y: 1.8, z: 0.0)"),
                ),
            ],
        ),
    ],
    relationships: [
        ParentChild(parent: 0, child: 1),
    ],
    prefab_refs: [],
)
```

### Binary Format (Production)

Binary scene files use `postcard` for zero-copy-friendly, compact serialization. File extension: `.scene.bin`.

```rust
impl SceneData {
    /// Serialize to postcard binary format with a 4-byte magic header.
    pub fn to_binary(&self) -> Result<Vec<u8>, SceneSerError> {
        let mut out = Vec::new();
        // Magic bytes: "NSCD" (Nebula Scene Data)
        out.extend_from_slice(&[0x4E, 0x53, 0x43, 0x44]);
        let payload = postcard::to_allocvec(self)
            .map_err(|e| SceneSerError::BinarySerialize(e.to_string()))?;
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Deserialize from postcard binary format.
    pub fn from_binary(data: &[u8]) -> Result<Self, SceneSerError> {
        if data.len() < 4 || &data[0..4] != b"NSCD" {
            return Err(SceneSerError::InvalidMagic);
        }
        let scene: SceneData = postcard::from_bytes(&data[4..])
            .map_err(|e| SceneSerError::BinaryDeserialize(e.to_string()))?;
        if scene.version > SCENE_FORMAT_VERSION {
            return Err(SceneSerError::FutureVersion {
                found: scene.version,
                max_supported: SCENE_FORMAT_VERSION,
            });
        }
        Ok(scene)
    }
}
```

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum SceneSerError {
    #[error("RON serialization failed: {0}")]
    RonSerialize(String),
    #[error("RON deserialization failed: {0}")]
    RonDeserialize(String),
    #[error("binary serialization failed: {0}")]
    BinarySerialize(String),
    #[error("binary deserialization failed: {0}")]
    BinaryDeserialize(String),
    #[error("invalid magic bytes in binary scene file")]
    InvalidMagic,
    #[error("scene version {found} is newer than max supported {max_supported}")]
    FutureVersion { found: u32, max_supported: u32 },
}
```

### Design Decisions

- **RON over TOML/JSON**: RON is Rust-native, supports enums and tuples natively, and produces output that closely matches Rust struct syntax. JSON lacks enum support; TOML is too flat for nested entity hierarchies.
- **Postcard over bincode**: `postcard` 1.1 produces smaller output for variable-length data (it uses varint encoding), supports `no_std`, and has a stable wire format suitable for save files. Bincode's format can change between versions.
- **Component data as opaque blobs**: Components are serialized as named blobs rather than using `bevy_reflect` or a trait-object registry. This keeps the scene format decoupled from the ECS component registry and allows unknown component types to be preserved (round-tripped) without losing data.
- **Stable IDs**: Each entity gets a `stable_id` rather than using transient ECS entity indices. This allows cross-references (e.g., "this trigger targets entity 1001") to survive save/load cycles.
- **Version header**: The version field is the first thing deserialized. If the format evolves, the loader can detect the version and either migrate or reject the file with a clear error message.

## Outcome

A `SceneData` type with `to_ron()` / `from_ron()` and `to_binary()` / `from_binary()` methods. RON files are human-readable and version-control friendly. Binary files are compact and fast to parse. Both formats include a version header for forward compatibility. The format captures entities with named components, parent-child relationships, and prefab references.

## Demo Integration

**Demo crate:** `nebula-demo`

A RON-based scene format is defined. Scene files are human-readable and describe entity hierarchies with component data.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `serde` | `1.0` | Serialize / Deserialize trait derivation for all scene types |
| `ron` | `0.12` | Human-readable RON serialization and deserialization |
| `postcard` | `1.1` | Compact binary serialization with stable wire format |
| `thiserror` | `2.0` | Ergonomic error type derivation |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a small scene with two entities and a parent-child edge.
    fn sample_scene() -> SceneData {
        SceneData {
            version: SCENE_FORMAT_VERSION,
            entities: vec![
                EntityData {
                    name: Some("Parent".into()),
                    stable_id: 1,
                    components: vec![ComponentEntry {
                        type_name: "WorldPos".into(),
                        data: ComponentValue::Ron("(x: 0, y: 100, z: 0)".into()),
                    }],
                },
                EntityData {
                    name: Some("Child".into()),
                    stable_id: 2,
                    components: vec![ComponentEntry {
                        type_name: "LocalPos".into(),
                        data: ComponentValue::Ron("(x: 0.0, y: 1.8, z: 0.0)".into()),
                    }],
                },
            ],
            relationships: vec![ParentChild { parent: 0, child: 1 }],
            prefab_refs: vec![],
        }
    }

    #[test]
    fn test_scene_serializes_to_ron() {
        let scene = sample_scene();
        let ron_str = scene.to_ron().expect("RON serialization failed");
        // RON output must contain the struct name and key fields
        assert!(ron_str.contains("SceneData"));
        assert!(ron_str.contains("version: 1"));
        assert!(ron_str.contains("WorldPos"));
        assert!(ron_str.contains("Parent"));
    }

    #[test]
    fn test_scene_deserializes_from_ron() {
        let scene = sample_scene();
        let ron_str = scene.to_ron().unwrap();
        let restored = SceneData::from_ron(&ron_str)
            .expect("RON deserialization failed");
        assert_eq!(scene, restored);
    }

    #[test]
    fn test_version_header_is_present_in_ron() {
        let scene = sample_scene();
        let ron_str = scene.to_ron().unwrap();
        // The very first meaningful field must be the version
        assert!(ron_str.contains("version: 1"));
    }

    #[test]
    fn test_version_header_is_present_in_binary() {
        let scene = sample_scene();
        let binary = scene.to_binary().unwrap();
        // Magic bytes "NSCD" followed by postcard payload that starts
        // with the version encoded as a varint (1 => 0x01)
        assert_eq!(&binary[0..4], b"NSCD");
        // The postcard payload begins at byte 4; version 1 encodes as 0x01
        assert_eq!(binary[4], 0x01);
    }

    #[test]
    fn test_entity_components_are_preserved_ron_roundtrip() {
        let scene = sample_scene();
        let ron_str = scene.to_ron().unwrap();
        let restored = SceneData::from_ron(&ron_str).unwrap();
        assert_eq!(restored.entities.len(), 2);
        assert_eq!(restored.entities[0].components[0].type_name, "WorldPos");
        assert_eq!(
            restored.entities[0].components[0].data,
            ComponentValue::Ron("(x: 0, y: 100, z: 0)".into())
        );
    }

    #[test]
    fn test_parent_child_relationships_maintained_ron() {
        let scene = sample_scene();
        let ron_str = scene.to_ron().unwrap();
        let restored = SceneData::from_ron(&ron_str).unwrap();
        assert_eq!(restored.relationships.len(), 1);
        assert_eq!(restored.relationships[0].parent, 0);
        assert_eq!(restored.relationships[0].child, 1);
    }

    #[test]
    fn test_binary_roundtrip() {
        let scene = sample_scene();
        let binary = scene.to_binary().unwrap();
        let restored = SceneData::from_binary(&binary).unwrap();
        assert_eq!(scene, restored);
    }

    #[test]
    fn test_binary_invalid_magic_rejected() {
        let result = SceneData::from_binary(&[0xFF, 0xFF, 0xFF, 0xFF, 0x01]);
        assert!(matches!(result, Err(SceneSerError::InvalidMagic)));
    }

    #[test]
    fn test_future_version_rejected() {
        let mut scene = sample_scene();
        scene.version = 999;
        let ron_str = scene.to_ron().unwrap();
        let result = SceneData::from_ron(&ron_str);
        assert!(matches!(
            result,
            Err(SceneSerError::FutureVersion { found: 999, .. })
        ));
    }

    #[test]
    fn test_empty_scene_roundtrip() {
        let scene = SceneData {
            version: SCENE_FORMAT_VERSION,
            entities: vec![],
            relationships: vec![],
            prefab_refs: vec![],
        };
        let ron_str = scene.to_ron().unwrap();
        let restored = SceneData::from_ron(&ron_str).unwrap();
        assert_eq!(restored.entities.len(), 0);
        assert_eq!(restored.relationships.len(), 0);
    }
}
```
