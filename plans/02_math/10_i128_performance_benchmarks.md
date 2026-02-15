# i128 Performance Benchmarks

## Problem

i128 arithmetic is not natively supported on most hardware — on x86_64, each i128 operation compiles to multiple 64-bit instructions. Before building the entire engine on i128 positions, the team needs empirical data on the cost of i128 operations relative to i64 and f64 alternatives. Benchmarks establish a baseline, identify bottlenecks (especially multiplication and division), and validate that the i128 approach is viable for real-time frame budgets (thousands of position operations per frame at 60+ FPS).

## Solution

Create a benchmark suite using the `criterion` crate in the `nebula_math` crate's `benches/` directory.

### Benchmark structure

```
nebula_math/
  benches/
    i128_benchmarks.rs
```

### Individual benchmarks

Each benchmark uses `criterion::black_box` to prevent the compiler from optimizing away the computation.

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nebula_math::*;

fn bench_i128_add(c: &mut Criterion) {
    let a: i128 = black_box(123_456_789_012_345_678);
    let b: i128 = black_box(987_654_321_098_765_432);
    c.bench_function("i128_add", |bencher| {
        bencher.iter(|| black_box(a + b))
    });
}

fn bench_i128_sub(c: &mut Criterion) {
    let a: i128 = black_box(987_654_321_098_765_432);
    let b: i128 = black_box(123_456_789_012_345_678);
    c.bench_function("i128_sub", |bencher| {
        bencher.iter(|| black_box(a - b))
    });
}

fn bench_i128_mul(c: &mut Criterion) {
    let a: i128 = black_box(1_000_000_000);
    let b: i128 = black_box(2_000_000_000);
    c.bench_function("i128_mul", |bencher| {
        bencher.iter(|| black_box(a * b))
    });
}

fn bench_i128_div(c: &mut Criterion) {
    let a: i128 = black_box(1_000_000_000_000_000_000);
    let b: i128 = black_box(1_000_000_000);
    c.bench_function("i128_div", |bencher| {
        bencher.iter(|| black_box(a / b))
    });
}

fn bench_world_position_subtraction(c: &mut Criterion) {
    let a = black_box(WorldPosition::new(
        1_000_000_000_000,
        2_000_000_000_000,
        3_000_000_000_000,
    ));
    let b = black_box(WorldPosition::new(
        1_000_000_001_000,
        2_000_000_002_000,
        3_000_000_003_000,
    ));
    c.bench_function("world_position_sub", |bencher| {
        bencher.iter(|| black_box(a - b))
    });
}

fn bench_distance_squared(c: &mut Criterion) {
    let a = black_box(WorldPosition::new(0, 0, 0));
    let b = black_box(WorldPosition::new(3000, 4000, 0));
    c.bench_function("distance_squared", |bencher| {
        bencher.iter(|| black_box(distance_squared(a, b)))
    });
}

fn bench_dot_product(c: &mut Criterion) {
    let a = black_box(Vec3I128::new(1_000_000, 2_000_000, 3_000_000));
    let b = black_box(Vec3I128::new(4_000_000, 5_000_000, 6_000_000));
    c.bench_function("dot_product", |bencher| {
        bencher.iter(|| black_box(a.dot(b)))
    });
}

fn bench_cross_product(c: &mut Criterion) {
    let a = black_box(Vec3I128::new(1_000_000, 2_000_000, 3_000_000));
    let b = black_box(Vec3I128::new(4_000_000, 5_000_000, 6_000_000));
    c.bench_function("cross_product", |bencher| {
        bencher.iter(|| black_box(a.cross(b)))
    });
}

fn bench_to_local_batch_1000(c: &mut Criterion) {
    let origin = WorldPosition::new(
        1_000_000_000_000_000,
        2_000_000_000_000_000,
        3_000_000_000_000_000,
    );
    let positions: Vec<WorldPosition> = (0..1000)
        .map(|i| WorldPosition::new(
            origin.x + i * 1000,
            origin.y + i * 500,
            origin.z + i * 250,
        ))
        .collect();
    let positions = black_box(positions);
    let mut out = Vec::with_capacity(1000);

    c.bench_function("to_local_batch_1000", |bencher| {
        bencher.iter(|| {
            to_local_batch(&positions, origin, &mut out);
            black_box(&out);
        })
    });
}

fn bench_i64_add_baseline(c: &mut Criterion) {
    let a: i64 = black_box(123_456_789);
    let b: i64 = black_box(987_654_321);
    c.bench_function("i64_add_baseline", |bencher| {
        bencher.iter(|| black_box(a + b))
    });
}

fn bench_f64_add_baseline(c: &mut Criterion) {
    let a: f64 = black_box(123456.789);
    let b: f64 = black_box(987654.321);
    c.bench_function("f64_add_baseline", |bencher| {
        bencher.iter(|| black_box(a + b))
    });
}

fn bench_f64_sqrt_baseline(c: &mut Criterion) {
    let v: f64 = black_box(25_000_000.0);
    c.bench_function("f64_sqrt_baseline", |bencher| {
        bencher.iter(|| black_box(v.sqrt()))
    });
}

criterion_group!(
    benches,
    bench_i128_add,
    bench_i128_sub,
    bench_i128_mul,
    bench_i128_div,
    bench_world_position_subtraction,
    bench_distance_squared,
    bench_dot_product,
    bench_cross_product,
    bench_to_local_batch_1000,
    bench_i64_add_baseline,
    bench_f64_add_baseline,
    bench_f64_sqrt_baseline,
);
criterion_main!(benches);
```

### Cargo.toml additions

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "i128_benchmarks"
harness = false
```

### Expected performance characteristics

| Operation | Expected relative cost (vs i64) | Notes |
|-----------|--------------------------------|-------|
| i128 add/sub | ~1.5-2x | Two 64-bit adds with carry |
| i128 mul | ~3-5x | Karatsuba-style decomposition |
| i128 div | ~10-20x | Software division via `__divti3` |
| WorldPosition sub | ~2x of 3× i128 sub | Three component subtractions |
| distance_squared | ~5x | Three muls + two adds |
| dot product | ~5x | Same as distance_squared |
| to_local (single) | ~3x | Sub + three i128→f32 casts |
| to_local batch (1000) | ~3-5 us total | ~3-5 ns per position |

These are rough estimates. The benchmarks will provide actual numbers on the target hardware.

### Benchmark analysis criteria

After running benchmarks, evaluate:

1. **Per-frame budget**: At 60 FPS, a frame is ~16.6 ms. If 10,000 positions are converted per frame, the to_local batch should take < 50 us (0.3% of frame).
2. **Division avoidance**: If i128 div is prohibitively slow, the engine should prefer multiplication by reciprocals or shift-based division where possible.
3. **Comparison with f64**: If i128 operations are > 10x slower than f64 equivalents, consider hybrid approaches (i128 for storage, f64 for math) in non-precision-critical paths.

## Outcome

After this story is complete:

- A `criterion` benchmark suite compiles and runs with `cargo bench`
- Baseline timing data is established for all core i128 operations
- Comparison with i64 and f64 baselines quantifies the overhead
- The batch `to_local` benchmark validates real-time viability
- Performance regression can be detected in CI by comparing against baselines
- HTML reports are generated in `target/criterion/` for visual analysis

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; benchmark results are logged at startup: `i128 add: 2.3ns, i128 mul: 4.1ns`, proving the math is fast enough for real-time.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `criterion` | `0.5` | Statistical benchmarking framework with HTML reports |

Rust edition 2024. `criterion` is added as a `dev-dependency` only — it does not affect the runtime binary.

## Unit Tests

Benchmarks are not unit tests, but the following compile-and-run tests ensure the benchmark infrastructure works:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the benchmark functions compile and can be called.
    /// This is not a performance test — just a smoke test.
    #[test]
    fn test_benchmark_operations_compile() {
        let a = WorldPosition::new(1000, 2000, 3000);
        let b = WorldPosition::new(4000, 5000, 6000);

        // These must compile and not panic
        let _delta = a - b;
        let _dist = distance_squared(a, b);
        let v1 = Vec3I128::new(1, 2, 3);
        let v2 = Vec3I128::new(4, 5, 6);
        let _dot = v1.dot(v2);
        let _cross = v1.cross(v2);
        let _local = to_local(b, a);
    }

    #[test]
    fn test_batch_conversion_produces_correct_count() {
        let origin = WorldPosition::new(0, 0, 0);
        let positions: Vec<WorldPosition> = (0..1000)
            .map(|i| WorldPosition::new(i, i * 2, i * 3))
            .collect();
        let mut out = Vec::new();
        to_local_batch(&positions, origin, &mut out);
        assert_eq!(out.len(), 1000);
    }

    #[test]
    fn test_i128_operations_do_not_panic() {
        // Ensure the specific values used in benchmarks don't cause issues
        let a: i128 = 123_456_789_012_345_678;
        let b: i128 = 987_654_321_098_765_432;
        let _ = a + b;
        let _ = b - a;
        let _ = 1_000_000_000i128 * 2_000_000_000i128;
        let _ = 1_000_000_000_000_000_000i128 / 1_000_000_000i128;
    }
}
```
