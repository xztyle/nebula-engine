# Memory Usage Tracking

## Problem

A voxel game engine with planetary-scale worlds is inherently memory-hungry. Chunk voxel data, mesh vertex/index buffers, texture atlases, entity component storage, audio samples, and physics collider geometry all compete for the same finite RAM. Without memory tracking:

- **Memory leaks are invisible until the OS kills the process** — A chunk that fails to deallocate its mesh buffer on unload leaks megabytes per occurrence. After 30 minutes of exploring, the process has consumed 8GB and the OS terminates it. Without per-subsystem tracking, the leak could be anywhere.
- **Budget allocation is guesswork** — How much memory should be reserved for chunk data? For textures? For physics? Without measurements, budget decisions are arbitrary and the first subsystem to allocate aggressively starves the others.
- **Peak usage is unknowable** — The average memory might be 2GB, but during a sector transition (when both old and new chunks are loaded simultaneously), it might spike to 4GB. Without peak tracking, this transient spike causes out-of-memory crashes that are impossible to reproduce.
- **Platform-specific limits are violated silently** — A 32-bit WebAssembly target has a 4GB address space limit (in practice, ~2GB usable). Desktop might have 16GB+. Without runtime budget enforcement, the engine works fine on the developer's 64GB workstation but crashes on players' machines.

A custom global allocator wrapper that counts allocations, combined with per-subsystem tagging, provides the visibility needed to manage memory proactively rather than reactively.

## Solution

### Custom Global Allocator (Debug Builds Only)

A wrapper around the system allocator that tracks total allocated bytes. This is compiled only in debug builds to avoid any overhead in release:

```rust
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct TrackingAllocator {
    inner: System,
}

pub static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
pub static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static DEALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static PEAK_ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            let new_total = ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed)
                + layout.size();
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);

            // Update peak using a CAS loop
            let mut peak = PEAK_ALLOCATED_BYTES.load(Ordering::Relaxed);
            while new_total > peak {
                match PEAK_ALLOCATED_BYTES.compare_exchange_weak(
                    peak,
                    new_total,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(current) => peak = current,
                }
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOCATED_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        DEALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { self.inner.dealloc(ptr, layout) };
    }
}

#[cfg(debug_assertions)]
#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator { inner: System };
```

### Per-Subsystem Tracking

Each engine subsystem reports its own memory usage through a `MemoryBudget` resource. Subsystems either track allocations explicitly (for large, well-defined allocations like chunk data and mesh buffers) or estimate usage based on container sizes:

```rust
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct SubsystemMemory {
    pub current_bytes: usize,
    pub peak_bytes: usize,
    pub budget_bytes: usize,
    pub allocation_count: usize,
}

pub struct MemoryBudget {
    pub subsystems: HashMap<&'static str, SubsystemMemory>,
    pub total_budget_bytes: usize,
    pub warning_threshold: f32, // Fraction (e.g., 0.85 for 85%)
}

impl MemoryBudget {
    pub fn report(&mut self, name: &'static str, current_bytes: usize, count: usize) {
        let entry = self.subsystems.entry(name).or_insert(SubsystemMemory {
            current_bytes: 0,
            peak_bytes: 0,
            budget_bytes: 0,
            allocation_count: 0,
        });
        entry.current_bytes = current_bytes;
        entry.allocation_count = count;
        if current_bytes > entry.peak_bytes {
            entry.peak_bytes = current_bytes;
        }
    }

    pub fn set_budget(&mut self, name: &'static str, budget_bytes: usize) {
        let entry = self.subsystems.entry(name).or_insert(SubsystemMemory {
            current_bytes: 0,
            peak_bytes: 0,
            budget_bytes: 0,
            allocation_count: 0,
        });
        entry.budget_bytes = budget_bytes;
    }

    pub fn is_over_budget(&self, name: &str) -> bool {
        self.subsystems.get(name).is_some_and(|s| {
            s.budget_bytes > 0
                && s.current_bytes as f32 > s.budget_bytes as f32 * self.warning_threshold
        })
    }

    pub fn total_current(&self) -> usize {
        self.subsystems.values().map(|s| s.current_bytes).sum()
    }
}
```

### Subsystem Reporters

Each major subsystem reports its memory usage every frame (or every N frames to reduce overhead):

| Subsystem  | What It Tracks                                                      | How It Measures                                    |
|------------|---------------------------------------------------------------------|----------------------------------------------------|
| Chunks     | Voxel palette + block data for all loaded chunks                    | `chunk_count * (palette_size + block_data_size)`   |
| Meshes     | Vertex and index buffers for terrain and entity meshes              | Sum of `buffer.size()` for all active mesh buffers |
| Textures   | Texture atlas pages, material textures, framebuffers                | Sum of `texture.size()` from wgpu texture descriptors |
| Entities   | ECS component storage, entity metadata                              | Estimated from archetype table sizes               |
| Audio      | Loaded audio samples, streaming buffers                             | Sum of sample byte lengths + streaming buffer sizes |
| Physics    | Rapier collision shapes, broadphase structures, island data         | `rapier_world.counters.memory_usage()`             |

```rust
fn report_chunk_memory(
    chunk_manager: Res<ChunkManager>,
    mut budget: ResMut<MemoryBudget>,
) {
    let mut total_bytes = 0usize;
    let mut count = 0usize;

    for chunk in chunk_manager.loaded_chunks() {
        total_bytes += chunk.palette_memory();
        total_bytes += chunk.block_data_memory();
        count += 1;
    }

    budget.report("Chunks", total_bytes, count);
}

fn report_mesh_memory(
    mesh_store: Res<MeshStore>,
    mut budget: ResMut<MemoryBudget>,
) {
    let mut total_bytes = 0usize;
    let mut count = 0usize;

    for mesh in mesh_store.all_meshes() {
        total_bytes += mesh.vertex_buffer_size();
        total_bytes += mesh.index_buffer_size();
        count += 1;
    }

    budget.report("Meshes", total_bytes, count);
}
```

### Default Budgets

Sensible default budgets are configured at startup and can be overridden via the config file:

```rust
fn configure_memory_budgets(mut budget: ResMut<MemoryBudget>) {
    budget.total_budget_bytes = 4 * 1024 * 1024 * 1024; // 4 GB total

    budget.set_budget("Chunks",    1024 * 1024 * 1024);  // 1 GB
    budget.set_budget("Meshes",     512 * 1024 * 1024);  // 512 MB
    budget.set_budget("Textures",   512 * 1024 * 1024);  // 512 MB
    budget.set_budget("Entities",   256 * 1024 * 1024);  // 256 MB
    budget.set_budget("Audio",      128 * 1024 * 1024);  // 128 MB
    budget.set_budget("Physics",    256 * 1024 * 1024);  // 256 MB
}
```

### Egui Display

The memory tracking is displayed as a table in an egui window:

```rust
fn draw_memory_window(
    mut egui_ctx: ResMut<EguiContext>,
    budget: Res<MemoryBudget>,
) {
    egui::Window::new("Memory Usage")
        .default_size([450.0, 300.0])
        .show(egui_ctx.get_mut(), |ui| {
            // Global stats from the tracking allocator
            #[cfg(debug_assertions)]
            {
                let allocated = ALLOCATED_BYTES.load(Ordering::Relaxed);
                let peak = PEAK_ALLOCATED_BYTES.load(Ordering::Relaxed);
                let alloc_count = ALLOCATION_COUNT.load(Ordering::Relaxed);
                let dealloc_count = DEALLOCATION_COUNT.load(Ordering::Relaxed);

                ui.label(format!(
                    "Global: {}  Peak: {}  Live allocs: {}",
                    format_memory(allocated),
                    format_memory(peak),
                    alloc_count.saturating_sub(dealloc_count),
                ));
                ui.separator();
            }

            // Per-subsystem table
            egui::Grid::new("memory_grid")
                .num_columns(5)
                .spacing([20.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    // Header
                    ui.strong("Subsystem");
                    ui.strong("Current");
                    ui.strong("Peak");
                    ui.strong("Budget");
                    ui.strong("Usage");
                    ui.end_row();

                    let mut sorted: Vec<_> = budget.subsystems.iter().collect();
                    sorted.sort_by_key(|(name, _)| *name);

                    for (name, mem) in sorted {
                        let usage_frac = if mem.budget_bytes > 0 {
                            mem.current_bytes as f32 / mem.budget_bytes as f32
                        } else {
                            0.0
                        };
                        let is_warning = budget.is_over_budget(name);

                        let label_color = if is_warning {
                            egui::Color32::from_rgb(220, 50, 50)
                        } else {
                            egui::Color32::from_rgb(200, 200, 200)
                        };

                        ui.colored_label(label_color, *name);
                        ui.label(format_memory(mem.current_bytes));
                        ui.label(format_memory(mem.peak_bytes));
                        ui.label(if mem.budget_bytes > 0 {
                            format_memory(mem.budget_bytes)
                        } else {
                            "N/A".to_string()
                        });

                        // Progress bar for usage
                        let bar = egui::ProgressBar::new(usage_frac.min(1.0))
                            .text(format!("{:.1}%", usage_frac * 100.0));
                        ui.add(bar);
                        ui.end_row();
                    }
                });

            // Total
            ui.separator();
            let total = budget.total_current();
            ui.label(format!(
                "Tracked Total: {}  /  Budget: {}",
                format_memory(total),
                format_memory(budget.total_budget_bytes),
            ));
        });
}

fn format_memory(bytes: usize) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}
```

### Warning System

When a subsystem approaches its budget (crossing the `warning_threshold`, default 85%), the engine:

1. Colors the subsystem row red in the memory window.
2. Emits a `tracing::warn!` event: `"Subsystem 'Chunks' at 87% of memory budget (890 MB / 1024 MB)"`.
3. Optionally triggers a callback that subsystems can register to handle pressure (e.g., the chunk manager unloads distant chunks, the texture system downsizes mip levels).

### Overhead Considerations

The custom global allocator adds two atomic operations per allocation/deallocation (fetch_add/fetch_sub on `ALLOCATED_BYTES` and a fetch_add on the counter). On modern hardware, this is approximately 5-15 nanoseconds per operation. For a typical game frame with ~10,000 allocations, this adds ~50-150 microseconds of overhead per frame — well within acceptable limits for a debug build. The per-subsystem reporters run once per frame and perform simple arithmetic on already-known sizes (no heap walking or introspection).

The `#[cfg(debug_assertions)]` guard ensures the global allocator wrapper is compiled out entirely in release builds, leaving zero overhead.

## Outcome

An egui window displays a table of engine memory usage broken down by subsystem (Chunks, Meshes, Textures, Entities, Audio, Physics). Each row shows current bytes, peak bytes, budget, and a usage percentage bar. Subsystems approaching their budget are highlighted in red and emit warning log events. A custom global allocator (debug builds only) tracks total allocated bytes, peak allocation, and live allocation count. The implementation lives in `crates/nebula-debug/src/memory_tracking.rs` (the display and `MemoryBudget` resource) and `crates/nebula-debug/src/tracking_allocator.rs` (the global allocator wrapper).

## Demo Integration

**Demo crate:** `nebula-demo`

A panel shows memory usage per subsystem: chunk data, mesh buffers, GPU textures, physics colliders, ECS archetypes. Each category has a progress bar toward its budget.

## Crates & Dependencies

- **`egui = "0.31"`** — Memory table rendering with `egui::Grid`, progress bars for usage visualization, colored labels for warnings, and the window container.
- **`tracing = "0.1"`** — Warning events when subsystems approach their memory budget, and informational logging of peak memory usage at engine shutdown.
- **`tracing-subscriber = "0.3"`** — Ensures memory budget warnings are captured by the configured log output (console and file).

No additional external crates are required. The custom allocator uses only `std::alloc` and `std::sync::atomic` from the standard library. Per-subsystem tracking uses engine-internal interfaces (no heap introspection crates).

## Unit Tests

- **`test_allocator_tracks_total_bytes`** — Reset the global counters. Allocate a `Vec<u8>` of 1024 bytes. Read `ALLOCATED_BYTES` and assert it increased by at least 1024 (may be more due to Vec's capacity rounding). Drop the Vec and assert `ALLOCATED_BYTES` decreased by the same amount. (This test only runs in debug builds with the tracking allocator active.)

- **`test_per_subsystem_tracking_accurate`** — Create a `MemoryBudget`. Call `report("Chunks", 500_000_000, 200)`. Assert `subsystems["Chunks"].current_bytes == 500_000_000` and `allocation_count == 200`. Call `report("Chunks", 450_000_000, 180)` and assert the current value updated to `450_000_000` while peak remains `500_000_000`.

- **`test_peak_tracks_maximum`** — Create a `MemoryBudget`. Report "Meshes" with 100MB, then 200MB, then 150MB. Assert `peak_bytes` is 200MB. Report 300MB and assert peak updates to 300MB. Report 250MB and assert peak remains 300MB.

- **`test_warning_triggers_near_budget`** — Create a `MemoryBudget` with `warning_threshold = 0.85`. Set budget for "Textures" to 512MB. Report "Textures" at 400MB (78%). Assert `is_over_budget("Textures")` is `false`. Report at 440MB (86%). Assert `is_over_budget("Textures")` is `true`. Report at 435MB (84.9%). Assert `is_over_budget("Textures")` is `false` (just below threshold).

- **`test_display_shows_correct_values`** — Create a `MemoryBudget` with known values for two subsystems. Assert `total_current()` equals the sum of both subsystems' current bytes. Verify `format_memory(1_073_741_824)` returns `"1.00 GB"`, `format_memory(5_242_880)` returns `"5.0 MB"`, `format_memory(2048)` returns `"2.0 KB"`, and `format_memory(500)` returns `"500 B"`.

- **`test_overhead_is_minimal`** — In a tight loop, perform 10,000 allocations and deallocations of 64-byte blocks. Measure the wall-clock time with and without the tracking allocator (if possible via feature flag). Assert the overhead per allocation is less than 100 nanoseconds on average. (This is a performance regression test, not a correctness test, and may be marked `#[ignore]` for CI.)

- **`test_concurrent_allocation_tracking`** — Spawn 4 threads, each performing 1000 allocations of 1KB. Join all threads. Assert `ALLOCATED_BYTES` reflects the sum of all live allocations (4000 * 1024 bytes, approximately). Assert `ALLOCATION_COUNT` is at least 4000. Drop all allocations and assert `ALLOCATED_BYTES` returns to approximately its starting value.

- **`test_zero_budget_no_warning`** — Create a `MemoryBudget`. Report "Audio" with 100MB but do not set a budget (budget remains 0). Assert `is_over_budget("Audio")` returns `false` regardless of usage, because a zero budget means "unbudgeted."
