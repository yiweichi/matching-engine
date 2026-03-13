mod bench;

use std::time::Instant;

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use bench::harness::*;
use bench::scenarios;

fn try_mlockall() {
    #[cfg(unix)]
    unsafe {
        if libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) != 0 {
            let err = std::io::Error::last_os_error();
            let hint = match err.raw_os_error() {
                Some(libc::EPERM) => {
                    "hint: run with sufficient privileges or raise RLIMIT_MEMLOCK"
                }
                Some(libc::ENOMEM) => {
                    "hint: increase `ulimit -l` / memlock limit, or disable mlockall on small/shared machines"
                }
                _ => "hint: see docs/tuning.md for mlockall setup",
            };
            eprintln!("warning: mlockall failed: {err}; {hint}");
        }
    }
}

fn main() {
    try_mlockall();

    let mut r = Reporter::new();

    r.header("=== Matching Engine Latency Benchmark ===");
    r.git_version();
    r.header(&format!(
        "    warmup={WARMUP}  iters={ITERS}  sweep_iters={SWEEP_ITERS}"
    ));

    let t0 = Instant::now();

    r.section("Passive Insert");
    for &d in &[0u64, 100, 10_000, 100_000] {
        r.row(
            &format!("depth={}", fmt_depth(d)),
            &scenarios::passive_insert(d),
        );
    }

    r.section("Aggressive Fill (1 lot)");
    for &d in &[100u64, 10_000, 100_000] {
        r.row(
            &format!("depth={}", fmt_depth(d)),
            &scenarios::aggressive_fill(d),
        );
    }

    r.section("Multi-Level Sweep");
    for &l in &[1u64, 5, 10, 50] {
        r.row(&format!("{} levels", l), &scenarios::multi_level_sweep(l));
    }

    r.section("Market Order (1 lot)");
    for &d in &[100u64, 10_000, 100_000] {
        r.row(
            &format!("depth={}", fmt_depth(d)),
            &scenarios::market_order(d),
        );
    }

    r.section("Cancel");
    for &d in &[100u64, 10_000, 100_000] {
        r.row(&format!("depth={}", fmt_depth(d)), &scenarios::cancel(d));
    }

    r.section("Cancel Hot Level (single price)");
    for &n in &[10u64, 100, 1_000, 10_000] {
        r.row(
            &format!("{} orders/level", n),
            &scenarios::cancel_hot_level(n),
        );
    }

    r.section("Drain Single Level");
    for &n in &[10u64, 50, 100, 500, 1_000] {
        r.row(&format!("{} orders", n), &scenarios::drain_single_level(n));
    }

    r.section("Mixed Workload (65% cancel, 25% insert, 10% fill)");
    for &d in &[100u64, 10_000, 100_000] {
        r.row(
            &format!("depth={}", fmt_depth(d)),
            &scenarios::mixed_workload(d),
        );
    }

    r.summary(t0.elapsed());
    r.save();
}
