use matching_engine::{Fill, Order, OrderBook, OrderId, OrderType, Price, Qty, Side};
const INITIAL_MID: Price = 10_000;
const HALF_SPREAD: Price = 1;
const LEVELS: Price = 50;
const TOP_LEVEL_QTY: Qty = 1;
const DEEP_LEVEL_QTY: Qty = 250;
const REFERENCE_EVENT_INTERVAL: u64 = 300;
const REPRICE_DELAY: u64 = 20;

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

#[derive(Debug, Clone)]
pub struct IocResult {
    pub fills: Vec<Fill>,
    pub stale_outcome: Option<StaleOutcome>,
}

#[derive(Debug, Clone, Copy)]
pub struct StaleOutcome {
    pub filled: bool,
    pub arrival_lag_ticks: u64,
}

#[derive(Debug, Clone, Copy)]
struct StaleQuote {
    side: Side,
    price: Price,
    event_tick: u64,
    active: bool,
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
    tick: u64,
    reference_mid: Price,
    target_mid: Price,
    last_reference_jump_tick: u64,
    ticks_to_next_event: u64,
    next_event_side: Side,
    price_low: Price,
    price_high: Price,
    pub num_events: u64,
    expired_stale_quotes: u64,
    stale_quote: Option<StaleQuote>,
    fills_buf: Vec<Fill>,
}

impl SimExchange {
    pub fn new() -> Self {
        let mut ex = Self {
            book: OrderBook::with_capacity(200_000),
            next_id: 1_000_000,
            tick: 0,
            reference_mid: INITIAL_MID,
            target_mid: INITIAL_MID,
            last_reference_jump_tick: 0,
            ticks_to_next_event: REFERENCE_EVENT_INTERVAL,
            next_event_side: Side::Buy,
            price_low: INITIAL_MID,
            price_high: INITIAL_MID,
            num_events: 0,
            expired_stale_quotes: 0,
            stale_quote: None,
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

    pub fn submit_ioc_limit_at(&mut self, side: Side, price: Price, qty: Qty) -> IocResult {
        let stale_attempt = self.stale_attempt(side, price);
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
        let fills = self.fills_buf.clone();
        let stale_outcome = stale_attempt.map(|mut outcome| {
            if self.stale_fill_matched(&fills) {
                outcome.filled = true;
                if let Some(stale) = &mut self.stale_quote {
                    stale.active = false;
                }
            }
            outcome
        });

        IocResult {
            fills,
            stale_outcome,
        }
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

    pub fn debug_l1(&self) -> L1 {
        self.snapshot()
    }

    pub fn price_low(&self) -> Price {
        self.price_low
    }

    pub fn price_high(&self) -> Price {
        self.price_high
    }

    pub fn stale_events(&self) -> u64 {
        self.num_events
    }

    pub fn expired_stale_quotes(&self) -> u64 {
        self.expired_stale_quotes
    }

    fn advance_reference(&mut self) {
        if self.ticks_to_next_event > 0 {
            self.ticks_to_next_event -= 1;
            return;
        }

        let old_mid = self.target_mid;
        let jump = 6;
        let jump_side = self.next_event_side;

        match jump_side {
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
        self.ticks_to_next_event = REFERENCE_EVENT_INTERVAL;
        self.num_events += 1;
        self.rebuild_book_with_stale_quote(old_mid, self.reference_mid, jump_side);
    }

    fn advance_target_book(&mut self) {
        if self.target_mid == self.reference_mid {
            return;
        }
        if self.tick < self.last_reference_jump_tick + REPRICE_DELAY {
            return;
        }

        if let Some(stale) = &mut self.stale_quote {
            if stale.active {
                self.expired_stale_quotes += 1;
                stale.active = false;
            }
        }
        self.target_mid = self.reference_mid;
        self.rebuild_book(self.target_mid);
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

    fn rebuild_book_with_stale_quote(&mut self, old_mid: Price, new_mid: Price, jump_side: Side) {
        self.book = OrderBook::with_capacity(200_000);

        match jump_side {
            Side::Buy => {
                self.place_side(Side::Buy, old_mid);
                let stale_price = old_mid + HALF_SPREAD;
                self.place_liquidity(Side::Sell, stale_price, TOP_LEVEL_QTY);
                self.place_side(Side::Sell, new_mid);
                self.stale_quote = Some(StaleQuote {
                    side: Side::Buy,
                    price: stale_price,
                    event_tick: self.tick,
                    active: true,
                });
            }
            Side::Sell => {
                self.place_side(Side::Sell, old_mid);
                let stale_price = old_mid.saturating_sub(HALF_SPREAD).max(1);
                self.place_liquidity(Side::Buy, stale_price, TOP_LEVEL_QTY);
                self.place_side(Side::Buy, new_mid);
                self.stale_quote = Some(StaleQuote {
                    side: Side::Sell,
                    price: stale_price,
                    event_tick: self.tick,
                    active: true,
                });
            }
        }
    }

    fn stale_attempt(&self, side: Side, price: Price) -> Option<StaleOutcome> {
        let stale = self.stale_quote?;
        let price_crosses = match side {
            Side::Buy => price >= stale.price,
            Side::Sell => price <= stale.price,
        };
        if side != stale.side || !price_crosses {
            return None;
        }
        Some(StaleOutcome {
            filled: false,
            arrival_lag_ticks: self.tick.saturating_sub(stale.event_tick),
        })
    }

    fn stale_fill_matched(&self, fills: &[Fill]) -> bool {
        let Some(stale) = self.stale_quote else {
            return false;
        };
        stale.active && fills.iter().any(|fill| fill.price == stale.price)
    }

    fn place_side(&mut self, side: Side, mid: Price) {
        for offset in 1..=LEVELS {
            let price = match side {
                Side::Buy => mid.saturating_sub(offset).max(1),
                Side::Sell => mid + offset,
            };
            self.place_liquidity(side, price, DEEP_LEVEL_QTY);
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
