use hdrhistogram::Histogram;
use matching_engine::*;

use super::harness::*;

// ── High-resolution timer (rdtsc on x86_64, Instant fallback) ──

#[cfg(target_arch = "x86_64")]
mod timer {
    use std::sync::OnceLock;

    static CYCLES_PER_NS: OnceLock<f64> = OnceLock::new();

    pub fn calibrate() {
        CYCLES_PER_NS.get_or_init(|| {
            let t0 = std::time::Instant::now();
            let c0 = unsafe { core::arch::x86_64::_rdtsc() };
            std::thread::sleep(std::time::Duration::from_millis(50));
            let c1 = unsafe { core::arch::x86_64::_rdtsc() };
            let elapsed_ns = t0.elapsed().as_nanos() as f64;
            let cpns = (c1 - c0) as f64 / elapsed_ns;
            eprintln!(
                "  TSC calibration: {:.3} cycles/ns ({:.0} MHz)",
                cpns,
                cpns * 1000.0
            );
            cpns
        });
    }

    pub fn cycles_per_ns() -> f64 {
        *CYCLES_PER_NS.get().expect("call timer::calibrate() first")
    }

    #[derive(Clone, Copy)]
    pub struct TimerStart(u64);

    #[inline(always)]
    pub fn start() -> TimerStart {
        unsafe {
            core::arch::x86_64::_mm_lfence();
            TimerStart(core::arch::x86_64::_rdtsc())
        }
    }

    #[inline(always)]
    pub fn elapsed_ns(s: TimerStart) -> u64 {
        let end = unsafe {
            core::arch::x86_64::_mm_lfence();
            core::arch::x86_64::_rdtsc()
        };
        ((end - s.0) as f64 / cycles_per_ns()) as u64
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod timer {
    #[derive(Clone, Copy)]
    pub struct TimerStart(std::time::Instant);

    pub fn calibrate() {}

    #[inline(always)]
    pub fn start() -> TimerStart {
        TimerStart(std::time::Instant::now())
    }

    #[inline(always)]
    pub fn elapsed_ns(s: TimerStart) -> u64 {
        s.0.elapsed().as_nanos() as u64
    }
}

// ── Seeding helpers ─────────────────────────────────────────────

fn seed_one_side(book: &mut OrderBook, side: Side, n: u64, id: &mut u64, fills: &mut Vec<Fill>) {
    for i in 0..n {
        let price = match side {
            Side::Buy => MID - SPREAD - (i % 200),
            Side::Sell => MID + SPREAD + (i % 200),
        };
        fills.clear();
        book.add_order(
            Order {
                id: *id,
                side,
                price,
                qty: 10,
                order_type: OrderType::Limit,
            },
            fills,
        );
        *id += 1;
    }
}

fn seed_both(book: &mut OrderBook, n: u64, id: &mut u64, fills: &mut Vec<Fill>) {
    for i in 0..n {
        let (side, price) = if i % 2 == 0 {
            (Side::Buy, MID - SPREAD - (i % 200))
        } else {
            (Side::Sell, MID + SPREAD + (i % 200))
        };
        fills.clear();
        book.add_order(
            Order {
                id: *id,
                side,
                price,
                qty: 10,
                order_type: OrderType::Limit,
            },
            fills,
        );
        *id += 1;
    }
}

fn fresh_book_both(depth: u64) -> (OrderBook, u64, Vec<Fill>) {
    let mut book = OrderBook::with_capacity(depth as usize);
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    seed_both(&mut book, depth, &mut id, &mut fills);
    (book, id, fills)
}

fn fresh_book_asks(depth: u64) -> (OrderBook, u64, Vec<Fill>) {
    let mut book = OrderBook::with_capacity(depth as usize);
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    seed_one_side(&mut book, Side::Sell, depth, &mut id, &mut fills);
    (book, id, fills)
}

const OUTLIER_THRESHOLD_NS: u64 = 1_000;

struct OutlierTracker {
    outliers: Vec<(u64, u64)>,
    iter: u64,
}

impl OutlierTracker {
    fn new() -> Self {
        Self {
            outliers: Vec::new(),
            iter: 0,
        }
    }

    #[inline(always)]
    fn record(&mut self, hist: &mut Histogram<u64>, ns: u64) {
        hist.record(ns).ok();
        if ns >= OUTLIER_THRESHOLD_NS {
            self.outliers.push((self.iter, ns));
        }
        self.iter += 1;
    }

    fn report(&self) {
        if !self.outliers.is_empty() {
            eprintln!(
                "  outliers (>{OUTLIER_THRESHOLD_NS}ns): {}",
                self.outliers.len()
            );
            for &(iter, ns) in &self.outliers {
                eprintln!("    iter={iter:<8} {ns}ns");
            }
        }
    }
}

/// Busy-spin to let nohz_full detect the single-task condition and stop the
/// scheduler tick. On CONFIG_HZ=1000, 2-3 tick periods (3ms) is enough.
#[inline(never)]
fn wait_for_nohz() {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < 3 {
        std::hint::spin_loop();
    }
}

/// Shared timing loop: runs `warmup + iters` iterations, records only after warmup.
fn timed_loop(warmup: u64, iters: u64, mut body: impl FnMut()) -> Histogram<u64> {
    timer::calibrate();
    wait_for_nohz();
    let mut hist = new_hist();
    let mut ot = OutlierTracker::new();
    for i in 0..(warmup + iters) {
        let t: timer::TimerStart = timer::start();
        body();
        if i >= warmup {
            ot.record(&mut hist, timer::elapsed_ns(t));
        }
    }
    ot.report();
    hist
}

/// Shared profile loop: runs the same workload without timing or histogram recording.
fn profile_loop(warmup: u64, iters: u64, mut body: impl FnMut()) {
    for _ in 0..(warmup + iters) {
        body();
    }
}

const PASSIVE_SIDES: [Side; 2] = [Side::Buy, Side::Sell];
const PASSIVE_PRICES: [Price; 2] = [MID - SPREAD - 200, MID + SPREAD + 200];

#[inline(always)]
fn next_random_side_idx(state: &mut u64) -> usize {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    (x & 1) as usize
}

#[inline(always)]
fn passive_side_order(id: OrderId, side_idx: usize) -> Order {
    Order {
        id,
        side: PASSIVE_SIDES[side_idx],
        price: PASSIVE_PRICES[side_idx],
        qty: 10,
        order_type: OrderType::Limit,
    }
}

// ── Scenarios ───────────────────────────────────────────────────

pub fn passive_insert(depth: u64) -> Histogram<u64> {
    let mut book = OrderBook::with_capacity((depth + WARMUP + ITERS) as usize);
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    seed_both(&mut book, depth, &mut id, &mut fills);

    timed_loop(WARMUP, ITERS, || {
        let (side, price) = if id.is_multiple_of(2) {
            (Side::Buy, MID - SPREAD - 200 - (id % 100))
        } else {
            (Side::Sell, MID + SPREAD + 200 + (id % 100))
        };
        fills.clear();
        book.add_order(
            Order {
                id,
                side,
                price,
                qty: 10,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        id += 1;
    })
}

pub fn add_order_random_side() -> Histogram<u64> {
    let mut book = OrderBook::with_capacity((WARMUP + ITERS) as usize);
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    let mut side_state = 0x9E37_79B9_7F4A_7C15u64;

    timed_loop(WARMUP, ITERS, || {
        let side_idx = next_random_side_idx(&mut side_state);
        fills.clear();
        book.add_order(passive_side_order(id, side_idx), &mut fills);
        id += 1;
    })
}

pub fn add_order_predictable_side() -> Histogram<u64> {
    let mut book = OrderBook::with_capacity((WARMUP + ITERS) as usize);
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;

    timed_loop(WARMUP, ITERS, || {
        let side_idx = (id & 1) as usize;
        fills.clear();
        book.add_order(passive_side_order(id, side_idx), &mut fills);
        id += 1;
    })
}

pub fn aggressive_fill(depth: u64) -> Histogram<u64> {
    timer::calibrate();
    let (mut book, mut id, mut fills) = fresh_book_asks(depth);
    let refill_at = (depth / 4).max(10) as usize;
    let mut hist = new_hist();
    let mut ot = OutlierTracker::new();

    for i in 0..(WARMUP + ITERS) {
        if book.len() < refill_at {
            let fresh = fresh_book_asks(depth);
            book = fresh.0;
            id = fresh.1;
            fills = fresh.2;
        }
        fills.clear();
        let t = timer::start();
        book.add_order(
            Order {
                id,
                side: Side::Buy,
                price: MID + SPREAD + 200,
                qty: 1,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        if i >= WARMUP {
            ot.record(&mut hist, timer::elapsed_ns(t));
        }
        id += 1;
    }
    ot.report();
    hist
}

pub fn multi_level_sweep(num_levels: u64) -> Histogram<u64> {
    timer::calibrate();
    let mut fills = Vec::with_capacity(num_levels as usize);
    let mut id = 1u64;
    let mut hist = new_hist();
    let mut ot = OutlierTracker::new();

    for i in 0..(WARMUP + SWEEP_ITERS) {
        let mut book = OrderBook::with_capacity(num_levels as usize);
        for l in 0..num_levels {
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side: Side::Sell,
                    price: MID + 1 + l,
                    qty: 10,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
            id += 1;
        }
        fills.clear();
        let t = timer::start();
        book.add_order(
            Order {
                id,
                side: Side::Buy,
                price: MID + num_levels,
                qty: num_levels * 10,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        if i >= WARMUP {
            ot.record(&mut hist, timer::elapsed_ns(t));
        }
        id += 1;
    }

    ot.report();
    hist
}

pub fn market_order(depth: u64) -> Histogram<u64> {
    timer::calibrate();
    let (mut book, mut id, mut fills) = fresh_book_asks(depth);
    let refill_at = (depth / 4).max(10) as usize;
    let mut hist = new_hist();
    let mut ot = OutlierTracker::new();

    for i in 0..(WARMUP + ITERS) {
        if book.len() < refill_at {
            let fresh = fresh_book_asks(depth);
            book = fresh.0;
            id = fresh.1;
            fills = fresh.2;
        }
        fills.clear();
        let t = timer::start();
        book.add_order(
            Order {
                id,
                side: Side::Buy,
                price: 0,
                qty: 1,
                order_type: OrderType::Market,
            },
            &mut fills,
        );
        if i >= WARMUP {
            ot.record(&mut hist, timer::elapsed_ns(t));
        }
        id += 1;
    }
    ot.report();
    hist
}

pub fn cancel(depth: u64) -> Histogram<u64> {
    timer::calibrate();
    let (mut book, mut id, _) = fresh_book_both(depth);
    let mut cancel_id = id - depth;
    let mut hist = new_hist();

    for i in 0..(WARMUP + ITERS) {
        if cancel_id >= id {
            let fresh = fresh_book_both(depth);
            book = fresh.0;
            id = fresh.1;
            cancel_id = id - depth;
        }
        let t = timer::start();
        book.cancel(cancel_id);
        if i >= WARMUP {
            hist.record(timer::elapsed_ns(t)).ok();
        }
        cancel_id += 1;
    }
    hist
}

pub fn cancel_hot_level(orders_per_level: u64) -> Histogram<u64> {
    timer::calibrate();
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    let price = MID + SPREAD;
    let iters = ITERS.min(orders_per_level);

    let mut hist = new_hist();
    let mut book = OrderBook::new();

    let mut seed = |book: &mut OrderBook, id: &mut u64| -> u64 {
        let fid = *id;
        for _ in 0..orders_per_level {
            fills.clear();
            book.add_order(
                Order {
                    id: *id,
                    side: Side::Sell,
                    price,
                    qty: 10,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
            *id += 1;
        }
        fid
    };

    let mut cancel_id = seed(&mut book, &mut id);

    for i in 0..(WARMUP + iters) {
        if cancel_id >= id {
            book = OrderBook::new();
            cancel_id = seed(&mut book, &mut id);
        }
        let t = timer::start();
        book.cancel(cancel_id);
        if i >= WARMUP {
            hist.record(timer::elapsed_ns(t)).ok();
        }
        cancel_id += 1;
    }
    hist
}

pub fn drain_single_level(orders: u64) -> Histogram<u64> {
    timer::calibrate();
    let mut fills = Vec::with_capacity(orders as usize);
    let mut id = 1u64;
    let price = MID + SPREAD;
    let mut hist = new_hist();

    for i in 0..(WARMUP + SWEEP_ITERS) {
        let mut book = OrderBook::with_capacity(orders as usize);
        for _ in 0..orders {
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side: Side::Sell,
                    price,
                    qty: 1,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
            id += 1;
        }
        fills.clear();
        let t = timer::start();
        book.add_order(
            Order {
                id,
                side: Side::Buy,
                price,
                qty: orders,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        if i >= WARMUP {
            hist.record(timer::elapsed_ns(t)).ok();
        }
        id += 1;
    }

    hist
}

pub fn mixed_workload(depth: u64) -> Histogram<u64> {
    timer::calibrate();
    let mut fills = Vec::with_capacity(8);
    let mut id = 1u64;
    let mut book = OrderBook::with_capacity(depth as usize);
    seed_both(&mut book, depth, &mut id, &mut fills);

    let ring_cap = depth.max(4096) as usize;
    let mut cancel_ring: Vec<u64> = (1..=depth).collect();
    let mut ring_idx: usize = 0;
    let mut hist = new_hist();

    for i in 0..(WARMUP + ITERS) {
        if book.len() < 50 {
            book = OrderBook::with_capacity(depth as usize);
            id = 1;
            seed_both(&mut book, depth, &mut id, &mut fills);
            cancel_ring = (1..=depth).collect();
            ring_idx = 0;
        }

        let roll = id % 20;
        let t = timer::start();

        if roll < 13 {
            if !cancel_ring.is_empty() {
                let cid = cancel_ring[ring_idx % cancel_ring.len()];
                book.cancel(cid);
                ring_idx += 1;
            }
        } else if roll < 18 {
            let (side, price) = if id.is_multiple_of(2) {
                (Side::Buy, MID - SPREAD - 200 - (id % 100))
            } else {
                (Side::Sell, MID + SPREAD + 200 + (id % 100))
            };
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side,
                    price,
                    qty: 10,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
            if cancel_ring.len() < ring_cap {
                cancel_ring.push(id);
            } else {
                cancel_ring[ring_idx % ring_cap] = id;
            }
        } else {
            let (side, price) = if id.is_multiple_of(2) {
                (Side::Buy, MID + SPREAD + 200)
            } else {
                (Side::Sell, MID - SPREAD - 200)
            };
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side,
                    price,
                    qty: 1,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
        }

        if i >= WARMUP {
            hist.record(timer::elapsed_ns(t)).ok();
        }
        id += 1;
    }
    hist
}

pub fn profile_passive_insert(depth: u64) {
    let mut book = OrderBook::with_capacity((depth + WARMUP + ITERS) as usize);
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    seed_both(&mut book, depth, &mut id, &mut fills);

    profile_loop(WARMUP, ITERS, || {
        let (side, price) = if id.is_multiple_of(2) {
            (Side::Buy, MID - SPREAD - 200 - (id % 100))
        } else {
            (Side::Sell, MID + SPREAD + 200 + (id % 100))
        };
        fills.clear();
        book.add_order(
            Order {
                id,
                side,
                price,
                qty: 10,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        id += 1;
    });
}

pub fn profile_add_order_random_side() {
    let mut book = OrderBook::with_capacity((WARMUP + ITERS) as usize);
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    let mut side_state = 0x9E37_79B9_7F4A_7C15u64;

    profile_loop(WARMUP, ITERS, || {
        let side_idx = next_random_side_idx(&mut side_state);
        fills.clear();
        book.add_order(passive_side_order(id, side_idx), &mut fills);
        id += 1;
    });
}

pub fn profile_add_order_predictable_side() {
    let mut book = OrderBook::with_capacity((WARMUP + ITERS) as usize);
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;

    profile_loop(WARMUP, ITERS, || {
        let side_idx = (id & 1) as usize;
        fills.clear();
        book.add_order(passive_side_order(id, side_idx), &mut fills);
        id += 1;
    });
}

pub fn profile_aggressive_fill(depth: u64) {
    let (mut book, mut id, mut fills) = fresh_book_asks(depth);
    let refill_at = (depth / 4).max(10) as usize;

    profile_loop(WARMUP, ITERS, || {
        if book.len() < refill_at {
            let fresh = fresh_book_asks(depth);
            book = fresh.0;
            id = fresh.1;
            fills = fresh.2;
        }
        fills.clear();
        book.add_order(
            Order {
                id,
                side: Side::Buy,
                price: MID + SPREAD + 200,
                qty: 1,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        id += 1;
    });
}

pub fn profile_multi_level_sweep(num_levels: u64) {
    let mut fills = Vec::with_capacity(num_levels as usize);
    let mut id = 1u64;

    profile_loop(WARMUP, SWEEP_ITERS, || {
        let mut book = OrderBook::with_capacity(num_levels as usize);
        for l in 0..num_levels {
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side: Side::Sell,
                    price: MID + 1 + l,
                    qty: 10,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
            id += 1;
        }
        fills.clear();
        book.add_order(
            Order {
                id,
                side: Side::Buy,
                price: MID + num_levels,
                qty: num_levels * 10,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        id += 1;
    });
}

pub fn profile_market_order(depth: u64) {
    let (mut book, mut id, mut fills) = fresh_book_asks(depth);
    let refill_at = (depth / 4).max(10) as usize;

    profile_loop(WARMUP, ITERS, || {
        if book.len() < refill_at {
            let fresh = fresh_book_asks(depth);
            book = fresh.0;
            id = fresh.1;
            fills = fresh.2;
        }
        fills.clear();
        book.add_order(
            Order {
                id,
                side: Side::Buy,
                price: 0,
                qty: 1,
                order_type: OrderType::Market,
            },
            &mut fills,
        );
        id += 1;
    });
}

pub fn profile_cancel(depth: u64) {
    let (mut book, mut id, _) = fresh_book_both(depth);
    let mut cancel_id = id - depth;

    profile_loop(WARMUP, ITERS, || {
        if cancel_id >= id {
            let fresh = fresh_book_both(depth);
            book = fresh.0;
            id = fresh.1;
            cancel_id = id - depth;
        }
        book.cancel(cancel_id);
        cancel_id += 1;
    });
}

pub fn profile_cancel_hot_level(orders_per_level: u64) {
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    let price = MID + SPREAD;
    let iters = ITERS.min(orders_per_level);

    let mut book = OrderBook::new();

    let mut seed = |book: &mut OrderBook, id: &mut u64| -> u64 {
        let fid = *id;
        for _ in 0..WARMUP + orders_per_level {
            fills.clear();
            book.add_order(
                Order {
                    id: *id,
                    side: Side::Sell,
                    price,
                    qty: 10,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
            *id += 1;
        }
        fid
    };

    let mut cancel_id = seed(&mut book, &mut id);

    profile_loop(WARMUP, iters, || {
        if cancel_id >= id {
            book = OrderBook::new();
            cancel_id = seed(&mut book, &mut id);
        }
        book.cancel(cancel_id);
        cancel_id += 1;
    });
}

pub fn profile_drain_single_level(orders: u64) {
    let mut fills = Vec::with_capacity(orders as usize);
    let mut id = 1u64;
    let price = MID + SPREAD;

    profile_loop(WARMUP, SWEEP_ITERS, || {
        let mut book = OrderBook::with_capacity(orders as usize);
        for _ in 0..orders {
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side: Side::Sell,
                    price,
                    qty: 1,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
            id += 1;
        }
        fills.clear();
        book.add_order(
            Order {
                id,
                side: Side::Buy,
                price,
                qty: orders,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        id += 1;
    });
}

pub fn profile_mixed_workload(depth: u64) {
    let mut fills = Vec::with_capacity(8);
    let mut id = 1u64;
    let mut book = OrderBook::with_capacity(depth as usize);
    seed_both(&mut book, depth, &mut id, &mut fills);

    let ring_cap = depth.max(4096) as usize;
    let mut cancel_ring: Vec<u64> = (1..=depth).collect();
    let mut ring_idx: usize = 0;

    profile_loop(WARMUP, ITERS, || {
        if book.len() < 50 {
            book = OrderBook::with_capacity(depth as usize);
            id = 1;
            seed_both(&mut book, depth, &mut id, &mut fills);
            cancel_ring = (1..=depth).collect();
            ring_idx = 0;
        }
        let roll = id % 20;

        if roll < 13 {
            if !cancel_ring.is_empty() {
                let cid = cancel_ring[ring_idx % cancel_ring.len()];
                book.cancel(cid);
                ring_idx += 1;
            }
        } else if roll < 18 {
            let (side, price) = if id.is_multiple_of(2) {
                (Side::Buy, MID - SPREAD - 200 - (id % 100))
            } else {
                (Side::Sell, MID + SPREAD + 200 + (id % 100))
            };
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side,
                    price,
                    qty: 10,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
            if cancel_ring.len() < ring_cap {
                cancel_ring.push(id);
            } else {
                cancel_ring[ring_idx % ring_cap] = id;
            }
        } else {
            let (side, price) = if id.is_multiple_of(2) {
                (Side::Buy, MID + SPREAD + 200)
            } else {
                (Side::Sell, MID - SPREAD - 200)
            };
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side,
                    price,
                    qty: 1,
                    order_type: OrderType::Limit,
                },
                &mut fills,
            );
        }

        id += 1;
    });
}

// ── Timer-only (noise floor) ────────────────────────────────────

pub fn timer_only() -> Histogram<u64> {
    let mut x = 0u64;
    timed_loop(WARMUP, ITERS, || {
        std::hint::black_box(&mut x);
    })
}

pub fn profile_timer_only() {
    let mut x = 0u64;
    profile_loop(WARMUP, ITERS, || {
        std::hint::black_box(&mut x);
    });
}

// timer_rdtsc is now identical to timer_only since all timing uses the
// unified timer module (rdtsc on x86_64, Instant fallback).
pub fn timer_rdtsc() -> Histogram<u64> {
    timer_only()
}

/// Tight rdtsc gap detector: stores raw timestamps in a pre-allocated array,
/// analyzes gaps post-hoc. Zero work between consecutive reads — no histogram,
/// no branches, no function calls. Any gap >> median is pure system interference.
///
/// Outlier report includes time offsets (seconds from measurement start) for
/// direct correlation with perf script / ftrace timestamps.
pub fn gap_detector() -> Histogram<u64> {
    timer::calibrate();
    wait_for_nohz();
    let count = (WARMUP + ITERS + 1) as usize;
    let mut hist = new_hist();
    let warmup = WARMUP as usize;

    #[cfg(target_arch = "x86_64")]
    {
        let cpns = timer::cycles_per_ns();
        let mut ts = vec![0u64; count];

        for slot in ts.iter_mut() {
            *slot = unsafe {
                core::arch::x86_64::_mm_lfence();
                core::arch::x86_64::_rdtsc()
            };
        }

        let base = ts[warmup];
        let mut outliers: Vec<(usize, u64, f64)> = Vec::new();

        for i in (warmup + 1)..count {
            let gap_ns = ((ts[i] - ts[i - 1]) as f64 / cpns) as u64;
            hist.record(gap_ns).ok();
            if gap_ns >= OUTLIER_THRESHOLD_NS {
                let offset_s = (ts[i - 1] - base) as f64 / cpns / 1_000_000_000.0;
                outliers.push((i - warmup, gap_ns, offset_s));
            }
        }

        if !outliers.is_empty() {
            eprintln!(
                "  outliers (>{OUTLIER_THRESHOLD_NS}ns): {} (offset = seconds from measurement start)",
                outliers.len()
            );
            for &(iter, gap_ns, offset_s) in &outliers {
                eprintln!("    +{offset_s:.6}s  iter={iter:<8} {gap_ns}ns");
            }
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let epoch = std::time::Instant::now();
        let mut ts = vec![0u64; count];

        for slot in ts.iter_mut() {
            *slot = epoch.elapsed().as_nanos() as u64;
        }

        let base = ts[warmup];
        let mut outliers: Vec<(usize, u64, f64)> = Vec::new();

        for i in (warmup + 1)..count {
            let gap_ns = ts[i] - ts[i - 1];
            hist.record(gap_ns).ok();
            if gap_ns >= OUTLIER_THRESHOLD_NS {
                let offset_s = (ts[i - 1] - base) as f64 / 1_000_000_000.0;
                outliers.push((i - warmup, gap_ns, offset_s));
            }
        }

        if !outliers.is_empty() {
            eprintln!(
                "  outliers (>{OUTLIER_THRESHOLD_NS}ns): {} (offset = seconds from measurement start)",
                outliers.len()
            );
            for &(iter, gap_ns, offset_s) in &outliers {
                eprintln!("    +{offset_s:.6}s  iter={iter:<8} {gap_ns}ns");
            }
        }
    }

    hist
}

pub fn profile_gap_detector() {
    timer::calibrate();
    let count = (WARMUP + ITERS + 1) as usize;

    #[cfg(target_arch = "x86_64")]
    {
        let mut ts = vec![0u64; count];
        for slot in ts.iter_mut() {
            *slot = unsafe {
                core::arch::x86_64::_mm_lfence();
                core::arch::x86_64::_rdtsc()
            };
        }
        std::hint::black_box(&ts);
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        let epoch = std::time::Instant::now();
        let mut ts = vec![0u64; count];
        for slot in ts.iter_mut() {
            *slot = epoch.elapsed().as_nanos() as u64;
        }
        std::hint::black_box(&ts);
    }
}
