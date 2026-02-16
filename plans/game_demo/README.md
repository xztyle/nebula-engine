# Elite Dangerous Lite - Playable Demo

## Vision
A flyable spaceship orbiting a real-scale cubesphere planet. Fly around in space, approach the planet, descend to the surface, land. No combat, no trading -- just the core flight experience that proves the engine works.

## Planet
- Earth-scale radius (~6,371 km = 6,371,000,000 mm in i128)
- Cubesphere with procedural terrain (noise-based height maps)
- Visible from orbit as a sphere, transitions to voxel terrain on approach
- LOD system handles the scale transition

## Ship
- 6DOF flight controls (already built in player module)
- Newtonian physics in space (thrust, inertia, rotation)
- Flight model: main thrust forward/back, lateral thrusters, rotation on all axes
- Speed indicator, altitude, throttle on HUD

## Stories (ordered by dependency)

### 01 - Clean Game Entry Point
Replace the 5000-line validation demo with a clean game binary. Keep the old demo as `nebula-demo-old` or behind a feature flag. New entry point: create window, init renderer, start game loop.

### 02 - Real-Scale Planet
Configure a cubesphere planet at Earth radius (6,371 km). Procedural terrain with noise. Render as orbital sphere from distance, transition to cubesphere faces on approach. Floating origin keeps precision.

### 03 - Ship Entity & Flight Model
Spawn a ship entity with 6DOF physics. WASD for lateral thrust, Space/Shift for vertical, mouse for rotation. Newtonian: thrust applies force, ship drifts when no input. Configurable max speed and thrust power.

### 04 - Ship Camera
Third-person camera following the ship. Smooth follow with configurable distance/offset. Switch to cockpit (first-person) view with a key.

### 05 - Basic HUD
Minimal wgpu text/line overlay: velocity (m/s), altitude above planet surface, throttle %, heading. No fancy UI framework -- just rendered text or debug lines.

### 06 - Orbit-to-Surface Transition
As ship descends, transition from orbital renderer to cubesphere terrain LOD. The LOD system should kick in and load higher-detail chunks as altitude decreases. Floating origin recenters near the surface.

### 07 - Landing
Detect when ship is near the surface. Simple landing gear logic: reduce speed, touch down, stop. Planet gravity pulls ship down when in atmosphere range.

### 08 - Skybox & Atmosphere
Stars skybox (already exists). Add simple atmospheric scattering effect near the planet -- blue haze at the horizon when close. Doesn't need to be physically accurate, just look decent.

### 09 - Polish & Feel
Thruster particle effects (simple), engine sound placeholder, screen shake on boost. Make it feel like you're flying something.
