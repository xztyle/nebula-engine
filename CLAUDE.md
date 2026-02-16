# CLAUDE.md - Nebula Engine

## What Is This

Nebula Engine is an AI-friendly voxel game engine written in Rust. Built to power Spacic, an MMO voxel space game with magic systems, cube-sphere planets, and galaxy-scale coordinates.

## Principles

- **DRY** -- Don't Repeat Yourself. Extract shared logic into functions/modules.
- **KISS** -- Keep It Simple, Stupid. Simplest solution that works correctly.
- **YAGNI** -- You Aren't Gonna Need It. Don't build features until they're needed.
- **Clean Architecture** -- Dependencies flow inward. Core logic has no external dependencies. Outer layers depend on inner layers, never the reverse.

## Code Style

- Rust edition 2024, toolchain 1.93+
- `cargo fmt` before every commit
- `cargo clippy --workspace --all-targets -- -D warnings` must pass (always use `--all-targets` to match CI)
- **After all checks pass, run the demo and validate via the AI Debug API:**
  - `cargo run -p nebula-demo &` (background the demo)
  - `curl http://localhost:9999/health` (verify engine is alive)
  - `curl http://localhost:9999/metrics` (check FPS, frame time, no regressions)
  - `curl http://localhost:9999/screenshot --output /tmp/nebula-screenshot.png` (capture visual state)
  - **Verify the screenshot is not just a black frame** -- check file size > 10KB or inspect pixels
  - Kill the demo process after validation
  - **This is mandatory for every story. No exceptions.**
- No `unwrap()` in library code -- use `Result`/`Option` properly
- `unwrap()` acceptable in tests and demo code only
- Max 500 lines per file. Split if exceeded.
- Prefer `pub(crate)` over `pub` unless the type is part of the public API
- Doc comments on all public items

## Architecture

### Crate Structure

Cargo workspace with crates under `crates/`. Dependencies flow downward:

```
nebula-math (leaf -- no engine deps)
  -> nebula-coords
    -> nebula-voxel, nebula-cubesphere
      -> nebula-mesh, nebula-terrain
        -> nebula-render, nebula-lod
          -> nebula-planet, nebula-space
            -> nebula-app (top-level binary)
```

No circular dependencies. If two crates need shared types, factor them into a lower crate.

### Key Design Decisions

- **i128 world positions** at millimeter precision (18 billion light-year range)
- **Bevy ECS standalone** (not the full Bevy engine) for archetype-based ECS with parallel scheduling
- **wgpu** for cross-platform GPU rendering
- **Rapier** for physics
- **Rhai** for scripting/modding
- **Kira** for audio
- No editor UI -- library-only engine with debug overlays as needed

## AI Debug API

Every debug/test build automatically exposes an HTTP endpoint (default `:9999`) that allows AI agents to:

- `GET /screenshot` -- capture current frame as PNG
- `GET /metrics` -- frame time, FPS, memory, draw calls, chunk count
- `POST /input` -- inject keyboard/mouse/gamepad events
- `GET /state` -- query ECS entities and components
- `POST /command` -- execute engine commands (teleport, spawn, set time, etc.)

This is the foundation for autonomous AI-driven development and testing. The debug API is part of the engine core, not an afterthought.

## Development Workflow

### Plans

The `plans/` directory contains 37 phases of implementation, each with numbered user stories. Each story is a self-contained unit of work with:

- Problem statement
- Solution with code snippets
- Expected outcome
- Demo integration
- Dependencies
- Unit tests

### The Demo Rule

**Every completed story must update `nebula-demo`.** The demo is the living proof the engine works. At no point should the demo regress or fail to run.

### Performance Validation Protocol

**Every completed story must pass performance validation.** This is non-negotiable.

1. **Before starting a story**: Run the demo, record baseline frame time
2. **After completing a story**: Run the demo, record new frame time
3. **Compare**: If frame time increased >10% without justification, the story is not done
4. **Budget enforcement**: CPU-heavy operations (meshing, terrain gen, LOD) have millisecond budgets defined in their story files
5. **Metrics source**: Use the AI debug API `/metrics` endpoint for automated measurement
6. **Frame time tracking exists from story 01_setup/06** and is always available

Performance is not a phase -- it's a constraint on every phase.

### Commit Convention

- One commit per completed story (squash if needed)
- Format: `feat(phase/story): short description`
- Example: `feat(01_setup/04): spawn window with winit`
- **Commit AND push after each story is validated** -- no local-only commits
- `git add -A && git commit -m "..." && git push` every time

### Testing

- `cargo test --workspace` must pass at all times
- Each story defines its own unit tests
- Integration tests live in `tests/` at workspace root
- Performance regression tests run in CI

## File Limits

- Max 500 lines per `.rs` file
- If a module grows beyond 500 lines, split into submodules
- Prefer many small, focused files over few large ones

## What NOT to Do

- Don't use `unsafe` without a comment explaining why and proving soundness
- Don't add dependencies without checking if the functionality can be built simply
- Don't optimize prematurely -- but DO measure always
- Don't skip tests to save time
- Don't merge code that breaks the demo
