# p99.99 Root Cause Hunt

## Context

- `passive-insert depth=100K`: p50=41ns, p99=42ns, p99.9=875ns, p99.99=16,047ns
- System noise already eliminated: `context-switches=1`, `cpu-migrations=0`,
  `irq_handler_entry=0`, `softirq_entry=1`, `page-faults=91` (all minor)
- `ITERS=200,000` means p99.99 is literally the worst ~20 samples out of 200K.
  This is statistically fragile.
- The timing method wraps every single iteration with `Instant::now()` /
  `elapsed()`, so any stall during any iteration is 100% attributed to that
  sample.

## Core Question

The p99.9 (875ns) to p99.99 (16us) jump is ~18x. Is this:

- (A) The measurement floor -- `Instant::now()` itself occasionally takes
  microseconds
- (B) A real but rare code path -- hash resize, BTree node allocation, cache
  miss storm
- (C) Residual CPU microarchitecture noise -- frequency transition, power state
  exit, TLB shootdown

## Step 1: Measure the noise floor (the single most important experiment)

Add one new scenario: `timer-only`. Same `timed_loop`, but the body does
**nothing** (or a trivial `black_box` computation to prevent elision).

```rust
pub fn timer_only() -> Histogram<u64> {
    let mut x = 0u64;
    timed_loop(WARMUP, ITERS, || {
        std::hint::black_box(&mut x);
    })
}
```

Run it pinned:

```bash
taskset -c 1 ./target/release/matching-engine bench --scenario timer-only
```

**What to look at:**

- If `timer-only` p99.99 is already in the 1-10us range, then `Instant::now()`
  variance is the dominant contributor. Your engine's real tail is much better
  than what you're measuring.
- If `timer-only` p99.99 is consistently under 100ns, the 16us tail is real
  engine behavior.

This one experiment saves you from chasing ghosts in the wrong layer.

## Step 2: Outlier capture (only if Step 1 shows noise floor is low)

Don't instrument every iteration. Instead, add a **conditional slow-path
logger** that only fires when a single iteration exceeds a threshold (e.g.
500ns). When it fires, print:

- iteration number (`i`)
- elapsed time
- `arena.slots.len()` vs `arena.slots.capacity()`
- `locations.len()` vs `locations.capacity()`
- `bids.len()` + `asks.len()`
- the `price` and `side` of the order that was slow

This tells you immediately whether outliers cluster around:

- **capacity boundaries** (hash resize / vec growth) -- `len == capacity` right
  before the spike
- **new price levels** (BTree node alloc) -- `bids.len()` or `asks.len()`
  increases on that iteration
- **nothing structural** (cache/TLB) -- state looks normal, outliers are
  randomly distributed

Implementation: modify `passive_insert` to use a manual loop instead of
`timed_loop`, adding a check after `elapsed()`. The logging path is cold and
won't affect the hot path.

## Step 3: Subsegment timing (only if Step 2 points to a specific operation)

Once you know outliers correlate with e.g. "hash insert" or "BTree entry", add
a second timer **only inside the outlier path** to split `place()` into:

1. `arena.alloc(...)` -- should be O(1) Vec::push, but verify
2. `bids/asks.entry(price).or_insert_with(...)` -- BTree walk + possible node
   alloc
3. tail splice (`arena.get_mut` on old_tail) -- pointer chase to potentially
   cold memory
4. `locations.insert(...)` -- hash probe + possible resize

This is the most invasive step so do it last, and only on the suspect path.

## What NOT to do next

- Don't keep tuning OS parameters. You've already gotten IRQ/scheduler/softirq
  to near-zero. Diminishing returns.
- Don't run broad `perf record` hoping to see a 16us spike in a flamegraph.
  Sampling profilers average over millions of samples; they can't catch 20
  outliers in 200K iterations.
- Don't increase `ITERS` to 10M hoping p99.99 stabilizes. First establish the
  noise floor; if it's high, more samples won't help.
- Don't switch to `rdtscp` yet. It's a valid optimization but won't explain the
  root cause if the tail is real.

## Decision Tree

```
timer-only p99.99 > 1us?
├── YES → measurement artifact. Consider:
│   ├── batch timing (N ops per Instant pair)
│   ├── rdtscp-based timer
│   └── accept that per-iteration wall-clock p99.99
│       has an inherent floor on this hardware
└── NO → real engine tail. Run outlier capture:
    ├── outliers at capacity boundary → hash/vec resize
    ├── outliers when bids/asks.len changes → BTree alloc
    ├── outliers random, old_tail far from new idx → cache miss
    └── outliers random, no pattern → deeper PMU investigation
```
