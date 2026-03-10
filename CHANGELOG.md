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