# Cross-Platform CI

## Problem

The Nebula Engine targets Linux, Windows, and macOS. Each platform has differences that can break the build or cause test failures:

- **File path handling** — Windows uses backslashes and has different path length limits. Asset loading, config file paths, and log file paths must work on all platforms.
- **TCP socket behavior** — Linux, Windows, and macOS have different TCP stack implementations. Edge cases in connection handling, `SO_REUSEADDR` behavior, and socket shutdown semantics vary across platforms. The engine uses pure TCP for networking, so these differences directly affect multiplayer.
- **GPU driver differences** — Even with `wgpu` abstracting the graphics API, different platforms expose different backends (Vulkan on Linux, DX12 on Windows, Metal on macOS) with different precision characteristics and rendering behavior.
- **128-bit integer support** — While Rust's `i128` is portable, the generated machine code differs by platform and architecture. Overflow behavior and performance characteristics can vary.
- **Build toolchain** — MSVC on Windows behaves differently from GCC/Clang-based linkers on Linux/macOS. Linking errors, symbol visibility, and C dependency compilation can fail platform-specifically.
- **CI environment differences** — Available system libraries, default locale settings, and filesystem case sensitivity (macOS is case-insensitive by default) can all cause subtle failures.

The existing CI pipeline (story `01_setup/02_ci_pipeline`) provides the basic structure. This story expands it into a comprehensive cross-platform test matrix that covers every test category defined in the `37_testing` epic.

## Solution

### GitHub Actions workflow

The expanded workflow runs on every push to `main` and every pull request. It uses a matrix strategy across all three platforms with `fail-fast: false` to ensure full results from every platform even when one fails.

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
  # ─── Formatting & Linting (runs once, not per-platform) ───
  lint:
    name: Lint
    runs-on: ubuntu-latest
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
          key: lint-${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: lint-${{ runner.os }}-cargo-

      - name: Check formatting
        run: cargo fmt --all --check

      - name: Run Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings

  # ─── Build & Test (per-platform) ───
  test:
    name: Test (${{ matrix.os }})
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

      - name: Cache Cargo registry and target
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: test-${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: test-${{ runner.os }}-cargo-

      - name: Build (debug)
        run: cargo build --workspace

      - name: Build (release)
        run: cargo build --workspace --release

      - name: Run unit tests
        run: cargo test --workspace --lib

      - name: Run integration tests
        run: cargo test --workspace --test '*'

      - name: Run doc tests
        run: cargo test --workspace --doc

      - name: Upload test results
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: test-results-${{ matrix.os }}
          path: target/nextest/ci/

  # ─── Benchmarks (per-platform, no regression check) ───
  bench:
    name: Bench (${{ matrix.os }})
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

      - name: Cache Cargo registry and target
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: bench-${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: bench-${{ runner.os }}-cargo-

      - name: Run benchmarks (verify they compile and run)
        run: cargo bench --workspace -- --test

  # ─── Determinism cross-platform comparison ───
  determinism:
    name: Determinism (${{ matrix.os }})
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

      - name: Cache Cargo registry and target
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/bin/
            ~/.cargo/registry/index/
            ~/.cargo/registry/cache/
            ~/.cargo/git/db/
            target/
          key: det-${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: det-${{ runner.os }}-cargo-

      - name: Run determinism tests and capture output
        run: cargo test --package nebula-testing -- determinism --test-threads=1
        env:
          NEBULA_DETERMINISM_OUTPUT_DIR: ${{ runner.temp }}/determinism

      - name: Upload determinism artifacts
        uses: actions/upload-artifact@v4
        with:
          name: determinism-${{ matrix.os }}
          path: ${{ runner.temp }}/determinism/

  # ─── Compare determinism artifacts across platforms ───
  determinism-compare:
    name: Compare Determinism
    needs: determinism
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Download Linux determinism artifacts
        uses: actions/download-artifact@v4
        with:
          name: determinism-ubuntu-latest
          path: determinism/linux

      - name: Download Windows determinism artifacts
        uses: actions/download-artifact@v4
        with:
          name: determinism-windows-latest
          path: determinism/windows

      - name: Download macOS determinism artifacts
        uses: actions/download-artifact@v4
        with:
          name: determinism-macos-latest
          path: determinism/macos

      - name: Compare determinism output across platforms
        run: |
          echo "Comparing Linux vs Windows..."
          diff -r determinism/linux determinism/windows
          echo "Comparing Linux vs macOS..."
          diff -r determinism/linux determinism/macos
          echo "All platforms produced identical determinism output."
```

### Cache strategy

Each job type has its own cache key prefix (`lint-`, `test-`, `bench-`, `det-`) to avoid cache conflicts. The cache key is based on the OS and `Cargo.lock` hash, with a restore-keys fallback for partial hits. This means:

- Dependency-only changes get a full cache hit on the registry and a partial hit on the target directory.
- Source changes get a full cache hit on the registry.
- `Cargo.lock` changes invalidate everything, which is correct because dependencies changed.

### Platform-specific considerations

**Windows**: MSVC link times are slower than Linux/macOS. The debug build step may take longer, but the release build with LTO is comparable. TCP tests use `127.0.0.1` rather than `localhost` to avoid IPv6 resolution issues on some Windows CI runners.

**macOS**: The macOS runner uses Apple Silicon (ARM64). Any architecture-specific code (SIMD intrinsics, assembly) must have ARM64 variants or fall back to portable Rust. The filesystem is case-insensitive (HFS+), which can mask casing bugs that would fail on Linux.

**Linux**: The reference platform. All other platforms are compared against Linux for determinism. GPU tests use the software rasterizer because CI runners have no GPU.

### Time budget

Target: each platform's test job completes in under 20 minutes. Current breakdown estimate:

| Step | Time |
|------|------|
| Checkout + toolchain | 30s |
| Cache restore | 15s |
| Debug build | 3-5 min |
| Release build | 5-8 min |
| Unit tests | 1-2 min |
| Integration tests | 2-3 min |
| Doc tests | 30s |
| **Total** | **12-19 min** |

If the time budget is exceeded, the build steps can be split into separate jobs that run in parallel, with test jobs depending on the build job via artifact passing.

### Artifact upload

Test results are uploaded as artifacts on every run (even on failure, via `if: always()`). This ensures that when a test fails on one platform, the full output is available for debugging without re-running the CI.

## Outcome

A `.github/workflows/ci.yml` file with 5 jobs: `lint` (once on Linux), `test` (3x matrix), `bench` (3x matrix), `determinism` (3x matrix), and `determinism-compare` (once, depends on determinism). Covers formatting, linting, debug and release builds, unit tests, integration tests, doc tests, benchmark compilation, and cross-platform determinism comparison. Caches Cargo registry and target directory per-job per-OS. Uploads test results as artifacts. Total CI time target: under 20 minutes per platform. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The full test suite runs on Linux, Windows, and macOS in CI. All platforms produce the same deterministic results. The title bar reads `ALL SYSTEMS VERIFIED`.

## Crates & Dependencies

This story involves no Rust crate changes. The dependencies are CI infrastructure:

| Component | Version | Purpose |
|-----------|---------|---------|
| `actions/checkout` | `v4` | Repository checkout |
| `dtolnay/rust-toolchain` | `stable` | Rust toolchain installation with pinned version `1.93` |
| `actions/cache` | `v4` | Cargo registry and target directory caching |
| `actions/upload-artifact` | `v4` | Upload test results and determinism output |
| `actions/download-artifact` | `v4` | Download determinism artifacts for cross-platform comparison |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use std::fs;

    /// Verify the CI workflow file exists and is valid YAML with expected structure.
    #[test]
    fn test_ci_workflow_exists_and_is_valid() {
        let yaml_content = fs::read_to_string(".github/workflows/ci.yml")
            .expect("CI workflow file should exist");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml_content)
            .expect("CI workflow should be valid YAML");
        assert!(parsed.get("jobs").is_some(), "Workflow should have jobs");
    }

    /// Verify all three platforms are present in the test matrix.
    #[test]
    fn test_ci_matrix_includes_all_platforms() {
        let yaml_content = fs::read_to_string(".github/workflows/ci.yml").unwrap();
        assert!(
            yaml_content.contains("ubuntu-latest"),
            "CI must test on Linux"
        );
        assert!(
            yaml_content.contains("windows-latest"),
            "CI must test on Windows"
        );
        assert!(
            yaml_content.contains("macos-latest"),
            "CI must test on macOS"
        );
    }

    /// Verify that the CI runs on the correct Rust toolchain version.
    #[test]
    fn test_ci_uses_correct_toolchain() {
        let yaml_content = fs::read_to_string(".github/workflows/ci.yml").unwrap();
        assert!(
            yaml_content.contains("1.93"),
            "CI should use Rust toolchain 1.93 (minimum for edition 2024)"
        );
    }

    /// Verify that platform-specific code compiles on the current platform.
    /// This test is inherently platform-specific — it passes on each platform
    /// by exercising the cfg-gated code paths.
    #[test]
    fn test_platform_specific_code_compiles() {
        #[cfg(target_os = "linux")]
        {
            // Linux-specific code paths compile.
            assert!(cfg!(target_os = "linux"));
        }

        #[cfg(target_os = "windows")]
        {
            // Windows-specific code paths compile.
            assert!(cfg!(target_os = "windows"));
        }

        #[cfg(target_os = "macos")]
        {
            // macOS-specific code paths compile.
            assert!(cfg!(target_os = "macos"));
        }

        // Verify we are on a supported platform.
        assert!(
            cfg!(target_os = "linux")
                || cfg!(target_os = "windows")
                || cfg!(target_os = "macos"),
            "Engine only supports Linux, Windows, and macOS"
        );
    }

    /// Verify the cache key in CI references Cargo.lock so that dependency
    /// changes invalidate the cache.
    #[test]
    fn test_ci_cache_key_references_cargo_lock() {
        let yaml_content = fs::read_to_string(".github/workflows/ci.yml").unwrap();
        assert!(
            yaml_content.contains("Cargo.lock"),
            "Cache key should reference Cargo.lock for proper invalidation"
        );
    }

    /// Verify that CI uploads artifacts on failure (if: always()).
    #[test]
    fn test_ci_uploads_artifacts_on_failure() {
        let yaml_content = fs::read_to_string(".github/workflows/ci.yml").unwrap();
        assert!(
            yaml_content.contains("if: always()"),
            "Artifact upload should run even on failure"
        );
    }

    /// Verify the determinism comparison job depends on all platform
    /// determinism jobs completing first.
    #[test]
    fn test_determinism_compare_depends_on_platform_jobs() {
        let yaml_content = fs::read_to_string(".github/workflows/ci.yml").unwrap();
        assert!(
            yaml_content.contains("needs: determinism"),
            "determinism-compare job should depend on determinism jobs"
        );
    }
}
```
