# CI Pipeline

## Problem

Without automated CI, regressions slip in silently. A Rust game engine with 30+ crates, cross-platform requirements (Linux, Windows, macOS), and complex interdependencies between math, rendering, networking, and ECS systems needs continuous validation. Manual testing is insufficient because:

- Formatting inconsistencies accumulate and create noisy diffs that obscure real changes.
- Clippy warnings, if ignored, evolve into real bugs (especially around unsafe code, integer overflow with 128-bit math, and platform-specific behavior).
- A passing build on Linux does not guarantee a passing build on Windows or macOS due to differences in GPU drivers, file path handling, and TCP socket behavior.
- Test regressions in foundational crates like `nebula-math` or `nebula-coords` can silently break every downstream crate.

Every push and every pull request must be validated automatically across all supported platforms before code is merged.

## Solution

Create a GitHub Actions workflow at `.github/workflows/ci.yml` that runs on every `push` and `pull_request` event targeting the `main` branch.

### Workflow Structure

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  check:
    name: Check (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.93
          components: rustfmt, clippy

      - uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-

      - name: Check formatting
        run: cargo fmt --all --check

      - name: Run Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: Build
        run: cargo build --workspace

      - name: Run tests
        run: cargo test --workspace
```

### Key Design Decisions

1. **`fail-fast: false`** — All three platforms run to completion even if one fails. This ensures we always get the full picture of cross-platform compatibility, rather than seeing only the first failure.

2. **Formatting check runs first** — `cargo fmt --check` is the fastest check and catches the most trivial issues. Running it first provides the fastest feedback for the most common mistake.

3. **Clippy with `-D warnings`** — All warnings are treated as errors. This prevents warning accumulation and enforces a clean codebase. The `--all-targets` flag ensures tests, benchmarks, and examples are also linted.

4. **Cache strategy** — The cache key is based on `Cargo.lock` hash, so dependency updates invalidate the cache. The `restore-keys` fallback allows partial cache hits when only some dependencies changed, which still saves significant time.

5. **Toolchain pinning** — The toolchain is pinned to `1.93` (the minimum version supporting Rust edition 2024) to ensure consistent behavior across developer machines and CI. As the project evolves, this version will be bumped deliberately.

6. **`RUST_BACKTRACE: 1`** — Ensures that any panics during testing produce useful stack traces in the CI logs.

### Future Additions (Not In Scope Yet)

- **GPU tests** — Once rendering is implemented, a separate job will run GPU-dependent tests on a runner with GPU access (or use software rendering via `wgpu`'s `gl` backend).
- **Integration tests** — A dedicated job for longer-running integration tests that are too slow for the main check.
- **Release builds** — A separate workflow for building optimized release binaries and creating GitHub releases.
- **Benchmark regression** — Use `criterion` benchmarks with `github-action-benchmark` to detect performance regressions.

## Outcome

Every PR gets a green or red status check within minutes. The status check is required for merging, so no code reaches `main` without passing formatting, linting, and tests on all three platforms. Contributors see exactly which platform and which step failed, with clear error messages. Cache hits reduce typical CI runs from 10+ minutes to 2-3 minutes for incremental changes.

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; CI ensures `nebula-demo` compiles on every push across all three platforms. The demo's build health is continuously monitored.

## Crates & Dependencies

This story involves no Rust crate changes. The dependencies are purely CI infrastructure:

- **`actions/checkout@v4`** — Checks out the repository at the triggering commit
- **`dtolnay/rust-toolchain@stable`** — Installs the specified Rust toolchain with requested components
- **`actions/cache@v4`** — Caches the Cargo registry and build artifacts between runs

## Unit Tests

- **`test_ci_yaml_valid`** — Parse `.github/workflows/ci.yml` as YAML and verify it contains the expected top-level keys (`name`, `on`, `jobs`). Verify the `check` job exists and has the expected steps. This can be done with a simple Rust test using the `serde_yaml` crate or as a shell script in CI itself.

- **`test_matrix_includes_all_platforms`** — Parse the CI YAML and extract the `matrix.os` array. Assert that it contains exactly `["ubuntu-latest", "windows-latest", "macos-latest"]`. This prevents accidental removal of a platform from the matrix.

- **`test_clippy_denies_warnings`** — Parse the CI YAML and find the Clippy step. Assert that the command string contains `-- -D warnings`. This prevents someone from relaxing the lint strictness.

- **`test_cache_key_uses_cargo_lock`** — Parse the CI YAML and find the cache step. Assert that the cache key contains a reference to `Cargo.lock` hashing. This ensures the cache is invalidated when dependencies change.

- **(CI itself validates by running)** — The ultimate test of the CI pipeline is that it runs successfully. If the workflow file is malformed, GitHub Actions will report an error on the workflow run itself.
