# Coordinate Display

## Problem

Nebula Engine uses a multi-layered coordinate system to handle planetary-scale worlds with voxel precision: 128-bit world positions, sector addressing, planet-relative coordinates (latitude/longitude/altitude), chunk addresses, and local f32 positions for rendering. When something looks wrong — a chunk loads at the wrong position, terrain generates with an offset, a player teleports to an unexpected location, multiplayer desync puts clients in different sectors — the developer needs to see the raw coordinate values to diagnose the issue.

Without a coordinate display:

- **Sector boundary bugs are invisible** — The camera might be at sector (1, 0, 0) but the terrain system thinks it is at (0, 0, 0). Without seeing both values simultaneously, this discrepancy goes unnoticed.
- **128-bit coordinate overflow is silent** — If an i128 addition wraps, the resulting position could be billions of units away. Seeing the raw i128 values makes overflow immediately obvious.
- **Planet coordinate mapping is unverifiable** — Is the player actually at latitude 45.3, longitude -122.7, altitude 1200m? Without a display, verifying the cube-to-sphere projection and coordinate conversion is tedious.
- **Chunk address mismatches cause subtle bugs** — If the chunk address display says (4, 2, -1) but the terrain system loaded chunk (4, 2, -2), the off-by-one is caught instantly.

This overlay is the single most important debugging tool for spatial reasoning in a 128-bit coordinate engine.

## Solution

### Coordinate Data Collection

A `CoordinateDisplay` resource aggregates all coordinate representations of the camera/player position each frame:

```rust
pub struct CoordinateDisplay {
    /// Full-precision world position (i128 per axis)
    pub world_pos: IVec3_128,
    /// Sector index and local offset within the sector
    pub sector: SectorAddress,
    pub sector_local: DVec3,
    /// Planet-relative coordinates (only populated when near a planet)
    pub planet_coords: Option<PlanetCoords>,
    /// Chunk address (sector-relative)
    pub chunk_address: ChunkPos,
    /// Local f32 position used for rendering (camera-relative)
    pub local_pos: Vec3,
    /// Whether the overlay is visible
    pub visible: bool,
}

pub struct PlanetCoords {
    pub planet_name: String,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub altitude_m: f64,
    pub face: CubeFace,        // Which cubesphere face the player is on
}

pub struct SectorAddress {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}
```

### Coordinate Computation

Each frame, a system reads the camera transform and computes all representations:

```rust
fn update_coordinate_display(
    camera: Query<&Transform, With<MainCamera>>,
    world_origin: Res<WorldOrigin>,   // The i128 origin the f32 camera is relative to
    sector_map: Res<SectorMap>,
    planet_query: Query<(&Planet, &Transform)>,
    mut display: ResMut<CoordinateDisplay>,
) {
    let camera_transform = camera.single();

    // 1. World position: origin (i128) + camera local offset (f32 -> i128)
    display.world_pos = world_origin.position + IVec3_128::from_f32(camera_transform.translation);

    // 2. Sector address: divide world position by sector size
    let sector_size: i128 = 1 << 40; // Example: 2^40 units per sector
    display.sector = SectorAddress {
        x: (display.world_pos.x / sector_size) as i64,
        y: (display.world_pos.y / sector_size) as i64,
        z: (display.world_pos.z / sector_size) as i64,
    };
    display.sector_local = DVec3::new(
        (display.world_pos.x % sector_size) as f64,
        (display.world_pos.y % sector_size) as f64,
        (display.world_pos.z % sector_size) as f64,
    );

    // 3. Chunk address: divide sector-local by chunk size
    let chunk_size: i128 = 32;
    display.chunk_address = ChunkPos {
        x: (display.sector_local.x as i128 / chunk_size) as i32,
        y: (display.sector_local.y as i128 / chunk_size) as i32,
        z: (display.sector_local.z as i128 / chunk_size) as i32,
    };

    // 4. Local f32 position (already available from camera transform)
    display.local_pos = camera_transform.translation;

    // 5. Planet coordinates (if near a planet)
    display.planet_coords = None;
    for (planet, planet_transform) in planet_query.iter() {
        let distance = camera_transform.translation.distance(planet_transform.translation);
        if distance < planet.atmosphere_radius {
            let relative = camera_transform.translation - planet_transform.translation;
            let (lat, lon) = cartesian_to_lat_lon(relative);
            let altitude = relative.length() - planet.surface_radius;
            let face = cubesphere_face_from_direction(relative.normalize());

            display.planet_coords = Some(PlanetCoords {
                planet_name: planet.name.clone(),
                latitude_deg: lat.to_degrees(),
                longitude_deg: lon.to_degrees(),
                altitude_m: altitude as f64,
                face,
            });
            break;
        }
    }
}
```

### Overlay Rendering

The coordinate display is rendered as a compact text block in the debug panel using egui:

```rust
fn draw_coordinate_display(
    mut egui_ctx: ResMut<EguiContext>,
    display: Res<CoordinateDisplay>,
) {
    if !display.visible {
        return;
    }

    egui::Area::new(egui::Id::new("coord_display"))
        .fixed_pos(egui::pos2(8.0, 100.0)) // Below the FPS counter
        .show(egui_ctx.get_mut(), |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_black_alpha(180))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);

                    ui.label(format!(
                        "World: ({}, {}, {})",
                        display.world_pos.x,
                        display.world_pos.y,
                        display.world_pos.z,
                    ));
                    ui.label(format!(
                        "Sector: ({}, {}, {})",
                        display.sector.x,
                        display.sector.y,
                        display.sector.z,
                    ));
                    ui.label(format!(
                        "Sector Local: ({:.2}, {:.2}, {:.2})",
                        display.sector_local.x,
                        display.sector_local.y,
                        display.sector_local.z,
                    ));
                    ui.label(format!(
                        "Chunk: ({}, {}, {})",
                        display.chunk_address.x,
                        display.chunk_address.y,
                        display.chunk_address.z,
                    ));
                    ui.label(format!(
                        "Local: ({:.3}, {:.3}, {:.3})",
                        display.local_pos.x,
                        display.local_pos.y,
                        display.local_pos.z,
                    ));

                    if let Some(ref planet) = display.planet_coords {
                        ui.separator();
                        ui.label(format!("Planet: {}", planet.planet_name));
                        ui.label(format!(
                            "Lat/Lon: {:.4}, {:.4}",
                            planet.latitude_deg,
                            planet.longitude_deg,
                        ));
                        ui.label(format!("Alt: {:.1} m", planet.altitude_m));
                        ui.label(format!("Face: {:?}", planet.face));
                    }
                });
        });
}
```

### Display Format

The overlay appears as a monospace text block:

```
World: (1099511627776, 42, -549755813888)
Sector: (1, 0, -1)
Sector Local: (0.00, 42.00, 549755813888.00)
Chunk: (0, 1, -3)
Local: (0.123, 42.456, -78.901)
---
Planet: Terra
Lat/Lon: 45.3012, -122.6789
Alt: 1247.3 m
Face: PosX
```

The world position uses full i128 display (no truncation, no scientific notation) because truncated values defeat the purpose of 128-bit coordinates. The monospace font ensures columns align for easy scanning.

### Toggle

The coordinate display shares the F3 toggle with the FPS counter (they are both part of the "debug info overlay" concept), or it can be given a separate toggle (F5). The `visible` flag is controlled by the same input system.

## Outcome

A compact monospace text overlay displays the camera position in five coordinate formats simultaneously: world (i128), sector (index + local offset), chunk address, local (f32), and planet-relative (lat/lon/alt) when near a planet. All values update every frame. The overlay is toggled with the debug key and positioned below the FPS counter. The implementation lives in `crates/nebula-debug/src/coordinate_display.rs` and depends on `nebula-coords` for coordinate conversions and `nebula-ui` for egui rendering.

## Demo Integration

**Demo crate:** `nebula-demo`

A panel shows all coordinate representations simultaneously: WorldPosition (i128), sector address, chunk address, local f32, latitude/longitude/altitude — all updating in real time.

## Crates & Dependencies

- **`egui = "0.31"`** — Rendering the coordinate text block with monospace font, semi-transparent background, and fixed positioning.
- **`tracing = "0.1"`** — Logging coordinate display toggle events and any coordinate conversion warnings (e.g., overflow detection during i128 arithmetic).

Internal crate dependencies (not external):
- `nebula-math` for `IVec3_128`, `DVec3`, `Vec3`.
- `nebula-coords` for `SectorAddress`, `ChunkPos`, `cartesian_to_lat_lon`, `cubesphere_face_from_direction`.
- `nebula-cubesphere` for `CubeFace` enum.

## Unit Tests

- **`test_world_position_matches_camera`** — Set the `WorldOrigin` to `IVec3_128(1000, 2000, 3000)` and the camera local position to `Vec3(1.5, -0.5, 2.0)`. Run the update system. Assert `display.world_pos` equals `IVec3_128(1001, 1999, 3002)` (origin + truncated local offset). Verify rounding behavior is documented and consistent.

- **`test_sector_computed_correctly`** — Set `world_pos` to `IVec3_128(2^40 + 100, -(2^40), 0)`. Assert `sector` is `(1, -1, 0)` and `sector_local` is `(100, 0, 0)`. Test boundary cases: position exactly on a sector boundary should report sector N with local offset 0, not sector N-1 with local offset = sector_size.

- **`test_planet_coords_shown_when_near_planet`** — Place a planet at the origin with `atmosphere_radius = 10000.0` and `surface_radius = 6371.0`. Place the camera at `(0, 6400, 0)` (above the north pole). Run the update. Assert `planet_coords` is `Some`, `latitude_deg` is approximately `90.0`, `altitude_m` is approximately `29.0` (6400 - 6371). Move the camera to `(20000, 0, 0)` (outside atmosphere). Assert `planet_coords` is `None`.

- **`test_chunk_address_matches_current_chunk`** — Set `sector_local` to `DVec3(65.0, 33.0, -10.0)` with a chunk size of 32. Assert `chunk_address` is `(2, 1, -1)`. Test edge case: position at exactly `(32.0, 0.0, 0.0)` should be chunk `(1, 0, 0)`, not `(0, 0, 0)`.

- **`test_all_formats_update_each_frame`** — Run the update system for two consecutive frames with different camera positions. Assert all five coordinate representations (`world_pos`, `sector`, `chunk_address`, `local_pos`, `planet_coords`) changed between frames. This verifies that stale data from the previous frame does not persist.

- **`test_large_coordinates_no_overflow`** — Set `world_pos` to near `i128::MAX / 2` on one axis. Run the update system. Assert no panic occurs and the sector address is computed correctly using i128 division without overflow.

- **`test_negative_coordinates`** — Set `world_pos` to `IVec3_128(-1000, -2000, -3000)`. Assert the sector address is correctly negative (e.g., `(-1, -1, -1)` for appropriate sector size) and the sector local offset is positive (representing position within the sector, not a negative offset).
