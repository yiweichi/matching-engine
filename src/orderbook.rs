use std::collections::BTreeMap;

use rustc_hash::FxHashMap;

use crate::types::*;

// ── Arena allocator ────────────────────────────────────────────

const INVALID: u32 = u32::MAX;

struct Node {
    id: OrderId,
    qty: Qty,
    prev: u32,
    next: u32,
}

enum Slot {
    Occupied(Node),
    Vacant { next_free: u32 },
}

struct Arena {
    slots: Vec<Slot>,
    free_head: u32,
}

impl Arena {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_head: INVALID,
        }
    }

    fn with_capacity(cap: usize) -> Self {
        Self {
            slots: Vec::with_capacity(cap),
            free_head: INVALID,
        }
    }

    #[inline]
    fn alloc(&mut self, node: Node) -> u32 {
        if self.free_head != INVALID {
            let idx = self.free_head;
            match &self.slots[idx as usize] {
                Slot::Vacant { next_free } => self.free_head = *next_free,
                Slot::Occupied(_) => unreachable!(),
            }
            self.slots[idx as usize] = Slot::Occupied(node);
            idx
        } else {
            let idx = self.slots.len() as u32;
            self.slots.push(Slot::Occupied(node));
            idx
        }
    }

    #[inline]
    fn dealloc(&mut self, idx: u32) {
        self.slots[idx as usize] = Slot::Vacant {
            next_free: self.free_head,
        };
        self.free_head = idx;
    }

    #[inline]
    fn get(&self, idx: u32) -> &Node {
        match &self.slots[idx as usize] {
            Slot::Occupied(node) => node,
            Slot::Vacant { .. } => unreachable!(),
        }
    }

    #[inline]
    fn get_mut(&mut self, idx: u32) -> &mut Node {
        match &mut self.slots[idx as usize] {
            Slot::Occupied(node) => node,
            Slot::Vacant { .. } => unreachable!(),
        }
    }
}

// ── Price level ────────────────────────────────────────────────

struct PriceLevel {
    head: u32,
    tail: u32,
}

impl PriceLevel {
    #[inline]
    fn new() -> Self {
        Self {
            head: INVALID,
            tail: INVALID,
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.head == INVALID
    }
}

// ── OrderBook ──────────────────────────────────────────────────

pub struct OrderBook {
    /// Bid side – keys are prices, highest = best bid.
    bids: BTreeMap<Price, PriceLevel>,
    /// Ask side – keys are prices, lowest = best ask.
    asks: BTreeMap<Price, PriceLevel>,
    /// O(1) lookup: order_id → (side, price, arena_idx).
    locations: FxHashMap<OrderId, (Side, Price, u32)>,
    /// Slab allocator for order nodes.
    arena: Arena,
    /// Cached best prices – O(1) access, avoids BTreeMap traversal.
    cached_best_bid: Option<Price>,
    cached_best_ask: Option<Price>,
}

impl OrderBook {
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            locations: FxHashMap::default(),
            arena: Arena::new(),
            cached_best_bid: None,
            cached_best_ask: None,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            locations: FxHashMap::with_capacity_and_hasher(cap, Default::default()),
            arena: Arena::with_capacity(cap),
            cached_best_bid: None,
            cached_best_ask: None,
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

    /// Cancel a resting order in O(1). Returns `true` if the order existed.
    #[inline]
    pub fn cancel(&mut self, order_id: OrderId) -> bool {
        let (side, price, idx) = match self.locations.remove(&order_id) {
            Some(loc) => loc,
            None => return false,
        };

        let (prev, next) = {
            let node = self.arena.get(idx);
            (node.prev, node.next)
        };

        // Unlink from doubly-linked list — O(1)
        if prev != INVALID {
            self.arena.get_mut(prev).next = next;
        }
        if next != INVALID {
            self.arena.get_mut(next).prev = prev;
        }

        // Update price level head/tail
        let book = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let mut remove_level = false;
        if let Some(level) = book.get_mut(&price) {
            if level.head == idx {
                level.head = next;
            }
            if level.tail == idx {
                level.tail = prev;
            }
            remove_level = level.is_empty();
        }
        if remove_level {
            book.remove(&price);
            match side {
                Side::Buy => {
                    if self.cached_best_bid == Some(price) {
                        self.cached_best_bid = self.bids.keys().next_back().copied();
                    }
                }
                Side::Sell => {
                    if self.cached_best_ask == Some(price) {
                        self.cached_best_ask = self.asks.keys().next().copied();
                    }
                }
            }
        }

        self.arena.dealloc(idx);
        true
    }

    /// Best bid price, or `None` if the bid side is empty. O(1).
    #[inline]
    pub fn best_bid(&self) -> Option<Price> {
        self.cached_best_bid
    }

    /// Best ask price, or `None` if the ask side is empty. O(1).
    #[inline]
    pub fn best_ask(&self) -> Option<Price> {
        self.cached_best_ask
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
        let level = match book.get(&price) {
            Some(l) => l,
            None => return 0,
        };
        let mut total: Qty = 0;
        let mut cur = level.head;
        while cur != INVALID {
            let node = self.arena.get(cur);
            total += node.qty;
            cur = node.next;
        }
        total
    }

    // ── Matching ────────────────────────────────────────────────

    /// Incoming **buy** matched against the ask side (lowest first).
    #[inline]
    fn match_buy(&mut self, order: &Order, remaining: &mut Qty, fills: &mut Vec<Fill>) {
        while *remaining > 0 {
            let best_ask = match self.cached_best_ask {
                Some(p) => p,
                None => return,
            };

            if order.order_type == OrderType::Limit && best_ask > order.price {
                return;
            }

            self.fill_at_level(Side::Buy, best_ask, order.id, remaining, fills);

            if self.asks.get(&best_ask).is_none_or(|l| l.is_empty()) {
                self.asks.remove(&best_ask);
                self.cached_best_ask = self.asks.keys().next().copied();
            }
        }
    }

    /// Incoming **sell** matched against the bid side (highest first).
    #[inline]
    fn match_sell(&mut self, order: &Order, remaining: &mut Qty, fills: &mut Vec<Fill>) {
        while *remaining > 0 {
            let best_bid = match self.cached_best_bid {
                Some(p) => p,
                None => return,
            };

            if order.order_type == OrderType::Limit && best_bid < order.price {
                return;
            }

            self.fill_at_level(Side::Sell, best_bid, order.id, remaining, fills);

            if self.bids.get(&best_bid).is_none_or(|l| l.is_empty()) {
                self.bids.remove(&best_bid);
                self.cached_best_bid = self.bids.keys().next_back().copied();
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
            if level.head == INVALID {
                return;
            }
            let head_idx = level.head;

            let (maker_id, fill_qty, maker_qty, maker_next) = {
                let maker = self.arena.get_mut(head_idx);
                let fq = (*remaining).min(maker.qty);
                maker.qty -= fq;
                *remaining -= fq;
                (maker.id, fq, maker.qty, maker.next)
            };

            fills.push(Fill {
                maker_id,
                taker_id,
                price,
                qty: fill_qty,
                side: taker_side,
            });

            if maker_qty == 0 {
                level.head = maker_next;
                if maker_next != INVALID {
                    self.arena.get_mut(maker_next).prev = INVALID;
                } else {
                    level.tail = INVALID;
                }
                self.arena.dealloc(head_idx);
                self.locations.remove(&maker_id);
            }
        }
    }

    // ── Placement ─────────────────────────────────────────────

    #[inline]
    fn place(&mut self, id: OrderId, side: Side, price: Price, qty: Qty) {
        let idx = self.arena.alloc(Node {
            id,
            qty,
            prev: INVALID,
            next: INVALID,
        });

        let level = match side {
            Side::Buy => self.bids.entry(price).or_insert_with(PriceLevel::new),
            Side::Sell => self.asks.entry(price).or_insert_with(PriceLevel::new),
        };

        let old_tail = level.tail;
        if old_tail != INVALID {
            self.arena.get_mut(old_tail).next = idx;
            self.arena.get_mut(idx).prev = old_tail;
        } else {
            level.head = idx;
        }
        level.tail = idx;

        match side {
            Side::Buy => {
                if self.cached_best_bid.is_none_or(|b| price > b) {
                    self.cached_best_bid = Some(price);
                }
            }
            Side::Sell => {
                if self.cached_best_ask.is_none_or(|a| price < a) {
                    self.cached_best_ask = Some(price);
                }
            }
        }

        self.locations.insert(id, (side, price, idx));
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
