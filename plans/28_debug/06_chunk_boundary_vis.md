# Chunk Boundary Visualization

## Problem

The voxel terrain system manages hundreds of chunks simultaneously, each at different LOD levels, in various lifecycle states (queued for generation, generating, meshing, ready, dirty, pending unload). When terrain bugs occur — chunks not loading, visible seams between LOD levels, chunks loading at wrong positions, chunks stuck in a generating state — the developer cannot see the chunk management state just by looking at the rendered terrain. The rendered surface hides the underlying chunk grid entirely.

Specific scenarios that require chunk boundary visualization:

- **LOD transition debugging** — When the camera moves, chunks transition between LOD levels. If a chunk at LOD 0 (full detail) is adjacent to a chunk at LOD 2 (quarter detail), the stitching algorithm must fill the gap. Seeing the LOD level of each chunk as a color makes transition issues immediately apparent.
- **Load/unload radius tuning** — The engine loads chunks within a radius around the camera and unloads chunks beyond a larger radius. If these radii are wrong, the player sees terrain pop in or the system loads too many chunks and runs out of memory. Visualizing the load/unload radii as wireframe spheres provides immediate feedback.
- **Dirty chunk tracking** — When a voxel is modified, the containing chunk (and sometimes its neighbors) must be re-meshed. If dirty chunks are not being processed, the player sees stale geometry. Highlighting dirty chunks makes the re-meshing pipeline's state visible.
- **Generation pipeline stalls** — If chunks are stuck in the "generating" state, the terrain generation threadpool might be saturated or deadlocked. Color-coding chunk states makes stalls visible as a sea of orange (generating) chunks instead of green (ready) ones.

## Solution

### Chunk State Data

The visualization reads from the existing chunk manager's state. Each loaded chunk has:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChunkState {
    Queued,       // Scheduled for generation, not yet started
    Generating,   // Terrain generation in progress on a background thread
    Meshing,      // Voxel data ready, mesh generation in progress
    Ready,        // Fully loaded and meshed, ready for rendering
    Dirty,        // Modified, needs re-meshing
    Unloading,    // Scheduled for removal
}

pub struct ChunkDebugInfo {
    pub position: ChunkPos,
    pub state: ChunkState,
    pub lod_level: u8,      // 0 = full detail, higher = less detail
    pub vertex_count: u32,  // For diagnostics
    pub voxel_count: u32,   // Non-air voxels
}
```

### Color Schemes

#### LOD Color Coding

Each LOD level gets a distinct, easily distinguishable color:

| LOD Level | Color       | RGB                  |
|-----------|-------------|----------------------|
| 0         | White       | (1.0, 1.0, 1.0, 0.5) |
| 1         | Cyan        | (0.0, 1.0, 1.0, 0.5) |
| 2         | Green       | (0.0, 1.0, 0.0, 0.5) |
| 3         | Yellow      | (1.0, 1.0, 0.0, 0.5) |
| 4         | Orange      | (1.0, 0.6, 0.0, 0.5) |
| 5+        | Red         | (1.0, 0.0, 0.0, 0.5) |

#### State Color Coding

When viewing by state (togglable mode):

| State      | Color        | RGB                  |
|------------|--------------|----------------------|
| Queued     | Gray         | (0.5, 0.5, 0.5, 0.4) |
| Generating | Orange       | (1.0, 0.6, 0.0, 0.6) |
| Meshing    | Yellow       | (1.0, 1.0, 0.0, 0.6) |
| Ready      | Green        | (0.0, 1.0, 0.0, 0.3) |
| Dirty      | Red (pulsing)| (1.0, 0.0, 0.0, 0.5-0.8) |
| Unloading  | Dark gray    | (0.3, 0.3, 0.3, 0.4) |

Dirty chunks pulse their alpha between 0.5 and 0.8 using a sine wave based on elapsed time, making them visually stand out.

### Wireframe Box Rendering

Each chunk boundary is rendered as 12 line segments forming a wireframe box:

```rust
pub fn generate_chunk_wireframe(
    chunk_pos: &ChunkPos,
    chunk_size: f32,
    color: [f32; 4],
) -> Vec<DebugLine> {
    let min = [
        chunk_pos.x as f32 * chunk_size,
        chunk_pos.y as f32 * chunk_size,
        chunk_pos.z as f32 * chunk_size,
    ];
    let max = [
        min[0] + chunk_size,
        min[1] + chunk_size,
        min[2] + chunk_size,
    ];

    let corners = [
        [min[0], min[1], min[2]], // 0: ---
        [max[0], min[1], min[2]], // 1: +--
        [max[0], max[1], min[2]], // 2: ++-
        [min[0], max[1], min[2]], // 3: -+-
        [min[0], min[1], max[2]], // 4: --+
        [max[0], min[1], max[2]], // 5: +-+
        [max[0], max[1], max[2]], // 6: +++
        [min[0], max[1], max[2]], // 7: -++
    ];

    // 12 edges of a box
    let edge_indices = [
        (0,1),(1,2),(2,3),(3,0), // bottom face
        (4,5),(5,6),(6,7),(7,4), // top face
        (0,4),(1,5),(2,6),(3,7), // vertical edges
    ];

    edge_indices
        .iter()
        .map(|&(a, b)| DebugLine {
            start: corners[a],
            end: corners[b],
            color,
        })
        .collect()
}
```

### Load/Unload Radius Spheres

The load and unload radii are rendered as wireframe spheres centered on the camera position. The spheres use latitude/longitude line generation with configurable segment counts:

```rust
pub fn generate_wireframe_sphere(
    center: [f32; 3],
    radius: f32,
    segments: u32,
    color: [f32; 4],
) -> Vec<DebugLine> {
    let mut lines = Vec::new();

    // Generate latitude circles
    for i in 0..segments {
        let phi = std::f32::consts::PI * (i as f32 / segments as f32);
        let r = radius * phi.sin();
        let y = center[1] + radius * phi.cos();

        for j in 0..segments {
            let theta0 = 2.0 * std::f32::consts::PI * (j as f32 / segments as f32);
            let theta1 = 2.0 * std::f32::consts::PI * ((j + 1) as f32 / segments as f32);

            lines.push(DebugLine {
                start: [center[0] + r * theta0.cos(), y, center[2] + r * theta0.sin()],
                end: [center[0] + r * theta1.cos(), y, center[2] + r * theta1.sin()],
                color,
            });
        }
    }

    // Generate longitude circles similarly
    // ...

    lines
}
```

The load radius sphere is green, and the unload radius sphere is red, making it visually obvious where chunks will appear and disappear.

### Per-Chunk Label

When the camera is close enough to a chunk (within a configurable distance, default 3 chunk widths), the chunk's position, LOD, and state are rendered as a small text label at the chunk center using egui's `painter.text()` in screen space. This provides detailed per-chunk information without cluttering the distant view.

### System Integration

```rust
fn draw_chunk_boundaries(
    mut egui_ctx: ResMut<EguiContext>,
    chunk_manager: Res<ChunkManager>,
    camera: Query<&Transform, With<MainCamera>>,
    debug_state: Res<ChunkDebugState>,
    mut line_buffer: ResMut<DebugLineBuffer>,
) {
    if !debug_state.visible {
        return;
    }

    let cam_pos = camera.single().translation;

    for chunk_info in chunk_manager.loaded_chunks_debug_info() {
        let color = match debug_state.color_mode {
            ColorMode::ByLod => lod_color(chunk_info.lod_level),
            ColorMode::ByState => state_color(chunk_info.state),
        };

        let lines = generate_chunk_wireframe(
            &chunk_info.position,
            chunk_manager.chunk_size(),
            color,
        );
        line_buffer.lines.extend(lines);
    }

    // Load/unload radius spheres
    line_buffer.lines.extend(generate_wireframe_sphere(
        [cam_pos.x, cam_pos.y, cam_pos.z],
        chunk_manager.load_radius(),
        24,
        [0.0, 1.0, 0.0, 0.3],
    ));
    line_buffer.lines.extend(generate_wireframe_sphere(
        [cam_pos.x, cam_pos.y, cam_pos.z],
        chunk_manager.unload_radius(),
        24,
        [1.0, 0.0, 0.0, 0.3],
    ));
}
```

### Toggle

F6 toggles chunk boundary visibility. When visible, pressing F6 again cycles through color modes: `ByLod -> ByState -> Off`.

## Outcome

Pressing F6 renders wireframe boxes at every loaded chunk position, color-coded by LOD level or chunk state. Dirty chunks pulse red. The load radius appears as a green wireframe sphere and the unload radius as a red sphere, both centered on the camera. Nearby chunks display text labels with position, LOD, and state. The visualization cycles through LOD color mode, state color mode, and off. Implementation lives in `crates/nebula-debug/src/chunk_debug.rs` and shares the `DebugLineBuffer` with the physics debug visualization.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunk boundaries are drawn as wireframe boxes in world space. Different LOD levels use different colors. The quadtree structure is visible as nested boxes.

## Crates & Dependencies

- **`wgpu = "28.0"`** — Line rendering pipeline for chunk wireframe boxes and radius spheres, shared with other debug visualization systems via the common `DebugLineBuffer`.
- **`egui = "0.31"`** — Per-chunk text labels rendered in screen space, and the color mode toggle indicator in the debug overlay.
- **`tracing = "0.1"`** — Logging chunk boundary visualization toggle events and chunk state statistics (e.g., "Showing 247 chunks: 180 ready, 30 generating, 12 meshing, 25 dirty").

## Unit Tests

- **`test_chunk_boundaries_align_with_positions`** — Generate a wireframe box for chunk at `ChunkPos(2, 3, -1)` with chunk size 32. Assert the minimum corner is at `(64.0, 96.0, -32.0)` and the maximum corner is at `(96.0, 128.0, 0.0)`. Verify all 12 edges connect the correct corners.

- **`test_lod_colors_are_distinct`** — Call `lod_color` for LOD levels 0 through 5. Assert each returned color is different from all others. Specifically verify: LOD 0 is white, LOD 1 is cyan, LOD 2 is green, LOD 3 is yellow, LOD 4 is orange, LOD 5 is red. No two colors share the same RGB values.

- **`test_dirty_chunks_are_highlighted`** — Call `state_color(ChunkState::Dirty)`. Assert the returned color is red (R > 0.8, G < 0.2, B < 0.2). Verify the alpha component changes over time by evaluating at two different timestamps (simulating the pulsing effect) and asserting the alpha values differ.

- **`test_load_radius_sphere_visible`** — Generate a wireframe sphere with radius 256.0 and 16 segments. Assert the output contains more than 0 lines. Verify at least one line endpoint is at distance approximately 256.0 from the center (within floating-point tolerance). Assert all line endpoints are at distance between 0.0 and 256.1 from the center.

- **`test_toggling_works`** — Create a `ChunkDebugState` with `visible: false`. Simulate F6 press: assert state is `visible: true, color_mode: ByLod`. Simulate F6 again: assert `visible: true, color_mode: ByState`. Simulate F6 again: assert `visible: false`.

- **`test_only_loaded_chunks_show_boundaries`** — Create a mock `ChunkManager` with 5 loaded chunks and 3 unloaded (recently removed) chunks. Run the visualization system. Assert exactly 5 wireframe boxes are generated (60 lines: 12 edges per box). Confirm no lines correspond to the unloaded chunk positions.

- **`test_wireframe_box_has_12_edges`** — Generate a wireframe box for any chunk position. Assert the output contains exactly 12 `DebugLine` entries. Verify each line connects two distinct corner points and no edge is duplicated.
