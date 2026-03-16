# Changelog

## 2026-03-05 — Baseline implementation

- BTreeMap price levels + VecDeque FIFO + FxHashMap cancel lookup
- Zero-allocation hot path (caller-owned fill buffer)
- Integer tick prices, no floating point
- Benchmark: `results/mac/20260305T213543.txt`

Key numbers (Apple Silicon, release):

| Operation | p50 | p99 |
|---|---|---|
| Passive insert | 42 ns | 84 ns |
| Aggressive fill (1 lot) | 42 ns | 125 ns |
| Cancel (depth=10K) | 42 ns | 125 ns |
| Cancel hot level (10K/level) | 4,251 ns | 9,463 ns |
| Mixed workload (depth=100K) | 42 ns | 625 ns |

Known bottleneck: `VecDeque::retain` in cancel is O(K) per price level.

## 2026-03-06 — O(1) cancel: arena + intrusive linked list

- Replaced `VecDeque<RestingOrder>` with slab arena allocator + doubly-linked list
- Cancel: O(n) `VecDeque::retain` → O(1) pointer unlink + arena slot recycle
- `locations` HashMap now stores arena slot index instead of `(Side, Price)`
- Node stores `prev`/`next` indices for O(1) insert/remove at any position

Key improvements vs 2026-03-05 (Linux):

| Operation | Before p50 | After p50 | Speedup |
|---|---|---|---|
| Cancel (depth=100K) | 1,420 ns | 41 ns | **34x** |
| Cancel hot level (10K/level) | 24,687 ns | 0 ns | **>1000x** |
| Mixed workload (depth=100K) | 110 ns | 41 ns | **2.7x** |
| Total benchmark time | 1.39s | 0.40s | **3.5x** |

## 2026-03-10 — mimalloc + benchmark improvements

- Switched global allocator to `mimalloc` for lower allocation overhead and reduced tail latency
- Cached `best_bid` / `best_ask` — O(log n) BTreeMap traversal → O(1) field read; benefits multi-level sweep most
- Reduced Node size: moved `(side, price)` into `locations` map; Node ~40→24 bytes, improves cache line utilization when traversing levels. (the benchmark result isn't showing the full benefit of this change, might be due to cpu prefetching, need to be investigated later)
- Added Summary section to benchmark report (total ops, throughput, aggregate latency)
- Added git commit hash to benchmark report header for version traceability
- Fixed clippy warnings (`map_or` → `is_none_or`, `% 2 == 0` → `is_multiple_of(2)`)
- Added GitHub Actions CI (fmt + clippy + test + benchmark smoke)

## 2026-03-11 — Build hardening + deployment tuning guide

- Enabled `panic = "abort"` and `strip = true` in release profile
- Added `Makefile` with `bench`, `bench-pin <cpu>`, `test`, `clippy`, `fmt` targets
- Added `docs/tuning.md` — deployment-level tuning guide (CPU pinning, isolcpus, frequency scaling, mlockall, NUMA, THP, etc.)
- Fixed unused variable warnings in benchmark scenarios (`fills`, `first_id`)
- Removed `target-cpu=native` — has negligible effect on this workload because the hot path is memory-bound (pointer chasing in HashMap, BTreeMap, arena linked list), not compute-bound; there is almost nothing for SIMD to vectorize
- Evaluated `lto`, `codegen-units = 1`, `opt-level = 3` — kept for correctness but measured no observable latency improvement; the crate is small (single crate, all hot functions already visible to the optimizer) and the bottleneck is memory access patterns, not instruction throughput

## 2026-03-16 — Benchmark methodology cleanup

- Removed setup work from timed sections in `multi_level_sweep` and `drain_single_level`
- Benchmarks now time only the target matching operation, not per-iteration `OrderBook` construction or scenario seeding
- This makes p99/p99.9/max more representative of steady-state hot-path latency instead of scenario setup cost
