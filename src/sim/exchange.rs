use matching_engine::{Fill, Order, OrderBook, OrderId, OrderType, Price, Qty, Side};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use rustc_hash::FxHashMap;

/// Passive fill against a resting HFT order, detected during noise injection.
#[derive(Debug, Clone)]
pub struct HftFillReport {
    pub order_id: OrderId,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
    pub leaves_qty: Qty,
}

struct HftOrderState {
    side: Side,
    remaining_qty: Qty,
}

const INITIAL_MID: Price = 10_000;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct L1 {
    pub tick: u64,
    pub bid: Price,
    pub ask: Price,
    pub bid_qty: Qty,
    pub ask_qty: Qty,
}

impl L1 {
    pub fn mid(&self) -> f64 {
        (self.bid as f64 + self.ask as f64) / 2.0
    }

    pub fn imbalance(&self) -> f64 {
        let total = self.bid_qty + self.ask_qty;
        if total == 0 {
            return 0.0;
        }
        (self.bid_qty as f64 - self.ask_qty as f64) / total as f64
    }

    pub fn valid(&self) -> bool {
        self.bid > 0 && self.ask > 0 && self.ask > self.bid
    }
}

struct FlowEvent {
    side: Side,
    remaining: u64,
}

/// Simulated exchange: real OrderBook + noise model with directional flow events.
///
/// Flow events create one-sided aggressive order flow that moves the price
/// and shifts the order-book imbalance. Directions alternate (buy, sell,
/// buy, …) to prevent net drift. Passive liquidity is mean-reverting around
/// the initial mid price.
pub struct SimExchange {
    book: OrderBook,
    next_id: OrderId,
    rng: SmallRng,
    tick: u64,
    noise_ids: Vec<OrderId>,
    fills_buf: Vec<Fill>,
    active_event: Option<FlowEvent>,
    ticks_to_next_event: u64,
    next_event_side: Side,
    price_low: Price,
    price_high: Price,
    pub num_events: u64,
    // HFT client order tracking
    hft_orders: FxHashMap<OrderId, HftOrderState>,
    pending_hft_fills: Vec<HftFillReport>,
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
            noise_ids: Vec::with_capacity(20_000),
            fills_buf: Vec::with_capacity(64),
            active_event: None,
            ticks_to_next_event: first_event,
            next_event_side: Side::Buy,
            price_low: u64::MAX,
            price_high: 0,
            num_events: 0,
            hft_orders: FxHashMap::default(),
            pending_hft_fills: Vec::new(),
        };
        ex.seed_book(INITIAL_MID);
        ex
    }

    pub fn step(&mut self) -> L1 {
        self.tick += 1;
        self.inject_noise();
        let snap = self.snapshot();
        if snap.valid() {
            let mid = snap.mid() as Price;
            self.price_low = self.price_low.min(mid);
            self.price_high = self.price_high.max(mid);
        }
        snap
    }

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
    pub fn price_low(&self) -> Price {
        self.price_low
    }
    pub fn price_high(&self) -> Price {
        self.price_high
    }

    // ── HFT client interface ─────────────────────────────────────────────

    /// Submit an order on behalf of the HFT client.  Returns immediate fills.
    /// If the order has remaining qty and is a limit order, it rests on the book
    /// and future passive fills will be queued in `pending_hft_fills`.
    pub fn submit_hft_order(
        &mut self,
        id: OrderId,
        side: Side,
        price: Price,
        qty: Qty,
        order_type: OrderType,
    ) -> Vec<Fill> {
        let order = Order {
            id,
            side,
            price,
            qty,
            order_type,
        };
        self.fills_buf.clear();
        self.book.add_order(order, &mut self.fills_buf);
        let fills = self.fills_buf.clone();

        let filled: Qty = fills.iter().map(|f| f.qty).sum();
        let remaining = qty.saturating_sub(filled);

        if remaining > 0 && order_type == OrderType::Limit {
            self.hft_orders.insert(
                id,
                HftOrderState {
                    side,
                    remaining_qty: remaining,
                },
            );
        }

        fills
    }

    /// Cancel a resting HFT limit order. Returns true if the order was found.
    pub fn cancel_hft_order(&mut self, order_id: OrderId) -> bool {
        if self.book.cancel(order_id) {
            self.hft_orders.remove(&order_id);
            true
        } else {
            false
        }
    }

    /// Drain passive fill reports accumulated during the last `step()`.
    pub fn drain_hft_reports(&mut self) -> Vec<HftFillReport> {
        std::mem::take(&mut self.pending_hft_fills)
    }

    fn alloc_id(&mut self) -> OrderId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn seed_book(&mut self, mid: Price) {
        for offset in 1..=50_u64 {
            let n_orders = if offset <= 5 { 10 } else { 5 };
            for _ in 0..n_orders {
                let qty = self.rng.gen_range(1..=10);
                self.place_noise(Side::Buy, mid - offset, qty);
                self.place_noise(Side::Sell, mid + offset, qty);
            }
        }
    }

    fn place_noise(&mut self, side: Side, price: Price, qty: Qty) {
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
        collect_hft_fills(
            &self.fills_buf,
            &mut self.hft_orders,
            &mut self.pending_hft_fills,
        );
        if self.fills_buf.is_empty() {
            self.noise_ids.push(id);
        }
    }

    fn inject_noise(&mut self) {
        // ── 1. Process active flow event ─────────────────────────────
        if let Some(ref mut event) = self.active_event {
            let event_side = event.side;
            let n_aggressive = self.rng.gen_range(1..=2_u32);
            event.remaining -= 1;
            let done = event.remaining == 0;
            for _ in 0..n_aggressive {
                let qty = self.rng.gen_range(1..=3);
                let id = self.alloc_id();
                self.fills_buf.clear();
                self.book.add_order(
                    Order {
                        id,
                        side: event_side,
                        price: 0,
                        qty,
                        order_type: OrderType::Market,
                    },
                    &mut self.fills_buf,
                );
                collect_hft_fills(
                    &self.fills_buf,
                    &mut self.hft_orders,
                    &mut self.pending_hft_fills,
                );
            }
            if done {
                self.active_event = None;
            }
        }

        // ── 2. Maybe start a new event ───────────────────────────────
        if self.active_event.is_none() {
            if self.ticks_to_next_event == 0 {
                let side = self.next_event_side;
                self.next_event_side = match side {
                    Side::Buy => Side::Sell,
                    Side::Sell => Side::Buy,
                };
                let duration = self.rng.gen_range(50..=100);
                self.active_event = Some(FlowEvent {
                    side,
                    remaining: duration,
                });
                self.ticks_to_next_event = self.rng.gen_range(500..=1200);
                self.num_events += 1;
            } else {
                self.ticks_to_next_event -= 1;
            }
        }

        // ── 3. Cancel some resting noise (~3%) ──────────────────────
        let num_cancels = (self.noise_ids.len() / 30).clamp(1, 20);
        for _ in 0..num_cancels {
            if self.noise_ids.is_empty() {
                break;
            }
            let idx = self.rng.gen_range(0..self.noise_ids.len());
            let id = self.noise_ids.swap_remove(idx);
            self.book.cancel(id);
        }

        // ── 4. Replenish passive liquidity (mean-reverting) ─────────
        let bid = self.book.best_bid().unwrap_or(INITIAL_MID - 1);
        let ask = self.book.best_ask().unwrap_or(INITIAL_MID + 1);
        let mid = ((bid + ask) / 2) as f64;

        // Bias new orders towards INITIAL_MID for mean reversion
        let buy_prob = if mid > INITIAL_MID as f64 { 0.6 } else { 0.4 };

        let num_new = self.rng.gen_range(12..=25);
        for _ in 0..num_new {
            let side = if self.rng.gen_bool(buy_prob) {
                Side::Buy
            } else {
                Side::Sell
            };
            let offset = self.rng.gen_range(1..=40_i64);
            let target_mid = (mid as i64 + INITIAL_MID as i64) / 2;
            let price = match side {
                Side::Buy => (target_mid - offset).max(1) as Price,
                Side::Sell => (target_mid + offset).max(1) as Price,
            };
            let qty = self.rng.gen_range(1..=8);
            self.place_noise(side, price, qty);
        }
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
            bid,
            ask,
            bid_qty,
            ask_qty,
        }
    }
}

/// Scan fills for passive hits against resting HFT orders.
/// Uses split borrows to avoid taking `&mut SimExchange`.
fn collect_hft_fills(
    fills_buf: &[Fill],
    hft_orders: &mut FxHashMap<OrderId, HftOrderState>,
    pending_hft_fills: &mut Vec<HftFillReport>,
) {
    if hft_orders.is_empty() {
        return;
    }
    let mut to_remove: Vec<OrderId> = Vec::new();
    for fill in fills_buf {
        if let Some(state) = hft_orders.get_mut(&fill.maker_id) {
            state.remaining_qty = state.remaining_qty.saturating_sub(fill.qty);
            pending_hft_fills.push(HftFillReport {
                order_id: fill.maker_id,
                side: state.side,
                price: fill.price,
                qty: fill.qty,
                leaves_qty: state.remaining_qty,
            });
            if state.remaining_qty == 0 {
                to_remove.push(fill.maker_id);
            }
        }
    }
    for id in to_remove {
        hft_orders.remove(&id);
    }
}
