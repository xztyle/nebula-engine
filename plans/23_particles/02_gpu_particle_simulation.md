# GPU Particle Simulation

## Problem

Simulating tens of thousands of particles per emitter on the CPU is prohibitively expensive when dozens of emitters are active simultaneously. The CPU must iterate every particle each frame to update position, velocity, age, and liveness — work that scales linearly with particle count and competes with physics, AI, and game logic for frame budget. Meanwhile, the GPU has thousands of cores purpose-built for exactly this kind of massively parallel, data-uniform computation. Moving particle simulation to a compute shader transforms it from a frame-rate bottleneck into a near-free operation that scales to hundreds of thousands of particles.

However, GPU particle simulation introduces complexity: double-buffered particle data (so the compute shader can read from one buffer and write to another without race conditions), atomic counters for tracking alive particle count after culling, and synchronization between the compute pass (simulation) and the render pass (drawing). wgpu 28.0's compute pipeline API handles all of this, but the setup must be carefully structured to avoid pipeline stalls and unnecessary GPU-CPU round trips.

## Solution

Implement a `GpuParticleSimulator` in the `nebula-particles` crate that runs a compute shader each frame to update all particles for an emitter. The simulator uses double-buffered storage buffers and renders particles as camera-facing billboards.

### Particle GPU Layout

Each particle is represented as a contiguous struct in GPU memory:

```wgsl
struct Particle {
    position: vec3<f32>,
    velocity: vec3<f32>,
    age: f32,
    lifetime: f32,
    color: vec4<f32>,
    size: f32,
    alive: u32, // 1 = alive, 0 = dead
};
```

Total: 64 bytes per particle (padded to 16-byte alignment for GPU access patterns).

### Double-Buffered Storage

```rust
pub struct GpuParticleBuffers {
    /// Buffer A: read source on even frames, write target on odd frames.
    pub buffer_a: wgpu::Buffer,
    /// Buffer B: write target on even frames, read source on odd frames.
    pub buffer_b: wgpu::Buffer,
    /// Atomic counter for alive particles after simulation.
    pub alive_counter: wgpu::Buffer,
    /// Simulation parameters uniform (dt, gravity, max_particles).
    pub params_uniform: wgpu::Buffer,
    /// Which buffer is the current read source (toggles each frame).
    pub read_index: u32,
    /// Maximum number of particles this buffer pair can hold.
    pub capacity: u32,
}
```

Each buffer is created with `wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::VERTEX`. The `VERTEX` usage allows the render pass to read directly from the output buffer without an extra copy. The `COPY_DST` usage allows CPU-side writes for initial particle upload.

### Compute Shader

```wgsl
@group(0) @binding(0) var<storage, read> particles_in: array<Particle>;
@group(0) @binding(1) var<storage, read_write> particles_out: array<Particle>;
@group(0) @binding(2) var<storage, read_write> alive_counter: atomic<u32>;
@group(0) @binding(3) var<uniform> params: SimParams;

struct SimParams {
    dt: f32,
    gravity: vec3<f32>,
    max_particles: u32,
};

@compute @workgroup_size(256)
fn cs_simulate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if (idx >= params.max_particles) {
        return;
    }

    var p = particles_in[idx];

    if (p.alive == 0u) {
        particles_out[idx] = p;
        return;
    }

    // Integrate velocity with gravity.
    p.velocity = p.velocity + params.gravity * params.dt;

    // Integrate position.
    p.position = p.position + p.velocity * params.dt;

    // Age the particle.
    p.age = p.age + params.dt;

    // Kill expired particles.
    if (p.age >= p.lifetime) {
        p.alive = 0u;
        particles_out[idx] = p;
        return;
    }

    // Write surviving particle and increment alive counter.
    particles_out[idx] = p;
    atomicAdd(&alive_counter, 1u);
}
```

The workgroup size of 256 is a standard choice that works well across Vulkan, DX12, and Metal. Dispatch count is `ceil(max_particles / 256)`.

### Simulation System

```rust
pub struct GpuParticleSimulator {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    /// One set of buffers per emitter entity.
    emitter_buffers: HashMap<EntityId, GpuParticleBuffers>,
}

impl GpuParticleSimulator {
    pub fn new(device: &wgpu::Device) -> Self { ... }

    /// Upload newly spawned particles from CPU to the current write buffer.
    pub fn upload_new_particles(
        &self,
        queue: &wgpu::Queue,
        entity: EntityId,
        new_particles: &[GpuParticle],
    ) { ... }

    /// Run the compute pass for all active emitters.
    pub fn simulate(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        dt: f32,
        gravity: Vec3,
    ) {
        for (entity, buffers) in &mut self.emitter_buffers {
            // Reset alive counter to 0.
            // Bind read buffer and write buffer based on read_index.
            // Dispatch compute shader.
            // Toggle read_index.
            ...
        }
    }

    /// Get the current read buffer for rendering (contains post-simulation data).
    pub fn get_render_buffer(&self, entity: EntityId) -> Option<&wgpu::Buffer> { ... }

    /// Get the alive particle count (read back from GPU counter).
    pub fn get_alive_count(&self, entity: EntityId) -> u32 { ... }
}
```

### Billboard Rendering

Particles are rendered as camera-facing quads (billboards). The vertex shader expands each particle into a quad:

```wgsl
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_particle(
    @builtin(vertex_index) vertex_id: u32,
    @builtin(instance_index) instance_id: u32,
) -> VertexOutput {
    let particle = particles[instance_id];

    // Quad corners in [-0.5, 0.5] range.
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>(-0.5,  0.5),
        vec2<f32>( 0.5,  0.5),
    );
    let corner = corners[vertex_id];

    // Billboard: offset in camera-right and camera-up directions.
    let world_pos = particle.position
        + camera.right * corner.x * particle.size
        + camera.up * corner.y * particle.size;

    var out: VertexOutput;
    out.position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    out.color = particle.color;
    out.uv = corner + vec2<f32>(0.5, 0.5);
    return out;
}

@fragment
fn fs_particle(in: VertexOutput) -> @location(0) vec4<f32> {
    // Soft circular falloff based on UV distance from center.
    let dist = length(in.uv - vec2<f32>(0.5, 0.5));
    let alpha = smoothstep(0.5, 0.3, dist) * in.color.a;
    return vec4<f32>(in.color.rgb, alpha);
}
```

Rendering uses instanced draw calls: `draw(4_vertices, alive_count_instances)` with a triangle-strip topology. The particle storage buffer is bound as a read-only storage buffer in the vertex shader.

### Synchronization

The compute pass and render pass are recorded in the same command encoder in this order:

1. Compute pass: simulate all emitters (reads buffer A, writes buffer B).
2. Render pass: draw particles from buffer B.

wgpu automatically inserts pipeline barriers between compute writes and vertex reads on the same buffer within the same command encoder submission.

## Outcome

A `GpuParticleSimulator` that runs a WGSL compute shader each frame to update particle position, velocity, age, and liveness on the GPU. Double-buffered storage buffers prevent read/write hazards. An atomic counter tracks alive particle count without GPU-CPU readback. Particles are rendered as camera-facing billboards via instanced draw calls reading directly from the output storage buffer. The system handles 100K+ particles per emitter at negligible CPU cost.

## Demo Integration

**Demo crate:** `nebula-demo`

Thousands of particles are simulated on the GPU via compute shaders. Frame rate remains stable even with high particle counts.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `28.0` | Compute pipelines, storage buffers, render pipelines |
| `glam` | `0.29` | Vec3 for gravity vector and simulation parameters |
| `bytemuck` | `1.21` | Safe casting of particle structs to byte slices for buffer upload |
| `log` | `0.4` | Logging compute dispatch counts and buffer allocations |
| `thiserror` | `2.0` | Error types for buffer creation and shader compilation failures |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula-particles` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a wgpu device and queue for testing (headless).
    fn create_test_gpu() -> (wgpu::Device, wgpu::Queue) {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .expect("no adapter");
            adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("no device")
        })
    }

    #[test]
    fn test_compute_shader_compiles() {
        let (device, _queue) = create_test_gpu();
        let simulator = GpuParticleSimulator::new(&device);
        // If the compute pipeline creation panics, this test fails.
        // A successful construction means the WGSL compiled and the
        // pipeline layout is valid.
        assert!(simulator.emitter_buffers.is_empty());
    }

    #[test]
    fn test_particles_move_over_time() {
        let (device, queue) = create_test_gpu();
        let mut simulator = GpuParticleSimulator::new(&device);
        let entity = EntityId(1);

        // Spawn one particle at origin with velocity (1, 0, 0).
        let particle = GpuParticle {
            position: Vec3::ZERO,
            velocity: Vec3::new(1.0, 0.0, 0.0),
            age: 0.0,
            lifetime: 5.0,
            color: Vec4::ONE,
            size: 1.0,
            alive: 1,
        };
        simulator.allocate_buffers(&device, entity, 1024);
        simulator.upload_new_particles(&queue, entity, &[particle]);

        // Simulate 1 second.
        let mut encoder = device.create_command_encoder(&Default::default());
        simulator.simulate(&device, &mut encoder, 1.0, Vec3::ZERO);
        queue.submit(std::iter::once(encoder.finish()));
        device.poll(wgpu::Maintain::Wait);

        // Read back and verify position moved to approximately (1, 0, 0).
        let readback = simulator.readback_particles(&device, &queue, entity);
        assert!((readback[0].position.x - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_expired_particles_are_removed() {
        let (device, queue) = create_test_gpu();
        let mut simulator = GpuParticleSimulator::new(&device);
        let entity = EntityId(2);

        // Spawn a particle with age nearly at its lifetime.
        let particle = GpuParticle {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            age: 0.9,
            lifetime: 1.0,
            color: Vec4::ONE,
            size: 1.0,
            alive: 1,
        };
        simulator.allocate_buffers(&device, entity, 1024);
        simulator.upload_new_particles(&queue, entity, &[particle]);

        // Simulate 0.2s — particle age becomes 1.1, exceeding lifetime 1.0.
        let mut encoder = device.create_command_encoder(&Default::default());
        simulator.simulate(&device, &mut encoder, 0.2, Vec3::ZERO);
        queue.submit(std::iter::once(encoder.finish()));
        device.poll(wgpu::Maintain::Wait);

        let alive = simulator.get_alive_count(entity);
        assert_eq!(alive, 0, "expired particle should be marked dead");
    }

    #[test]
    fn test_gravity_affects_trajectory() {
        let (device, queue) = create_test_gpu();
        let mut simulator = GpuParticleSimulator::new(&device);
        let entity = EntityId(3);

        // Particle with zero velocity, subject to gravity.
        let particle = GpuParticle {
            position: Vec3::new(0.0, 10.0, 0.0),
            velocity: Vec3::ZERO,
            age: 0.0,
            lifetime: 10.0,
            color: Vec4::ONE,
            size: 1.0,
            alive: 1,
        };
        simulator.allocate_buffers(&device, entity, 1024);
        simulator.upload_new_particles(&queue, entity, &[particle]);

        // Simulate with gravity pulling downward.
        let gravity = Vec3::new(0.0, -9.81, 0.0);
        let mut encoder = device.create_command_encoder(&Default::default());
        simulator.simulate(&device, &mut encoder, 1.0, gravity);
        queue.submit(std::iter::once(encoder.finish()));
        device.poll(wgpu::Maintain::Wait);

        let readback = simulator.readback_particles(&device, &queue, entity);
        // After 1s of gravity, y position should be less than initial 10.0.
        assert!(readback[0].position.y < 10.0);
        // Velocity should be negative in y.
        assert!(readback[0].velocity.y < 0.0);
    }

    #[test]
    fn test_particle_count_stays_within_budget() {
        let (device, queue) = create_test_gpu();
        let mut simulator = GpuParticleSimulator::new(&device);
        let entity = EntityId(4);
        let capacity = 512u32;

        simulator.allocate_buffers(&device, entity, capacity);

        // Upload exactly capacity particles.
        let particles: Vec<GpuParticle> = (0..capacity)
            .map(|_| GpuParticle {
                position: Vec3::ZERO,
                velocity: Vec3::ZERO,
                age: 0.0,
                lifetime: 5.0,
                color: Vec4::ONE,
                size: 1.0,
                alive: 1,
            })
            .collect();
        simulator.upload_new_particles(&queue, entity, &particles);

        let mut encoder = device.create_command_encoder(&Default::default());
        simulator.simulate(&device, &mut encoder, 0.016, Vec3::ZERO);
        queue.submit(std::iter::once(encoder.finish()));
        device.poll(wgpu::Maintain::Wait);

        let alive = simulator.get_alive_count(entity);
        assert!(alive <= capacity, "alive count {} exceeds budget {}", alive, capacity);
    }

    #[test]
    fn test_double_buffer_toggles_each_frame() {
        let (device, _queue) = create_test_gpu();
        let mut simulator = GpuParticleSimulator::new(&device);
        let entity = EntityId(5);
        simulator.allocate_buffers(&device, entity, 256);

        let initial_read = simulator.emitter_buffers[&entity].read_index;

        let mut encoder = device.create_command_encoder(&Default::default());
        simulator.simulate(&device, &mut encoder, 0.016, Vec3::ZERO);

        let after_one = simulator.emitter_buffers[&entity].read_index;
        assert_ne!(initial_read, after_one, "read_index should toggle after simulation");
    }
}
```
