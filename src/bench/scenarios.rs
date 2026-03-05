use std::time::Instant;

use hdrhistogram::Histogram;
use matching_engine::*;

use super::harness::*;

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

/// Shared timing loop: runs `warmup + iters` iterations, records only after warmup.
fn timed_loop(warmup: u64, iters: u64, mut body: impl FnMut()) -> Histogram<u64> {
    let mut hist = new_hist();
    for i in 0..(warmup + iters) {
        let t = Instant::now();
        body();
        if i >= warmup {
            hist.record(t.elapsed().as_nanos() as u64).ok();
        }
    }
    hist
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

pub fn aggressive_fill(depth: u64) -> Histogram<u64> {
    let (mut book, mut id, mut fills) = fresh_book_asks(depth);
    let refill_at = (depth / 4).max(10) as usize;

    timed_loop(WARMUP, ITERS, || {
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
    })
}

pub fn multi_level_sweep(num_levels: u64) -> Histogram<u64> {
    let mut fills = Vec::with_capacity(num_levels as usize);
    let mut id = 1u64;

    timed_loop(WARMUP, SWEEP_ITERS, || {
        let mut book = OrderBook::new();
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
    })
}

pub fn market_order(depth: u64) -> Histogram<u64> {
    let (mut book, mut id, mut fills) = fresh_book_asks(depth);
    let refill_at = (depth / 4).max(10) as usize;

    timed_loop(WARMUP, ITERS, || {
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
    })
}

pub fn cancel(depth: u64) -> Histogram<u64> {
    let (mut book, mut id, mut fills) = fresh_book_both(depth);
    let mut cancel_id = id - depth;

    timed_loop(WARMUP, ITERS, || {
        if cancel_id >= id {
            let fresh = fresh_book_both(depth);
            book = fresh.0;
            id = fresh.1;
            fills = fresh.2;
            cancel_id = id - depth;
        }
        book.cancel(cancel_id);
        cancel_id += 1;
    })
}

pub fn cancel_hot_level(orders_per_level: u64) -> Histogram<u64> {
    let mut fills = Vec::with_capacity(4);
    let mut id = 1u64;
    let price = MID + SPREAD;

    let mut book = OrderBook::new();
    let mut first_id = id;
    for _ in 0..orders_per_level {
        fills.clear();
        book.add_order(
            Order {
                id,
                side: Side::Sell,
                price,
                qty: 10,
                order_type: OrderType::Limit,
            },
            &mut fills,
        );
        id += 1;
    }
    let mut cancel_id = first_id;

    timed_loop(WARMUP, ITERS.min(orders_per_level), || {
        if cancel_id >= id {
            book = OrderBook::new();
            first_id = id;
            for _ in 0..orders_per_level {
                fills.clear();
                book.add_order(
                    Order {
                        id,
                        side: Side::Sell,
                        price,
                        qty: 10,
                        order_type: OrderType::Limit,
                    },
                    &mut fills,
                );
                id += 1;
            }
            cancel_id = first_id;
        }
        book.cancel(cancel_id);
        cancel_id += 1;
    })
}

pub fn mixed_workload(depth: u64) -> Histogram<u64> {
    let mut fills = Vec::with_capacity(8);
    let mut id = 1u64;
    let mut book = OrderBook::with_capacity(depth as usize);
    seed_both(&mut book, depth, &mut id, &mut fills);

    let ring_cap = depth.max(4096) as usize;
    let mut cancel_ring: Vec<u64> = (1..=depth).collect();
    let mut ring_idx: usize = 0;

    timed_loop(WARMUP, ITERS, || {
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

        if book.len() < 50 {
            book = OrderBook::with_capacity(depth as usize);
            id = 1;
            seed_both(&mut book, depth, &mut id, &mut fills);
            cancel_ring = (1..=depth).collect();
            ring_idx = 0;
        }
    })
}
