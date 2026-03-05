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
