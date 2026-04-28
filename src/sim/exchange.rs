use matching_engine::{Fill, Order, OrderBook, OrderId, OrderType, Price, Qty, Side};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

const INITIAL_MID: Price = 10_000;
const HALF_SPREAD: Price = 1;
const LEVELS: Price = 50;
const TOP_LEVEL_QTY: Qty = 1;
const DEEP_LEVEL_QTY: Qty = 250;
const REPRICE_DELAY: u64 = 20;
const REPRICE_STEP: Price = 1;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct L1 {
    pub tick: u64,
    pub reference_mid: Price,
    pub bid: Price,
    pub ask: Price,
    pub bid_qty: Qty,
    pub ask_qty: Qty,
}

impl L1 {
    pub fn valid(&self) -> bool {
        self.bid > 0 && self.ask > 0 && self.ask > self.bid
    }
}

/// Simulated target exchange for stale-quote arbitrage.
///
/// `reference_mid` is the leading fair value. The target order book intentionally
/// reprices after a short delay, leaving executable stale quotes for low-latency
/// traders to race for.
pub struct SimExchange {
    book: OrderBook,
    next_id: OrderId,
    rng: SmallRng,
    tick: u64,
    reference_mid: Price,
    target_mid: Price,
    last_reference_jump_tick: u64,
    ticks_to_next_event: u64,
    next_event_side: Side,
    price_low: Price,
    price_high: Price,
    pub num_events: u64,
    fills_buf: Vec<Fill>,
}

impl SimExchange {
    pub fn new(seed: u64) -> Self {
        let mut rng = SmallRng::seed_from_u64(seed);
        let first_event = rng.gen_range(200..=600);
        let mut ex = Self {
            book: OrderBook::with_capacity(200_000),
            next_id: 1_000_000,
            rng,
            tick: 0,
            reference_mid: INITIAL_MID,
            target_mid: INITIAL_MID,
            last_reference_jump_tick: 0,
            ticks_to_next_event: first_event,
            next_event_side: Side::Buy,
            price_low: INITIAL_MID,
            price_high: INITIAL_MID,
            num_events: 0,
            fills_buf: Vec::with_capacity(64),
        };
        ex.rebuild_book(INITIAL_MID);
        ex
    }

    pub fn step(&mut self) -> L1 {
        self.tick += 1;
        self.advance_reference();
        self.advance_target_book();
        let snap = self.snapshot();
        self.price_low = self.price_low.min(self.reference_mid).min(self.target_mid);
        self.price_high = self.price_high.max(self.reference_mid).max(self.target_mid);
        snap
    }

    pub fn submit_ioc_limit(&mut self, side: Side, price: Price, qty: Qty) -> Vec<Fill> {
        let id = self.alloc_id();
        self.fills_buf.clear();
        self.book.add_order(
            Order {
                id,
                side,
                price,
                qty,
                order_type: OrderType::Limit,
            },
            &mut self.fills_buf,
        );
        self.book.cancel(id);
        self.fills_buf.clone()
    }

    #[allow(dead_code)]
    pub fn submit_market(&mut self, side: Side, qty: Qty) -> Vec<Fill> {
        let id = self.alloc_id();
        self.fills_buf.clear();
        self.book.add_order(
            Order {
                id,
                side,
                price: 0,
                qty,
                order_type: OrderType::Market,
            },
            &mut self.fills_buf,
        );
        self.fills_buf.clone()
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn reference_mid(&self) -> Price {
        self.reference_mid
    }

    pub fn target_mid(&self) -> Price {
        self.target_mid
    }

    pub fn debug_l1(&self) -> L1 {
        self.snapshot()
    }

    pub fn price_low(&self) -> Price {
        self.price_low
    }

    pub fn price_high(&self) -> Price {
        self.price_high
    }

    fn advance_reference(&mut self) {
        if self.ticks_to_next_event > 0 {
            self.ticks_to_next_event -= 1;
            return;
        }

        let jump = self.rng.gen_range(3..=6);
        match self.next_event_side {
            Side::Buy => {
                self.reference_mid += jump;
                self.next_event_side = Side::Sell;
            }
            Side::Sell => {
                self.reference_mid = self.reference_mid.saturating_sub(jump).max(HALF_SPREAD + 1);
                self.next_event_side = Side::Buy;
            }
        }

        self.last_reference_jump_tick = self.tick;
        self.ticks_to_next_event = self.rng.gen_range(500..=1200);
        self.num_events += 1;
    }

    fn advance_target_book(&mut self) {
        if self.target_mid == self.reference_mid {
            return;
        }
        if self.tick < self.last_reference_jump_tick + REPRICE_DELAY {
            return;
        }

        let next_mid = if self.target_mid < self.reference_mid {
            (self.target_mid + REPRICE_STEP).min(self.reference_mid)
        } else {
            self.target_mid
                .saturating_sub(REPRICE_STEP)
                .max(self.reference_mid)
        };

        if next_mid != self.target_mid {
            self.target_mid = next_mid;
            self.rebuild_book(next_mid);
        }
    }

    fn rebuild_book(&mut self, mid: Price) {
        self.book = OrderBook::with_capacity(200_000);

        for offset in 1..=LEVELS {
            let bid_price = mid.saturating_sub(offset).max(1);
            let ask_price = mid + offset;
            let qty = if offset <= 10 {
                TOP_LEVEL_QTY
            } else {
                DEEP_LEVEL_QTY
            };
            self.place_liquidity(Side::Buy, bid_price, qty);
            self.place_liquidity(Side::Sell, ask_price, qty);
        }
    }

    fn place_liquidity(&mut self, side: Side, price: Price, qty: Qty) {
        let id = self.alloc_id();
        self.fills_buf.clear();
        self.book.add_order(
            Order {
                id,
                side,
                price,
                qty,
                order_type: OrderType::Limit,
            },
            &mut self.fills_buf,
        );
    }

    fn snapshot(&self) -> L1 {
        let bid = self.book.best_bid().unwrap_or(0);
        let ask = self.book.best_ask().unwrap_or(0);
        let bid_qty = if bid > 0 {
            self.book.depth_at(Side::Buy, bid)
        } else {
            0
        };
        let ask_qty = if ask > 0 {
            self.book.depth_at(Side::Sell, ask)
        } else {
            0
        };
        L1 {
            tick: self.tick,
            reference_mid: self.reference_mid,
            bid,
            ask,
            bid_qty,
            ask_qty,
        }
    }

    fn alloc_id(&mut self) -> OrderId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}
