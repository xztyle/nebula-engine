# Workspace & Crate Structure

## Problem

A game engine of this scale needs clear module boundaries from day one. Without a well-organized Cargo workspace, compile times balloon, dependency graphs become tangled, and onboarding new contributors (human or AI) becomes painful. Nebula Engine encompasses 128-bit coordinate math, cubesphere-voxel planet rendering, multiplayer networking, procedural terrain, physics, audio, scripting, and editor tooling. If all of this lives in a single crate, incremental compilation suffers, test isolation is impossible, and the public API surface becomes an unnavigable mess. A workspace with narrowly scoped crates enforces separation of concerns at the compiler level, not just by convention.

## Solution

Create a Cargo workspace with the following top-level crates, each in its own directory under the workspace root:

### Binary Crates

- **`nebula-app`** — The binary entry point. Initializes the window, GPU, ECS world, and game loop. Depends on nearly every other crate. Contains the `main()` function and the top-level application orchestration.
- **`nebula-server`** — Headless dedicated server binary. Runs the simulation, networking, and ECS without any rendering or windowing. Shares game logic crates with `nebula-app` but excludes `nebula-render`, `nebula-ui`, `nebula-audio`, and other client-only crates.

### Core Math & Coordinate Crates

- **`nebula-math`** — i128/u128 vector types (`IVec3_128`, `DVec3`), fixed-point arithmetic, conversion utilities between fixed and floating point, and fundamental math operations (lerp, clamp, remap). No external dependencies beyond `std`. This is the foundation everything else builds on.
- **`nebula-coords`** — Coordinate spaces (local, chunk, sector, planet, universe), sector addressing, spatial hashing for broad-phase queries, and transformations between coordinate frames. Depends on `nebula-math`.

### Rendering Crates

- **`nebula-render`** — The wgpu rendering pipeline: surface management, render pass orchestration, shader loading, bind group layouts, and the frame graph. Owns the `GpuContext` struct and all direct wgpu interactions.
- **`nebula-lighting`** — Light types (directional, point, spot), shadow mapping (cascaded shadow maps for terrain), PBR shading calculations, and ambient occlusion integration.
- **`nebula-materials`** — Material system with PBR parameters (albedo, roughness, metallic, normal, emission), texture atlas management for voxel faces, and material palette indexing.
- **`nebula-particles`** — GPU-driven particle systems: emitters, affectors (gravity, wind, turbulence), and billboard/mesh particle rendering.

### Planet & Terrain Crates

- **`nebula-cubesphere`** — Cube-sphere geometry generation: cube-to-sphere projection (tangent-space or normalized), face quadtree subdivision, and UV mapping. Provides the geometric backbone for planetary rendering.
- **`nebula-voxel`** — Voxel storage using palette compression (block IDs stored as indices into a per-chunk palette), chunk data structures (typically 32x32x32), run-length encoding for serialization, and the chunk manager that handles load/unload lifecycle.
- **`nebula-mesh`** — Meshing algorithms: greedy meshing for combining coplanar voxel faces, ambient occlusion vertex calculation, LOD stitching to eliminate T-junctions between chunks at different detail levels, and mesh data structures (vertex buffers, index buffers).
- **`nebula-terrain`** — Procedural terrain generation: multi-octave noise (Perlin, Simplex, Worley), biome assignment, ore/cave distribution, heightmap generation, and the terrain generation pipeline that converts noise into voxel data.
- **`nebula-lod`** — Level-of-detail management: distance-based LOD selection, transition blending, LOD bias configuration, and the LOD quadtree that determines which chunks to render at which detail level.
- **`nebula-planet`** — Planet-level rendering orchestration: atmosphere rendering (Rayleigh/Mie scattering), ocean surface, planetary-scale LOD coordination, and horizon culling.
- **`nebula-space`** — Space rendering: procedural starfield generation, nebula volumetrics, skybox management, and celestial body rendering (distant planets, moons, sun).

### ECS & Game Logic Crates

- **`nebula-ecs`** — Bevy ECS world setup, schedule definitions (Startup, PreUpdate, Update, PostUpdate, Render), core component types (Transform, Visibility, Name), and system registration utilities. This crate configures the ECS but does not contain game-specific logic.
- **`nebula-input`** — Input abstraction layer: keyboard, mouse, and gamepad input mapped through configurable keybindings. Provides an `InputState` resource with action-based queries (e.g., `input.just_pressed(Action::Jump)`) rather than raw key checks.
- **`nebula-player`** — Camera controllers (first-person, third-person, free-fly, orbit), player physics bridge (sends movement intents to physics, reads back position), and player state management.
- **`nebula-physics`** — Rapier physics integration: rigid body management, collision shape generation from voxel meshes, raycasting, and physics world stepping synchronized with the fixed timestep.

### Networking Crates

- **`nebula-net`** — Pure TCP networking layer: connection management, message framing (length-prefixed), serialization/deserialization of network messages using `postcard`, bandwidth tracking, and connection lifecycle (handshake, heartbeat, disconnect).
- **`nebula-multiplayer`** — High-level multiplayer systems: entity replication (server-authoritative), client-side prediction with server reconciliation, interest management (only replicate entities near the player), and lobby/session management.

### Asset & Content Crates

- **`nebula-audio`** — Kira audio integration: sound effect playback, music streaming, spatial audio (3D positioned sounds attenuated by distance), and audio bus management (master, SFX, music, ambient).
- **`nebula-assets`** — Asset loading pipeline: async asset loading, in-memory caching with reference counting, hot-reload file watching in development builds, and support for common formats (PNG, OBJ/glTF, RON, WAV/OGG).
- **`nebula-scene`** — Scene format definition (RON-based), scene serialization/deserialization, save/load game state, and scene hierarchy management.

### UI & Editor Crates

- **`nebula-ui`** — Egui integration for in-game UI: HUD elements (health, coordinates, minimap), pause menu, settings menu, and chat window. Manages the egui render pass and input forwarding.
- **`nebula-editor`** — Editor tools: entity inspector, voxel brush tools, terrain painting, scene hierarchy viewer, and console command interface. Only compiled in development builds.
- **`nebula-debug`** — Debug overlays: FPS counter, frame time graph, chunk boundary visualization, physics collider wireframes, coordinate system gizmos, and integration with `tracy` or `puffin` for profiling.

### Scripting & Animation Crates

- **`nebula-scripting`** — Rhai scripting bridge: expose engine APIs (spawn entity, modify voxel, play sound) to Rhai scripts, script hot-reloading, and a scripting console in the editor.
- **`nebula-animation`** — Skeletal animation: bone hierarchies, keyframe interpolation, animation blending, inverse kinematics (IK) for foot placement on terrain, and animation state machines.

### Workspace Configuration

The root `Cargo.toml` uses the workspace feature:

```toml
[workspace]
resolver = "2"
members = [
    "crates/nebula-app",
    "crates/nebula-math",
    "crates/nebula-coords",
    "crates/nebula-render",
    "crates/nebula-cubesphere",
    "crates/nebula-voxel",
    "crates/nebula-mesh",
    "crates/nebula-ecs",
    "crates/nebula-terrain",
    "crates/nebula-lod",
    "crates/nebula-planet",
    "crates/nebula-space",
    "crates/nebula-lighting",
    "crates/nebula-materials",
    "crates/nebula-input",
    "crates/nebula-player",
    "crates/nebula-physics",
    "crates/nebula-net",
    "crates/nebula-multiplayer",
    "crates/nebula-audio",
    "crates/nebula-assets",
    "crates/nebula-ui",
    "crates/nebula-particles",
    "crates/nebula-animation",
    "crates/nebula-scripting",
    "crates/nebula-scene",
    "crates/nebula-editor",
    "crates/nebula-debug",
    "crates/nebula-server",
]

[workspace.package]
edition = "2024"
version = "0.1.0"
license = "MIT OR Apache-2.0"
repository = "https://github.com/your-org/nebulaengine"

[workspace.dependencies]
# Dependencies declared here, consumed by member crates via:
# [dependencies]
# wgpu = { workspace = true }
```

All shared dependencies are declared in `[workspace.dependencies]` so that version management is centralized. Each member crate's `Cargo.toml` references workspace dependencies with `{ workspace = true }` and inherits the workspace package metadata where appropriate.

### Dependency Flow

The dependency graph flows strictly downward with no cycles:

```
nebula-math (leaf, no engine dependencies)
    |
nebula-coords (depends on nebula-math)
    |
nebula-voxel (depends on nebula-coords, nebula-math)
    |
nebula-mesh (depends on nebula-voxel, nebula-math)
    |
nebula-cubesphere (depends on nebula-math, nebula-coords)
    |
nebula-terrain (depends on nebula-voxel, nebula-cubesphere, nebula-math)
    |
nebula-lod (depends on nebula-coords, nebula-math)
    |
nebula-render (depends on nebula-mesh, nebula-materials, nebula-math)
    |
nebula-planet (depends on nebula-cubesphere, nebula-terrain, nebula-lod, nebula-render)
    |
nebula-app (depends on everything, top of the graph)
```

Crates at the same level may depend on each other only if it does not create a cycle. If two crates need to share types, those types must be factored into a lower-level crate.

## Outcome

Running `cargo check --workspace` succeeds with zero errors. Each crate compiles independently. The dependency graph flows downward (math -> coords -> voxel -> mesh, etc.) with no cycles. Adding a new feature means creating a new crate or extending an existing one without touching unrelated code. Compile times remain manageable because changing `nebula-audio` does not trigger recompilation of `nebula-render`.

## Demo Integration

**Demo crate:** `nebula-demo`

The `nebula-demo` crate is created as a binary in the workspace. Running `cargo run -p nebula-demo` starts and exits cleanly with code 0. The demo exists as the tangible proof that the workspace compiles.

## Crates & Dependencies

- Rust edition **2024**, toolchain **1.93+**
- No external crates yet -- this story is purely about project structure
- The workspace `resolver = "2"` ensures correct feature unification across the workspace

## Unit Tests

- **`test_workspace_compiles`** — Run `cargo check --workspace` and assert exit code 0. This validates that all crate declarations, dependency references, and module structures are syntactically and semantically correct.
- **`test_no_circular_deps`** — Run `cargo tree --workspace` and parse the output to confirm no crate appears as its own transitive dependency. Alternatively, `cargo tree -d` (duplicates) should show no self-references.
- **`test_each_crate_independent`** — For each crate in the workspace, run `cargo check -p <name>` independently and assert exit code 0. This ensures no crate silently depends on another crate being checked first.
- **`test_workspace_members_match_filesystem`** — Compare the `members` list in the root `Cargo.toml` against the actual directories in `crates/` to ensure they are in sync and no crate is missing from the workspace.
