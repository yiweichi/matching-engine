use std::collections::{BTreeMap, VecDeque};

use rustc_hash::FxHashMap;

use crate::types::*;

// ── Internal types ──────────────────────────────────────────────

struct RestingOrder {
    id: OrderId,
    qty: Qty,
}

struct PriceLevel {
    orders: VecDeque<RestingOrder>,
}

impl PriceLevel {
    #[inline]
    fn new() -> Self {
        Self {
            orders: VecDeque::new(),
        }
    }
}

// ── OrderBook ───────────────────────────────────────────────────

pub struct OrderBook {
    /// Bid side – keys are prices, highest = best bid.
    bids: BTreeMap<Price, PriceLevel>,
    /// Ask side – keys are prices, lowest = best ask.
    asks: BTreeMap<Price, PriceLevel>,
    /// O(1) lookup: order_id → (side, price) for fast cancel.
    locations: FxHashMap<OrderId, (Side, Price)>,
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            locations: FxHashMap::default(),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            locations: FxHashMap::with_capacity_and_hasher(cap, Default::default()),
        }
    }

    // ── Public API ──────────────────────────────────────────────

    /// Submit an order. Fills are appended to the caller-owned buffer
    /// so the hot path never allocates.
    #[inline]
    pub fn add_order(&mut self, order: Order, fills: &mut Vec<Fill>) {
        let mut remaining = order.qty;

        match order.side {
            Side::Buy => self.match_buy(&order, &mut remaining, fills),
            Side::Sell => self.match_sell(&order, &mut remaining, fills),
        }

        if remaining > 0 && order.order_type == OrderType::Limit {
            self.place(order.id, order.side, order.price, remaining);
        }
    }

    /// Cancel a resting order. Returns `true` if the order existed.
    #[inline]
    pub fn cancel(&mut self, order_id: OrderId) -> bool {
        let (side, price) = match self.locations.remove(&order_id) {
            Some(loc) => loc,
            None => return false,
        };

        let book = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        if let Some(level) = book.get_mut(&price) {
            level.orders.retain(|o| o.id != order_id);
            if level.orders.is_empty() {
                book.remove(&price);
            }
        }

        true
    }

    /// Best bid price, or `None` if the bid side is empty.
    #[inline]
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.keys().next_back().copied()
    }

    /// Best ask price, or `None` if the ask side is empty.
    #[inline]
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.keys().next().copied()
    }

    /// Spread = best_ask - best_bid.  `None` if either side is empty.
    #[inline]
    pub fn spread(&self) -> Option<u64> {
        match (self.best_ask(), self.best_bid()) {
            (Some(a), Some(b)) if a >= b => Some(a - b),
            _ => None,
        }
    }

    /// Number of resting orders on the book.
    #[inline]
    pub fn len(&self) -> usize {
        self.locations.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }

    /// Total resting quantity at a given price on a given side.
    #[inline]
    pub fn depth_at(&self, side: Side, price: Price) -> Qty {
        let book = match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        };
        book.get(&price)
            .map(|lvl| lvl.orders.iter().map(|o| o.qty).sum())
            .unwrap_or(0)
    }

    // ── Matching ────────────────────────────────────────────────

    /// Incoming **buy** matched against the ask side (lowest first).
    #[inline]
    fn match_buy(&mut self, order: &Order, remaining: &mut Qty, fills: &mut Vec<Fill>) {
        while *remaining > 0 {
            let best_ask = match self.asks.keys().next().copied() {
                Some(p) => p,
                None => return,
            };

            if order.order_type == OrderType::Limit && best_ask > order.price {
                return;
            }

            self.fill_at_level(Side::Buy, best_ask, order.id, remaining, fills);

            if self.asks.get(&best_ask).map_or(true, |l| l.orders.is_empty()) {
                self.asks.remove(&best_ask);
            }
        }
    }

    /// Incoming **sell** matched against the bid side (highest first).
    #[inline]
    fn match_sell(&mut self, order: &Order, remaining: &mut Qty, fills: &mut Vec<Fill>) {
        while *remaining > 0 {
            let best_bid = match self.bids.keys().next_back().copied() {
                Some(p) => p,
                None => return,
            };

            if order.order_type == OrderType::Limit && best_bid < order.price {
                return;
            }

            self.fill_at_level(Side::Sell, best_bid, order.id, remaining, fills);

            if self.bids.get(&best_bid).map_or(true, |l| l.orders.is_empty()) {
                self.bids.remove(&best_bid);
            }
        }
    }

    /// Walk a single price level, filling against resting orders.
    /// `taker_side` is the side of the incoming aggressive order.
    #[inline]
    fn fill_at_level(
        &mut self,
        taker_side: Side,
        price: Price,
        taker_id: OrderId,
        remaining: &mut Qty,
        fills: &mut Vec<Fill>,
    ) {
        let level = match taker_side {
            Side::Buy => self.asks.get_mut(&price),
            Side::Sell => self.bids.get_mut(&price),
        };

        let level = match level {
            Some(l) => l,
            None => return,
        };

        while *remaining > 0 {
            let maker = match level.orders.front_mut() {
                Some(m) => m,
                None => return,
            };

            let fill_qty = (*remaining).min(maker.qty);

            fills.push(Fill {
                maker_id: maker.id,
                taker_id,
                price,
                qty: fill_qty,
                side: taker_side,
            });

            *remaining -= fill_qty;
            maker.qty -= fill_qty;

            if maker.qty == 0 {
                let done = level.orders.pop_front().unwrap();
                self.locations.remove(&done.id);
            }
        }
    }

    // ── Placement ───────────────────────────────────────────────

    #[inline]
    fn place(&mut self, id: OrderId, side: Side, price: Price, qty: Qty) {
        let book = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        book.entry(price)
            .or_insert_with(PriceLevel::new)
            .orders
            .push_back(RestingOrder { id, qty });

        self.locations.insert(id, (side, price));
    }
}

impl Default for OrderBook {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(id: u64, side: Side, price: u64, qty: u64) -> Order {
        Order {
            id,
            side,
            price,
            qty,
            order_type: OrderType::Limit,
        }
    }

    fn market(id: u64, side: Side, qty: u64) -> Order {
        Order {
            id,
            side,
            price: 0,
            qty,
            order_type: OrderType::Market,
        }
    }

    #[test]
    fn test_no_match_wide_spread() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Buy, 100, 10), &mut fills);
        assert!(fills.is_empty());
        book.add_order(limit(2, Side::Sell, 110, 10), &mut fills);
        assert!(fills.is_empty());

        assert_eq!(book.best_bid(), Some(100));
        assert_eq!(book.best_ask(), Some(110));
        assert_eq!(book.spread(), Some(10));
        assert_eq!(book.len(), 2);
    }

    #[test]
    fn test_exact_fill() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Sell, 100, 10), &mut fills);
        assert!(fills.is_empty());

        book.add_order(limit(2, Side::Buy, 100, 10), &mut fills);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].maker_id, 1);
        assert_eq!(fills[0].taker_id, 2);
        assert_eq!(fills[0].price, 100);
        assert_eq!(fills[0].qty, 10);

        assert!(book.is_empty());
    }

    #[test]
    fn test_partial_fill() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Sell, 100, 20), &mut fills);
        fills.clear();

        book.add_order(limit(2, Side::Buy, 100, 5), &mut fills);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].qty, 5);

        // 15 left resting on ask
        assert_eq!(book.depth_at(Side::Sell, 100), 15);
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn test_price_improvement() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Sell, 95, 10), &mut fills);
        fills.clear();

        // Buy at 100 should match at 95 (maker's price)
        book.add_order(limit(2, Side::Buy, 100, 10), &mut fills);
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].price, 95);
    }

    #[test]
    fn test_fifo_priority() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Sell, 100, 5), &mut fills);
        book.add_order(limit(2, Side::Sell, 100, 5), &mut fills);
        fills.clear();

        book.add_order(limit(3, Side::Buy, 100, 7), &mut fills);
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].maker_id, 1);
        assert_eq!(fills[0].qty, 5);
        assert_eq!(fills[1].maker_id, 2);
        assert_eq!(fills[1].qty, 2);

        assert_eq!(book.depth_at(Side::Sell, 100), 3);
    }

    #[test]
    fn test_multi_level_fill() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Sell, 100, 5), &mut fills);
        book.add_order(limit(2, Side::Sell, 101, 5), &mut fills);
        book.add_order(limit(3, Side::Sell, 102, 5), &mut fills);
        fills.clear();

        book.add_order(limit(4, Side::Buy, 102, 12), &mut fills);
        assert_eq!(fills.len(), 3);
        assert_eq!(fills[0].price, 100);
        assert_eq!(fills[1].price, 101);
        assert_eq!(fills[2].price, 102);
        assert_eq!(fills[2].qty, 2);

        assert_eq!(book.depth_at(Side::Sell, 102), 3);
    }

    #[test]
    fn test_market_order() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Sell, 100, 10), &mut fills);
        book.add_order(limit(2, Side::Sell, 105, 10), &mut fills);
        fills.clear();

        book.add_order(market(3, Side::Buy, 15), &mut fills);
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].price, 100);
        assert_eq!(fills[0].qty, 10);
        assert_eq!(fills[1].price, 105);
        assert_eq!(fills[1].qty, 5);

        // Market order remainder is discarded
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn test_cancel() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Buy, 100, 10), &mut fills);
        assert_eq!(book.len(), 1);

        assert!(book.cancel(1));
        assert_eq!(book.len(), 0);
        assert!(book.best_bid().is_none());

        // Double cancel returns false
        assert!(!book.cancel(1));
    }

    #[test]
    fn test_cancel_nonexistent() {
        let mut book = OrderBook::new();
        assert!(!book.cancel(999));
    }

    #[test]
    fn test_sell_market() {
        let mut book = OrderBook::new();
        let mut fills = Vec::new();

        book.add_order(limit(1, Side::Buy, 100, 10), &mut fills);
        book.add_order(limit(2, Side::Buy, 99, 10), &mut fills);
        fills.clear();

        book.add_order(market(3, Side::Sell, 15), &mut fills);
        assert_eq!(fills.len(), 2);
        // Should match highest bid first
        assert_eq!(fills[0].price, 100);
        assert_eq!(fills[0].qty, 10);
        assert_eq!(fills[1].price, 99);
        assert_eq!(fills[1].qty, 5);
    }
}
