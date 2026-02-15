# Asset Manifest (RON)

## Problem

As the Nebula Engine grows, the number of assets — textures, models, sounds, shaders, materials — reaches hundreds or thousands. Without a central registry, the engine has no way to know what assets exist until code explicitly requests them by path. This makes it impossible to preload assets for a level transition, validate that all referenced assets exist at build time, group related assets for batch loading (e.g., all "menu" assets vs. all "gameplay" assets), or prioritize which assets to load first (e.g., the player model before distant terrain textures).

The engine needs a single manifest file that declares every asset with its path, type, load priority, and group membership. The manifest is parsed once at startup and becomes the authoritative source of truth for asset existence. Loading code never constructs paths manually — it asks the manifest "give me the path for asset named X" or "give me all assets in group Y."

## Solution

### Manifest File Format

The manifest uses RON (Rusty Object Notation), a data format that maps naturally to Rust types and is human-readable and version-control-friendly. A sample `assets.ron`:

```ron
AssetManifest(
    assets: [
        (
            name: "player_model",
            path: "models/player.glb",
            asset_type: Model,
            priority: 0,
            groups: ["gameplay", "core"],
        ),
        (
            name: "terrain_atlas",
            path: "textures/terrain_atlas.png",
            asset_type: Texture,
            priority: 1,
            groups: ["gameplay"],
        ),
        (
            name: "menu_background",
            path: "textures/menu_bg.png",
            asset_type: Texture,
            priority: 0,
            groups: ["menu"],
        ),
        (
            name: "footstep_dirt",
            path: "sounds/footstep_dirt.ogg",
            asset_type: Sound,
            priority: 2,
            groups: ["gameplay", "audio"],
        ),
        (
            name: "terrain_shader",
            path: "shaders/terrain.wgsl",
            asset_type: Shader,
            priority: 0,
            groups: ["gameplay", "core"],
        ),
        (
            name: "stone_material",
            path: "materials/stone.ron",
            asset_type: Material,
            priority: 1,
            groups: ["gameplay"],
        ),
    ],
)
```

### Rust Types

The manifest is deserialized into strongly-typed Rust structs:

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetManifest {
    pub assets: Vec<AssetEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetEntry {
    /// Unique name used to look up this asset.
    pub name: String,
    /// Relative path from the asset root directory.
    pub path: String,
    /// The type of asset (determines which loader to use).
    pub asset_type: AssetType,
    /// Load priority. Lower numbers load first. Default: 0.
    #[serde(default)]
    pub priority: u32,
    /// Groups this asset belongs to. An asset can be in multiple groups.
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetType {
    Texture,
    Model,
    Sound,
    Shader,
    Material,
    Font,
    Animation,
}
```

### Manifest Loading and Indexing

After parsing, the manifest builds lookup indices for O(1) queries:

```rust
pub struct AssetManifestIndex {
    /// The raw parsed manifest.
    manifest: AssetManifest,
    /// Name -> index into manifest.assets
    by_name: HashMap<String, usize>,
    /// Group name -> list of indices into manifest.assets
    by_group: HashMap<String, Vec<usize>>,
    /// Asset type -> list of indices
    by_type: HashMap<AssetType, Vec<usize>>,
}

impl AssetManifestIndex {
    /// Parse a RON file and build the index.
    pub fn load_from_file(path: &Path) -> Result<Self, ManifestError> {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            ManifestError::IoError {
                path: path.to_path_buf(),
                source: e,
            }
        })?;

        Self::load_from_str(&contents, path)
    }

    /// Parse a RON string and build the index.
    pub fn load_from_str(
        ron_str: &str,
        source_path: &Path,
    ) -> Result<Self, ManifestError> {
        let manifest: AssetManifest =
            ron::from_str(ron_str).map_err(|e| ManifestError::ParseError {
                path: source_path.to_path_buf(),
                reason: e.to_string(),
            })?;

        let mut by_name = HashMap::new();
        let mut by_group: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_type: HashMap<AssetType, Vec<usize>> = HashMap::new();

        for (i, entry) in manifest.assets.iter().enumerate() {
            // Check for duplicate names
            if by_name.contains_key(&entry.name) {
                log::warn!(
                    "Duplicate asset name '{}' in manifest — \
                     later entry at index {} shadows earlier one",
                    entry.name, i
                );
            }
            by_name.insert(entry.name.clone(), i);

            for group in &entry.groups {
                by_group.entry(group.clone()).or_default().push(i);
            }

            by_type.entry(entry.asset_type).or_default().push(i);

            // Warn on unknown asset types (this is caught at compile time
            // by the enum, but extensions could add new variants)
        }

        Ok(Self {
            manifest,
            by_name,
            by_group,
            by_type,
        })
    }

    /// Look up an asset entry by its unique name.
    pub fn get_by_name(&self, name: &str) -> Option<&AssetEntry> {
        self.by_name
            .get(name)
            .map(|&i| &self.manifest.assets[i])
    }

    /// Get all asset entries belonging to a group, sorted by priority.
    pub fn get_group(&self, group: &str) -> Vec<&AssetEntry> {
        let mut entries: Vec<&AssetEntry> = self
            .by_group
            .get(group)
            .map(|indices| {
                indices
                    .iter()
                    .map(|&i| &self.manifest.assets[i])
                    .collect()
            })
            .unwrap_or_default();

        entries.sort_by_key(|e| e.priority);
        entries
    }

    /// Get all asset entries of a given type.
    pub fn get_by_type(&self, asset_type: AssetType) -> Vec<&AssetEntry> {
        self.by_type
            .get(&asset_type)
            .map(|indices| {
                indices
                    .iter()
                    .map(|&i| &self.manifest.assets[i])
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all asset entries, sorted by priority (lowest first).
    pub fn all_sorted_by_priority(&self) -> Vec<&AssetEntry> {
        let mut entries: Vec<&AssetEntry> =
            self.manifest.assets.iter().collect();
        entries.sort_by_key(|e| e.priority);
        entries
    }

    /// Total number of declared assets.
    pub fn len(&self) -> usize {
        self.manifest.assets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.manifest.assets.is_empty()
    }

    /// List all known group names.
    pub fn group_names(&self) -> Vec<&str> {
        self.by_group.keys().map(String::as_str).collect()
    }
}
```

### Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read manifest file {path}: {source}")]
    IoError {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to parse manifest at {path}: {reason}")]
    ParseError { path: PathBuf, reason: String },
}
```

### Group Loading Integration

The manifest integrates with the async loading system to load all assets in a group:

```rust
impl AssetManifestIndex {
    /// Kick off loading for every asset in a group. Returns the handles
    /// for all submitted loads, keyed by asset name.
    pub fn load_group(
        &self,
        group: &str,
        asset_root: &Path,
        texture_loader: &AssetLoadingSystem<CpuTexture>,
        texture_store: &mut AssetStore<CpuTexture>,
        model_loader: &AssetLoadingSystem<ModelAsset>,
        model_store: &mut AssetStore<ModelAsset>,
        // ... other loader/store pairs
    ) -> HashMap<String, u64> {
        let mut handles = HashMap::new();

        for entry in self.get_group(group) {
            let full_path = asset_root.join(&entry.path);
            match entry.asset_type {
                AssetType::Texture => {
                    let h = texture_loader.request_load(
                        texture_store,
                        full_path,
                        ImageLoader::color_texture(),
                    );
                    handles.insert(entry.name.clone(), h.id());
                }
                AssetType::Model => {
                    let h = model_loader.request_load(
                        model_store,
                        full_path,
                        GltfLoader,
                    );
                    handles.insert(entry.name.clone(), h.id());
                }
                // ... other types
                _ => {
                    log::warn!(
                        "No loader registered for asset type {:?} (asset '{}')",
                        entry.asset_type, entry.name
                    );
                }
            }
        }

        handles
    }
}
```

### ECS Resource

The manifest index is stored as an ECS resource, available to any system:

```rust
#[derive(Resource)]
pub struct ManifestRes(pub AssetManifestIndex);
```

## Outcome

An `assets.ron` manifest file parsed at startup into an `AssetManifestIndex` with O(1) lookup by name, efficient group queries, and type-filtered listings. Assets declare their path, type, load priority, and group memberships. Groups enable batch loading of related assets (e.g., loading all "gameplay" assets during a loading screen). Priority ordering ensures critical assets (player model, core shaders) load before optional ones (distant decorations). The manifest is the single source of truth — no asset path is hardcoded anywhere else in the engine.

## Demo Integration

**Demo crate:** `nebula-demo`

A `manifest.ron` file lists all assets with paths, types, and load priorities. The asset system uses it to preload critical resources.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `ron` | `0.12` | Parse RON manifest files into Rust types |
| `serde` | `1.0` | Derive `Serialize` and `Deserialize` for manifest types |
| `bevy_ecs` | `0.15` | `Resource` derive for `ManifestRes` |
| `thiserror` | `2.0` | Error type derivation |
| `log` | `0.4` | Warnings for duplicate names and unknown types |

Rust edition 2024. The RON format is chosen over JSON or TOML because it maps directly to Rust enum variants and struct syntax, making the manifest feel natural to Rust developers.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const VALID_MANIFEST: &str = r#"
        AssetManifest(
            assets: [
                (
                    name: "player_model",
                    path: "models/player.glb",
                    asset_type: Model,
                    priority: 0,
                    groups: ["gameplay", "core"],
                ),
                (
                    name: "terrain_atlas",
                    path: "textures/terrain_atlas.png",
                    asset_type: Texture,
                    priority: 1,
                    groups: ["gameplay"],
                ),
                (
                    name: "menu_bg",
                    path: "textures/menu_bg.png",
                    asset_type: Texture,
                    priority: 0,
                    groups: ["menu"],
                ),
                (
                    name: "footstep",
                    path: "sounds/footstep.ogg",
                    asset_type: Sound,
                    priority: 2,
                    groups: ["gameplay", "audio"],
                ),
            ],
        )
    "#;

    #[test]
    fn test_manifest_parses_valid_ron() {
        let result = AssetManifestIndex::load_from_str(
            VALID_MANIFEST,
            Path::new("test_assets.ron"),
        );
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        let index = result.unwrap();
        assert_eq!(index.len(), 4);
    }

    #[test]
    fn test_missing_file_returns_error() {
        let result = AssetManifestIndex::load_from_file(
            Path::new("/nonexistent/path/assets.ron"),
        );
        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ManifestError::IoError { .. }),
            "Missing file should produce IoError"
        );
    }

    #[test]
    fn test_invalid_ron_returns_parse_error() {
        let bad_ron = "this is not valid RON {{{";
        let result = AssetManifestIndex::load_from_str(
            bad_ron,
            Path::new("bad.ron"),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            ManifestError::ParseError { reason, .. } => {
                assert!(
                    !reason.is_empty(),
                    "Parse error should have a reason"
                );
            }
            other => panic!("Expected ParseError, got: {other:?}"),
        }
    }

    #[test]
    fn test_asset_lookup_by_name() {
        let index = AssetManifestIndex::load_from_str(
            VALID_MANIFEST,
            Path::new("test.ron"),
        )
        .unwrap();

        let entry = index.get_by_name("player_model");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.path, "models/player.glb");
        assert_eq!(entry.asset_type, AssetType::Model);
        assert_eq!(entry.priority, 0);

        assert!(index.get_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_group_loading_loads_all_group_assets() {
        let index = AssetManifestIndex::load_from_str(
            VALID_MANIFEST,
            Path::new("test.ron"),
        )
        .unwrap();

        let gameplay = index.get_group("gameplay");
        assert_eq!(
            gameplay.len(),
            3,
            "gameplay group should have player_model, terrain_atlas, footstep"
        );

        let names: Vec<&str> = gameplay.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"player_model"));
        assert!(names.contains(&"terrain_atlas"));
        assert!(names.contains(&"footstep"));
    }

    #[test]
    fn test_priority_ordering_works() {
        let index = AssetManifestIndex::load_from_str(
            VALID_MANIFEST,
            Path::new("test.ron"),
        )
        .unwrap();

        let gameplay = index.get_group("gameplay");
        // Priority 0 should come before priority 1, which comes before 2
        let priorities: Vec<u32> = gameplay.iter().map(|e| e.priority).collect();
        let mut sorted = priorities.clone();
        sorted.sort();
        assert_eq!(
            priorities, sorted,
            "Group assets should be sorted by priority"
        );
    }

    #[test]
    fn test_all_sorted_by_priority() {
        let index = AssetManifestIndex::load_from_str(
            VALID_MANIFEST,
            Path::new("test.ron"),
        )
        .unwrap();

        let all = index.all_sorted_by_priority();
        assert_eq!(all.len(), 4);
        for window in all.windows(2) {
            assert!(
                window[0].priority <= window[1].priority,
                "Assets should be sorted by priority: {} has {} but is before {} with {}",
                window[0].name, window[0].priority,
                window[1].name, window[1].priority,
            );
        }
    }

    #[test]
    fn test_get_by_type() {
        let index = AssetManifestIndex::load_from_str(
            VALID_MANIFEST,
            Path::new("test.ron"),
        )
        .unwrap();

        let textures = index.get_by_type(AssetType::Texture);
        assert_eq!(textures.len(), 2);
        for t in &textures {
            assert_eq!(t.asset_type, AssetType::Texture);
        }

        let models = index.get_by_type(AssetType::Model);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "player_model");
    }

    #[test]
    fn test_empty_manifest() {
        let empty = r#"AssetManifest(assets: [])"#;
        let index = AssetManifestIndex::load_from_str(
            empty,
            Path::new("empty.ron"),
        )
        .unwrap();

        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert!(index.get_group("anything").is_empty());
    }

    #[test]
    fn test_group_names_listed() {
        let index = AssetManifestIndex::load_from_str(
            VALID_MANIFEST,
            Path::new("test.ron"),
        )
        .unwrap();

        let groups = index.group_names();
        assert!(groups.contains(&"gameplay"));
        assert!(groups.contains(&"menu"));
        assert!(groups.contains(&"core"));
        assert!(groups.contains(&"audio"));
    }

    #[test]
    fn test_asset_in_multiple_groups() {
        let index = AssetManifestIndex::load_from_str(
            VALID_MANIFEST,
            Path::new("test.ron"),
        )
        .unwrap();

        // player_model is in both "gameplay" and "core"
        let gameplay_names: Vec<&str> = index
            .get_group("gameplay")
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        let core_names: Vec<&str> = index
            .get_group("core")
            .iter()
            .map(|e| e.name.as_str())
            .collect();

        assert!(gameplay_names.contains(&"player_model"));
        assert!(core_names.contains(&"player_model"));
    }
}
```
