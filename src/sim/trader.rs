use super::exchange::L1;
use super::strategy::{Action, Strategy};
use matching_engine::{Fill, Qty, Side};
use std::collections::VecDeque;

#[allow(dead_code)]
pub struct TraderConfig {
    pub name: String,
    pub md_latency: u64,
    pub order_latency: u64,
    pub max_position: i64,
}

pub struct Trader {
    pub config: TraderConfig,
    pub position: i64,
    pub cash: i64,
    pub num_buys: u64,
    pub num_sells: u64,
    pub total_buy_qty: Qty,
    pub total_sell_qty: Qty,
    pub total_buy_cost: i64,
    pub total_sell_proceeds: i64,
    pub missed_orders: u64,
    pub stale_fills: u64,
    pub stale_misses: u64,
    pub stale_decision_latency_ticks_sum: u64,
    pub stale_arrival_lag_ticks_sum: u64,
    strategy: Strategy,
    md_queue: VecDeque<(u64, L1)>,
    order_queue: VecDeque<(u64, Action, u64)>,
}

impl Trader {
    pub fn new(config: TraderConfig) -> Self {
        Self {
            strategy: Strategy::new(config.max_position),
            config,
            position: 0,
            cash: 0,
            num_buys: 0,
            num_sells: 0,
            total_buy_qty: 0,
            total_sell_qty: 0,
            total_buy_cost: 0,
            total_sell_proceeds: 0,
            missed_orders: 0,
            stale_fills: 0,
            stale_misses: 0,
            stale_decision_latency_ticks_sum: 0,
            stale_arrival_lag_ticks_sum: 0,
            md_queue: VecDeque::with_capacity(256),
            order_queue: VecDeque::with_capacity(64),
        }
    }

    pub fn queue_md(&mut self, delivery_tick: u64, l1: L1) {
        self.md_queue.push_back((delivery_tick, l1));
    }

    /// Process arrived market data, run strategy, queue any resulting orders.
    pub fn process_md(&mut self, current_tick: u64) {
        let mut latest: Option<L1> = None;
        while let Some(&(delivery, _)) = self.md_queue.front() {
            if delivery <= current_tick {
                let (_, l1) = self.md_queue.pop_front().unwrap();
                latest = Some(l1);
            } else {
                break;
            }
        }

        if let Some(l1) = latest {
            if self.order_queue.is_empty() {
                if let Some(action) = self.strategy.decide(&l1, self.position, current_tick) {
                    let delivery = current_tick + self.config.order_latency;
                    let decision_latency = current_tick.saturating_sub(l1.tick);
                    self.order_queue
                        .push_back((delivery, action, decision_latency));
                }
            }
        }
    }

    /// Return orders that have arrived at the exchange this tick.
    pub fn drain_arrived_orders(&mut self, current_tick: u64) -> Vec<(Action, u64)> {
        let mut orders = Vec::new();
        while let Some(&(delivery, _, _)) = self.order_queue.front() {
            if delivery <= current_tick {
                let (_, action, decision_latency) = self.order_queue.pop_front().unwrap();
                orders.push((action, decision_latency));
            } else {
                break;
            }
        }
        orders
    }

    pub fn record_miss(&mut self) {
        self.missed_orders += 1;
    }

    pub fn record_stale_outcome(
        &mut self,
        filled: bool,
        decision_latency_ticks: u64,
        arrival_lag_ticks: u64,
    ) {
        if filled {
            self.stale_fills += 1;
        } else {
            self.stale_misses += 1;
        }
        self.stale_decision_latency_ticks_sum += decision_latency_ticks;
        self.stale_arrival_lag_ticks_sum += arrival_lag_ticks;
    }

    pub fn apply_fills(&mut self, fills: &[Fill], side: Side) {
        for fill in fills {
            let price = fill.price as i64;
            let qty = fill.qty as i64;
            match side {
                Side::Buy => {
                    self.position += qty;
                    self.cash -= price * qty;
                    self.num_buys += 1;
                    self.total_buy_qty += fill.qty;
                    self.total_buy_cost += price * qty;
                }
                Side::Sell => {
                    self.position -= qty;
                    self.cash += price * qty;
                    self.num_sells += 1;
                    self.total_sell_qty += fill.qty;
                    self.total_sell_proceeds += price * qty;
                }
            }
        }
    }

    pub fn total_pnl(&self) -> i64 {
        self.cash
    }

    pub fn total_trades(&self) -> u64 {
        self.num_buys + self.num_sells
    }

    pub fn avg_buy_price(&self) -> f64 {
        if self.total_buy_qty == 0 {
            return 0.0;
        }
        self.total_buy_cost as f64 / self.total_buy_qty as f64
    }

    pub fn avg_sell_price(&self) -> f64 {
        if self.total_sell_qty == 0 {
            return 0.0;
        }
        self.total_sell_proceeds as f64 / self.total_sell_qty as f64
    }
}
