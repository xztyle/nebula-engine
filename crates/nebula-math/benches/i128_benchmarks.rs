use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nebula_math::*;

fn bench_i128_add(c: &mut Criterion) {
    let a: i128 = black_box(123_456_789_012_345_678);
    let b: i128 = black_box(987_654_321_098_765_432);
    c.bench_function("i128_add", |bencher| bencher.iter(|| black_box(a + b)));
}

fn bench_i128_sub(c: &mut Criterion) {
    let a: i128 = black_box(987_654_321_098_765_432);
    let b: i128 = black_box(123_456_789_012_345_678);
    c.bench_function("i128_sub", |bencher| bencher.iter(|| black_box(a - b)));
}

fn bench_i128_mul(c: &mut Criterion) {
    let a: i128 = black_box(1_000_000_000);
    let b: i128 = black_box(2_000_000_000);
    c.bench_function("i128_mul", |bencher| bencher.iter(|| black_box(a * b)));
}

fn bench_i128_div(c: &mut Criterion) {
    let a: i128 = black_box(1_000_000_000_000_000_000);
    let b: i128 = black_box(1_000_000_000);
    c.bench_function("i128_div", |bencher| bencher.iter(|| black_box(a / b)));
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
        .map(|i| WorldPosition::new(origin.x + i * 1000, origin.y + i * 500, origin.z + i * 250))
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

#[cfg(test)]
mod tests {

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
