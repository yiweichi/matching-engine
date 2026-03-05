use std::time::Instant;

use matching_engine::{Order, OrderBook, OrderType, Side};

const NUM_SEED_ORDERS: u64 = 100_000;
const NUM_AGGRESSIVE: u64 = 1_000_000;
const MID_PRICE: u64 = 10_000;
const HALF_SPREAD: u64 = 50;

fn main() {
    println!("=== Matching Engine Benchmark ===\n");

    let mut book = OrderBook::with_capacity(NUM_SEED_ORDERS as usize);
    let mut fills = Vec::with_capacity(64);
    let mut id: u64 = 1;

    // ── Phase 1: Seed the book with passive limit orders ────────

    let start = Instant::now();
    for i in 0..NUM_SEED_ORDERS {
        let (side, price) = if i % 2 == 0 {
            (Side::Buy, MID_PRICE - HALF_SPREAD - (i % 200))
        } else {
            (Side::Sell, MID_PRICE + HALF_SPREAD + (i % 200))
        };

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
        fills.clear();
        id += 1;
    }
    let seed_elapsed = start.elapsed();
    println!(
        "Seeded {} passive orders in {:.2?}  ({:.0} orders/sec)",
        NUM_SEED_ORDERS,
        seed_elapsed,
        NUM_SEED_ORDERS as f64 / seed_elapsed.as_secs_f64()
    );
    println!(
        "Book depth: {} orders | best bid={:?} best ask={:?} spread={:?}\n",
        book.len(),
        book.best_bid(),
        book.best_ask(),
        book.spread()
    );

    // ── Phase 2: Fire aggressive limit orders that cross the spread ─

    let mut total_fills: u64 = 0;
    let start = Instant::now();
    for i in 0..NUM_AGGRESSIVE {
        let (side, price) = if i % 2 == 0 {
            (Side::Buy, MID_PRICE + HALF_SPREAD + 200)
        } else {
            (Side::Sell, MID_PRICE - HALF_SPREAD - 200)
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
        total_fills += fills.len() as u64;
        id += 1;
    }
    let aggr_elapsed = start.elapsed();
    println!(
        "Processed {} aggressive orders in {:.2?}  ({:.0} orders/sec)",
        NUM_AGGRESSIVE,
        aggr_elapsed,
        NUM_AGGRESSIVE as f64 / aggr_elapsed.as_secs_f64()
    );
    println!(
        "  avg latency: {:.0} ns/order",
        aggr_elapsed.as_nanos() as f64 / NUM_AGGRESSIVE as f64
    );
    println!("  total fills: {}", total_fills);
    println!(
        "  book depth after: {} orders\n",
        book.len()
    );

    // ── Phase 3: Cancellations (re-seed then cancel) ──────────

    let cancel_n: u64 = 100_000;
    for i in 0..cancel_n {
        let (side, price) = if i % 2 == 0 {
            (Side::Buy, MID_PRICE - HALF_SPREAD - (i % 200))
        } else {
            (Side::Sell, MID_PRICE + HALF_SPREAD + (i % 200))
        };
        fills.clear();
        book.add_order(
            Order { id, side, price, qty: 10, order_type: OrderType::Limit },
            &mut fills,
        );
        id += 1;
    }
    let cancel_start_id = id - cancel_n;

    let start = Instant::now();
    let mut cancelled = 0u64;
    for oid in cancel_start_id..id {
        if book.cancel(oid) {
            cancelled += 1;
        }
    }
    let cancel_elapsed = start.elapsed();
    println!(
        "Cancelled {} orders in {:.2?}  ({:.0} cancels/sec)",
        cancelled,
        cancel_elapsed,
        cancelled as f64 / cancel_elapsed.as_secs_f64()
    );
    println!(
        "  avg latency: {:.0} ns/cancel",
        cancel_elapsed.as_nanos() as f64 / cancelled.max(1) as f64
    );
    println!("  book depth after: {}", book.len());
}
