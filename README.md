# matching-engine

A simple, high-performance order book and matching engine in Rust.

## Features

- **Price-time priority (FIFO)** continuous matching
- Limit orders and Market orders
- O(log N) insert/match via `BTreeMap` price levels
- O(1) cancel lookup via `FxHashMap`
- Zero-allocation hot path (caller-owned fill buffer)
- Integer tick prices — no floating point

## Project Structure

```
src/
  lib.rs              Library exports
  types.rs            Order, Fill, Side, OrderType
  orderbook.rs        Order book + matching engine
  main.rs             Benchmark runner
  bench/
    harness.rs        Timing infrastructure + Reporter
    scenarios.rs      7 benchmark scenarios
benches/
  engine.rs           Criterion microbenchmarks
results/
  mac/                Benchmark results from macOS
  linux/              Benchmark results from Linux
```

## Quick Start

```bash
cargo run --release     # run latency benchmark (saves to results/)
```

## Benchmark Scenarios

| Scenario | What it measures |
|---|---|
| Passive Insert | Limit order that doesn't cross — pure BTreeMap + HashMap insert |
| Aggressive Fill | Take 1 lot from best price — match + partial fill |
| Multi-Level Sweep | Large order crossing N price levels |
| Market Order | Aggressive fill with no price limit |
| Cancel | Remove a resting order by ID |
| Cancel Hot Level | Cancel from a single price with 10–10K queued orders |
| Mixed Workload | Realistic flow: 65% cancel, 25% insert, 10% fill |

Each scenario reports latency percentiles: **p50, p99, p99.9, p99.99, min, max**.