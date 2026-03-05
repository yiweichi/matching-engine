use criterion::{black_box, criterion_group, criterion_main, Criterion};
use matching_engine::{Order, OrderBook, OrderType, Side};

fn seed_book(n: u64) -> (OrderBook, u64) {
    let mut book = OrderBook::with_capacity(n as usize);
    let mut fills = Vec::new();
    let mid = 10_000u64;
    let mut id = 1u64;

    for i in 0..n {
        let (side, price) = if i % 2 == 0 {
            (Side::Buy, mid - 50 - (i % 200))
        } else {
            (Side::Sell, mid + 50 + (i % 200))
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
    (book, id)
}

fn bench_add_passive(c: &mut Criterion) {
    c.bench_function("add_passive_limit", |b| {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();
        let mut id = 1u64;
        b.iter(|| {
            let side = if id.is_multiple_of(2) { Side::Buy } else { Side::Sell };
            let price = if side == Side::Buy { 9900 } else { 10100 };
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
        });
    });
}

fn bench_aggressive_fill(c: &mut Criterion) {
    c.bench_function("aggressive_fill_1lot", |b| {
        let (mut book, mut id) = seed_book(10_000);
        let mut fills = Vec::with_capacity(4);
        b.iter(|| {
            let side = if id.is_multiple_of(2) { Side::Buy } else { Side::Sell };
            let price = if side == Side::Buy { 11_000 } else { 9_000 };
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
            black_box(&fills);
            id += 1;

            // Re-seed periodically to keep the book populated
            if book.len() < 1000 {
                let fresh = seed_book(10_000);
                book = fresh.0;
                id = fresh.1;
            }
        });
    });
}

fn bench_cancel(c: &mut Criterion) {
    c.bench_function("cancel_order", |b| {
        let (mut book, _next_id) = seed_book(50_000);
        let mut cancel_id = 1u64;
        b.iter(|| {
            if !book.cancel(black_box(cancel_id)) {
                let fresh = seed_book(50_000);
                book = fresh.0;
                cancel_id = 1;
            }
            cancel_id += 1;
        });
    });
}

fn bench_market_order(c: &mut Criterion) {
    c.bench_function("market_order_10lot", |b| {
        let (mut book, mut id) = seed_book(10_000);
        let mut fills = Vec::with_capacity(16);
        b.iter(|| {
            let side = if id.is_multiple_of(2) { Side::Buy } else { Side::Sell };
            fills.clear();
            book.add_order(
                Order {
                    id,
                    side,
                    price: 0,
                    qty: 10,
                    order_type: OrderType::Market,
                },
                &mut fills,
            );
            black_box(&fills);
            id += 1;

            if book.len() < 1000 {
                let fresh = seed_book(10_000);
                book = fresh.0;
                id = fresh.1;
            }
        });
    });
}

criterion_group!(
    benches,
    bench_add_passive,
    bench_aggressive_fill,
    bench_cancel,
    bench_market_order,
);
criterion_main!(benches);
