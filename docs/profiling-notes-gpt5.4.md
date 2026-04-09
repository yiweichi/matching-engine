# Profiling Notes

## Tail Latency Root Cause Plan

### Goal

Identify whether the current `passive-insert` `p99.99` tail is mainly caused by
benchmark methodology or by rare slow paths inside the order book.

### What We Already Know

- OS noise is not the main suspect anymore: `cpu-migrations` and
  `context-switches` are near zero, and IRQ/softirq activity on the pinned core
  is extremely low.
- The timed benchmark in `src/bench/scenarios.rs` measures each iteration with
  `Instant::now()` around a single operation, so timer overhead and rare stalls
  are fully attributed to that sample.
- The passive path in `src/orderbook.rs` is mostly `match_*` early-exit plus
  `place()`, which touches the arena, `BTreeMap`, and `locations` hash map.

### Investigation Order

1. Establish a measurement baseline.
- Add or run a null benchmark that uses the same timing loop in
  `src/bench/scenarios.rs` but with an empty or trivial body.
- Compare its `p50`, `p99`, `p99.9`, and `p99.99` with `passive-insert`.
- If the null benchmark already has microsecond-class tail samples, the
  per-iteration wall-clock measurement is a major contributor.

2. Make the extreme tail statistically less fragile.
- Re-run `passive-insert` with much larger recorded sample counts than the
  current `ITERS` in `src/bench/harness.rs`.
- Focus on stability across repeated runs: if `p99.9` is stable but `p99.99`
  jumps around, treat `p99.99` as sparse-outlier territory rather than a stable
  service level.

3. Instrument only outliers, not every iteration.
- Add threshold-based logging around the passive path so only iterations above
  a chosen cutoff are captured.
- Log the high-level state needed to classify the outlier: iteration number,
  `id`, `price`, `side`, arena length/capacity, `locations` length/capacity,
  and bid/ask level counts.
- This should quickly tell us whether outliers correlate with map growth, rare
  new price levels, or are structurally random.

4. Split the passive path into timed subsegments.
- Inside `src/orderbook.rs`, isolate timing for:
- arena allocation
- `BTreeMap::entry(...).or_insert_with(...)`
- linked-list tail splice
- `locations.insert(...)`
- Record these subsegment timings only when the full operation exceeds the
  outlier threshold.
- This should distinguish cache/locality issues from hash-table or tree-related
  occasional slow paths.

5. Use PMU counters only after a code-path suspect appears.
- Once an outlier cluster points to a specific region, use `perf record/report`
  on that narrowed case rather than broad whole-program counters.
- Prioritize LLC and DTLB-related counters if the evidence points to
  memory-locality stalls.

### Extra Check

- Check the benchmark CPU's `TLB shootdowns`; this is a good indicator of memory
  access pressure.

### Expected Outcome

By the end of this sequence, we should know which of these is true:

- The current wall-clock benchmark method is manufacturing most of the observed
  `p99.99`.
- The extreme tail comes from a specific rare structural event such as hash
  growth or tree work.
- The tail is dominated by memory-locality/cache behavior in the steady-state
  passive path.