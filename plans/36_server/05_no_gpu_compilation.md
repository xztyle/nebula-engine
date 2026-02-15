# No-GPU Compilation

## Problem

The `nebula-server` binary must compile and run on machines that have no GPU, no GPU drivers, no display server, and no windowing system. This is not a nice-to-have — it is a hard requirement for every realistic server deployment scenario:

- **Cloud VMs** — AWS EC2 `c7a` instances, Hetzner Cloud CPX servers, and DigitalOcean droplets do not have GPUs. They run headless Linux with no X11, no Wayland, and no Mesa/Vulkan drivers installed. If `nebula-server` transitively depends on `wgpu`, it fails to compile because `wgpu-hal` requires a graphics backend (Vulkan/Metal/DX12) at link time.
- **Docker containers** — Minimal container images (e.g., `debian:bookworm-slim`, `alpine:3.20`) do not include `libvulkan.so`, `libX11.so`, or any display libraries. Installing them adds hundreds of megabytes to the image and defeats the purpose of slim containers.
- **CI runners** — GitHub Actions runners, GitLab CI runners, and Jenkins agents are headless. If the server crate depends on windowing or GPU crates, `cargo build -p nebula-server` fails in CI unless GPU libraries are installed, adding fragile system dependencies.
- **Embedded / ARM servers** — Raspberry Pi or ARM-based edge servers may not have Vulkan drivers at all. The server binary must compile with `cargo build --target aarch64-unknown-linux-gnu` without GPU dependencies.

The solution is architectural: rendering code lives in rendering crates, shared logic lives in shared crates, and the `nebula-server` binary depends only on shared crates. Cargo features are used as an additional safety net at the workspace level.

## Solution

### Crate-Level Separation (Primary Mechanism)

The primary mechanism for excluding GPU code is crate-level dependency boundaries, not feature flags. The workspace is already structured (see `01_setup/01_workspace_and_crate_structure.md`) so that rendering crates are separate from logic crates:

```
crates/
├── nebula-math/          # Pure Rust math, no GPU
├── nebula-coords/        # Coordinate systems, no GPU
├── nebula-voxel/         # Voxel storage, no GPU
├── nebula-terrain/       # Terrain generation, no GPU
├── nebula-physics/       # Physics (Rapier), no GPU
├── nebula-ecs/           # Bevy ECS setup, no GPU
├── nebula-net/           # TCP networking, no GPU
├── nebula-multiplayer/   # Replication/prediction, no GPU
├── nebula-mesh/          # Mesh data structures, no GPU
│                         #   (vertex/index buffers as CPU-side data)
│
├── nebula-render/        # wgpu pipelines, GPU REQUIRED
├── nebula-lighting/      # Shadow maps, GPU REQUIRED
├── nebula-materials/     # Texture atlas, GPU REQUIRED
├── nebula-particles/     # GPU particles, GPU REQUIRED
├── nebula-ui/            # egui integration, GPU REQUIRED
├── nebula-audio/         # Audio playback, AUDIO REQUIRED
├── nebula-input/         # OS input events, WINDOW REQUIRED
│
├── nebula-app/           # Client binary (depends on everything)
└── nebula-server/        # Server binary (depends only on shared crates)
```

The `nebula-server/Cargo.toml` simply does not list `nebula-render`, `nebula-lighting`, `nebula-materials`, `nebula-particles`, `nebula-ui`, `nebula-audio`, or `nebula-input` as dependencies. No feature flag needed — the crates are not in the dependency graph at all.

### Workspace Features (Safety Net)

As an additional safeguard, the workspace `Cargo.toml` defines features that make the boundary explicit:

```toml
# Workspace root Cargo.toml

[workspace]
members = [
    "crates/nebula-app",
    "crates/nebula-server",
    "crates/nebula-math",
    "crates/nebula-coords",
    "crates/nebula-voxel",
    # ... all crates
]

[workspace.metadata.features]
# These are documentation-only; actual feature gates are per-crate.
# The workspace-level features document which binary includes what.
client = ["nebula-app"]
server = ["nebula-server"]
```

### Shared Crate Rules

Every shared crate (used by both client and server) must follow these rules:

1. **No `wgpu` dependency** — Not even optional. If a crate needs GPU upload, that code belongs in `nebula-render`, not in the shared crate.
2. **No `winit` dependency** — Shared crates must not depend on windowing or OS event loops.
3. **No `egui` dependency** — UI code belongs in `nebula-ui`.
4. **No conditional compilation for server vs. client** — Shared crates compile identically for both. No `#[cfg(feature = "server")]` sprinkled throughout. The code is the same; the difference is which binary links it.
5. **`nebula-mesh` provides data, not GPU buffers** — The mesh crate defines `MeshData` (vertices, indices, normals as `Vec<f32>`) but does not create `wgpu::Buffer` objects. GPU upload is handled by `nebula-render`.

```rust
// nebula-mesh/src/lib.rs — this is SHARED code, no GPU dependency

/// CPU-side mesh data. Can be used by the server for collision
/// or by the client for GPU upload.
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}
```

### CI Validation

The CI pipeline (see `01_setup/02_ci_pipeline.md`) runs two critical checks:

```yaml
# .github/workflows/ci.yml

jobs:
  build-server:
    runs-on: ubuntu-latest  # No GPU, no display server
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build server binary
        run: cargo build -p nebula-server

      - name: Verify no GPU crates in dependency tree
        run: |
          cargo tree -p nebula-server --no-indent | grep -qvE 'wgpu|winit|egui|naga|gpu-allocator|raw-window-handle' \
            || (echo "ERROR: GPU/window crate found in nebula-server dependency tree" && cargo tree -p nebula-server | grep -E 'wgpu|winit|egui|naga' && exit 1)

      - name: Run server tests
        run: cargo test -p nebula-server

  build-client:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install GPU/window system dependencies
        run: sudo apt-get install -y libvulkan-dev libwayland-dev libxkbcommon-dev
      - name: Build client binary
        run: cargo build -p nebula-app
```

The server build job does not install any GPU or windowing libraries. If `nebula-server` gains a transitive dependency on wgpu or winit, the build fails immediately.

### Dependency Tree Verification Script

A script in the repository verifies the separation:

```bash
#!/bin/bash
# scripts/verify_server_deps.sh

echo "Checking nebula-server dependency tree for GPU/window crates..."

FORBIDDEN_CRATES="wgpu wgpu-core wgpu-hal wgpu-types naga gpu-allocator gpu-descriptor winit raw-window-handle egui egui-wgpu kira cpal rodio"

TREE=$(cargo tree -p nebula-server --no-indent 2>/dev/null)

FOUND=0
for crate in $FORBIDDEN_CRATES; do
    if echo "$TREE" | grep -q "^${crate} "; then
        echo "FAIL: Found forbidden crate: $crate"
        FOUND=1
    fi
done

if [ $FOUND -eq 0 ]; then
    echo "PASS: No GPU/window/audio crates in nebula-server dependency tree"
else
    echo ""
    echo "Full dependency tree:"
    cargo tree -p nebula-server
    exit 1
fi
```

### Handling Edge Cases

**`nebula-physics` (Rapier):** Rapier is a pure CPU physics engine. It has no GPU dependency. It compiles on headless machines without issue.

**`nebula-terrain` (noise generation):** Terrain generation uses CPU-based noise functions. No GPU compute is involved.

**`nebula-mesh` (mesh data):** The mesh crate provides CPU-side data structures. The GPU upload path is in `nebula-render`, which the server does not depend on. The server uses `MeshData` only for generating collision geometry.

**Platform-specific I/O:** The server uses `tokio::net::TcpListener` for networking and `std::io::stdin` for admin input. Neither requires a display server. The server can run over SSH, in `tmux`, in a Docker container, or as a systemd service.

## Outcome

The `nebula-server` binary compiles and runs on any machine that has a Rust toolchain, regardless of GPU, display server, or windowing system availability. The dependency tree contains zero rendering, windowing, UI, or audio crates. CI validates this on every commit by building `nebula-server` on a headless Ubuntu runner without GPU libraries installed. A verification script checks the dependency tree for forbidden crates. Shared crates follow strict rules to avoid introducing transitive GPU dependencies. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The server binary's dependency tree excludes wgpu, winit, egui, and all rendering crates. It compiles on a headless Linux server without GPU drivers installed.

## Crates & Dependencies

No new external crates are introduced by this story. The separation is achieved through architecture (crate-level dependency boundaries) and CI enforcement, not through additional dependencies.

Crates verified to be absent from `nebula-server`'s dependency tree:

| Forbidden Crate | Reason |
|-----------------|--------|
| `wgpu`, `wgpu-core`, `wgpu-hal`, `wgpu-types` | GPU rendering backend |
| `naga` | Shader compilation |
| `gpu-allocator`, `gpu-descriptor` | GPU memory management |
| `winit` | Windowing and event loop |
| `raw-window-handle` | Window handle abstraction |
| `egui`, `egui-wgpu` | Immediate-mode UI |
| `kira`, `cpal`, `rodio` | Audio playback |

## Unit Tests

- **`test_server_builds_without_wgpu_in_dependency_tree`** — Run `cargo tree -p nebula-server --no-indent` and parse the output. Assert that no line starts with `wgpu`, `wgpu-core`, `wgpu-hal`, or `wgpu-types`. This is a build-time integration test that catches transitive dependency leaks.

- **`test_server_builds_without_winit`** — Same as above but checking for `winit` and `raw-window-handle` in the dependency tree output.

- **`test_server_runs_without_display_server`** — Run the server binary in a subprocess with `DISPLAY` and `WAYLAND_DISPLAY` environment variables explicitly unset. Assert the process starts successfully (exit code 0 after receiving SIGTERM, or connects to a TCP port within 2 seconds). This validates that no code path attempts to open a window or connect to a display server.

- **`test_shared_crates_compile_under_both_targets`** — Run `cargo check -p nebula-math -p nebula-coords -p nebula-voxel -p nebula-terrain -p nebula-physics -p nebula-ecs -p nebula-net -p nebula-multiplayer -p nebula-mesh` with no features. Assert all compile successfully. Then run `cargo check -p nebula-app` (which includes rendering). Assert it also compiles. This validates that shared crates are truly shared and not biased toward either target.

- **`test_no_dead_code_warnings_in_server_build`** — Run `cargo build -p nebula-server 2>&1` and parse stderr. Assert no lines contain `warning: unused` or `warning: dead_code` related to shared crate code. Dead code warnings would indicate that shared crates contain client-only code behind no feature gate, which is a design violation.
