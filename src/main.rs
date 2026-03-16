mod arg;
mod bench;

use std::time::Instant;

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use arg::{BenchArgs, Cli, Command, ProfileArgs, Scenario};
use bench::harness::*;
use bench::scenarios;
use clap::{Parser, ValueEnum};

fn values(custom: Option<u64>, defaults: &[u64]) -> Vec<u64> {
    custom.map_or_else(|| defaults.to_vec(), |v| vec![v])
}

fn run_profile(args: &ProfileArgs) {
    let t0 = Instant::now();
    match args.scenario {
        Scenario::PassiveInsert => {
            for d in values(args.depth, &[0u64, 100, 10_000, 100_000]) {
                scenarios::profile_passive_insert(d);
            }
        }
        Scenario::AggressiveFill => {
            for d in values(args.depth, &[100u64, 10_000, 100_000]) {
                scenarios::profile_aggressive_fill(d);
            }
        }
        Scenario::MultiLevelSweep => {
            for l in values(args.levels, &[1u64, 5, 10, 50]) {
                scenarios::profile_multi_level_sweep(l);
            }
        }
        Scenario::MarketOrder => {
            for d in values(args.depth, &[100u64, 10_000, 100_000]) {
                scenarios::profile_market_order(d);
            }
        }
        Scenario::Cancel => {
            for d in values(args.depth, &[100u64, 10_000, 100_000]) {
                scenarios::profile_cancel(d);
            }
        }
        Scenario::CancelHotLevel => {
            for n in values(args.orders, &[10u64, 100, 1_000, 10_000]) {
                scenarios::profile_cancel_hot_level(n);
            }
        }
        Scenario::DrainSingleLevel => {
            for n in values(args.orders, &[10u64, 50, 100, 500, 1_000]) {
                scenarios::profile_drain_single_level(n);
            }
        }
        Scenario::MixedWorkload => {
            for d in values(args.depth, &[100u64, 10_000, 100_000]) {
                scenarios::profile_mixed_workload(d);
            }
        }
    }
    eprintln!(
        "profile complete: scenario={} elapsed={:.2?}",
        args.scenario.to_possible_value().unwrap().get_name(),
        t0.elapsed()
    );
}

fn run_bench(r: &mut Reporter, args: &BenchArgs) {
    match args.scenario {
        None => {
            r.section("Passive Insert");
            for d in values(None, &[0u64, 100, 10_000, 100_000]) {
                r.row(
                    &format!("depth={}", fmt_depth(d)),
                    &scenarios::passive_insert(d),
                );
            }

            r.section("Aggressive Fill (1 lot)");
            for d in values(None, &[100u64, 10_000, 100_000]) {
                r.row(
                    &format!("depth={}", fmt_depth(d)),
                    &scenarios::aggressive_fill(d),
                );
            }

            r.section("Multi-Level Sweep");
            for l in values(None, &[1u64, 5, 10, 50]) {
                r.row(&format!("{} levels", l), &scenarios::multi_level_sweep(l));
            }

            r.section("Market Order (1 lot)");
            for d in values(None, &[100u64, 10_000, 100_000]) {
                r.row(
                    &format!("depth={}", fmt_depth(d)),
                    &scenarios::market_order(d),
                );
            }

            r.section("Cancel");
            for d in values(None, &[100u64, 10_000, 100_000]) {
                r.row(&format!("depth={}", fmt_depth(d)), &scenarios::cancel(d));
            }

            r.section("Cancel Hot Level (single price)");
            for n in values(None, &[10u64, 100, 1_000, 10_000]) {
                r.row(
                    &format!("{} orders/level", n),
                    &scenarios::cancel_hot_level(n),
                );
            }

            r.section("Drain Single Level");
            for n in values(None, &[10u64, 50, 100, 500, 1_000]) {
                r.row(&format!("{} orders", n), &scenarios::drain_single_level(n));
            }

            r.section("Mixed Workload (65% cancel, 25% insert, 10% fill)");
            for d in values(None, &[100u64, 10_000, 100_000]) {
                r.row(
                    &format!("depth={}", fmt_depth(d)),
                    &scenarios::mixed_workload(d),
                );
            }
        }
        Some(Scenario::PassiveInsert) => {
            r.section("Passive Insert");
            for d in values(args.depth, &[0u64, 100, 10_000, 100_000]) {
                r.row(
                    &format!("depth={}", fmt_depth(d)),
                    &scenarios::passive_insert(d),
                );
            }
        }
        Some(Scenario::AggressiveFill) => {
            r.section("Aggressive Fill (1 lot)");
            for d in values(args.depth, &[100u64, 10_000, 100_000]) {
                r.row(
                    &format!("depth={}", fmt_depth(d)),
                    &scenarios::aggressive_fill(d),
                );
            }
        }
        Some(Scenario::MultiLevelSweep) => {
            r.section("Multi-Level Sweep");
            for l in values(args.levels, &[1u64, 5, 10, 50]) {
                r.row(&format!("{} levels", l), &scenarios::multi_level_sweep(l));
            }
        }
        Some(Scenario::MarketOrder) => {
            r.section("Market Order (1 lot)");
            for d in values(args.depth, &[100u64, 10_000, 100_000]) {
                r.row(
                    &format!("depth={}", fmt_depth(d)),
                    &scenarios::market_order(d),
                );
            }
        }
        Some(Scenario::Cancel) => {
            r.section("Cancel");
            for d in values(args.depth, &[100u64, 10_000, 100_000]) {
                r.row(&format!("depth={}", fmt_depth(d)), &scenarios::cancel(d));
            }
        }
        Some(Scenario::CancelHotLevel) => {
            r.section("Cancel Hot Level (single price)");
            for n in values(args.orders, &[10u64, 100, 1_000, 10_000]) {
                r.row(
                    &format!("{} orders/level", n),
                    &scenarios::cancel_hot_level(n),
                );
            }
        }
        Some(Scenario::DrainSingleLevel) => {
            r.section("Drain Single Level");
            for n in values(args.orders, &[10u64, 50, 100, 500, 1_000]) {
                r.row(&format!("{} orders", n), &scenarios::drain_single_level(n));
            }
        }
        Some(Scenario::MixedWorkload) => {
            r.section("Mixed Workload (65% cancel, 25% insert, 10% fill)");
            for d in values(args.depth, &[100u64, 10_000, 100_000]) {
                r.row(
                    &format!("depth={}", fmt_depth(d)),
                    &scenarios::mixed_workload(d),
                );
            }
        }
    }
}

fn try_mlockall() {
    #[cfg(target_os = "linux")]
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
    let cli = Cli::parse();

    try_mlockall();

    match cli.command.unwrap_or(Command::Bench(BenchArgs::default())) {
        Command::Profile(args) => run_profile(&args),
        Command::Bench(args) => {
            let mut r = Reporter::new();

            r.header("=== Matching Engine Latency Benchmark ===");
            r.git_version();
            r.header(&format!(
                "    warmup={WARMUP}  iters={ITERS}  sweep_iters={SWEEP_ITERS}"
            ));

            let t0 = Instant::now();
            run_bench(&mut r, &args);

            r.summary(t0.elapsed());
            r.save();
        }
    }
}
