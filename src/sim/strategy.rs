use super::exchange::L1;
use matching_engine::{Price, Qty, Side};

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Buy { qty: Qty, limit_price: Price },
    Sell { qty: Qty, limit_price: Price },
}

impl Action {
    pub fn side(&self) -> Side {
        match self {
            Action::Buy { .. } => Side::Buy,
            Action::Sell { .. } => Side::Sell,
        }
    }

    pub fn qty(&self) -> Qty {
        match self {
            Action::Buy { qty, .. } | Action::Sell { qty, .. } => *qty,
        }
    }

    pub fn limit_price(&self) -> Price {
        match self {
            Action::Buy { limit_price, .. } | Action::Sell { limit_price, .. } => *limit_price,
        }
    }
}

/// Stale-quote arbitrage strategy.
///
/// The strategy treats `reference_mid` as the leading fair value and the target
/// book's best bid/ask as executable stale quotes. It sends marketable IOC-style
/// limit orders only when the expected edge exceeds `edge_threshold`.
pub struct Strategy {
    edge_threshold: Price,
    order_qty: Qty,
    max_position: i64,
}

impl Strategy {
    pub fn new(max_position: i64) -> Self {
        Self {
            edge_threshold: 2,
            order_qty: 1,
            max_position,
        }
    }

    pub fn decide(&mut self, l1: &L1, position: i64, _current_tick: u64) -> Option<Action> {
        if !l1.valid() {
            return None;
        }

        let reference = l1.reference_mid;

        if reference >= l1.ask.saturating_add(self.edge_threshold) && position < self.max_position {
            return Some(Action::Buy {
                qty: self.order_qty,
                limit_price: l1.ask,
            });
        }

        if l1.bid >= reference.saturating_add(self.edge_threshold) && position > -self.max_position
        {
            return Some(Action::Sell {
                qty: self.order_qty,
                limit_price: l1.bid,
            });
        }

        None
    }
}
