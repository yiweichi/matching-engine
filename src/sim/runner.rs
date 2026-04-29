use super::exchange::SimExchange;
use super::report::SimResult;
use super::strategy::Action;
use super::trader::{Trader, TraderConfig};
use matching_engine::{Qty, Side};

pub struct SimConfig {
    pub ticks: u64,
    pub fast_md_latency: u64,
    pub fast_order_latency: u64,
    pub slow_md_latency: u64,
    pub slow_order_latency: u64,
    pub max_position: i64,
    pub seed: u64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            ticks: 1_000_000,
            fast_md_latency: 0,
            fast_order_latency: 0,
            slow_md_latency: 100,
            slow_order_latency: 100,
            max_position: 1,
            seed: 42,
        }
    }
}

pub fn run(cfg: &SimConfig) -> SimResult {
    let mut exchange = SimExchange::new(cfg.seed);

    let mut fast = Trader::new(TraderConfig {
        name: "A (fast)".into(),
        md_latency: cfg.fast_md_latency,
        order_latency: cfg.fast_order_latency,
        max_position: cfg.max_position,
    });
    let mut slow = Trader::new(TraderConfig {
        name: "B (slow)".into(),
        md_latency: cfg.slow_md_latency,
        order_latency: cfg.slow_order_latency,
        max_position: cfg.max_position,
    });

    for _ in 0..cfg.ticks {
        let l1 = exchange.step();
        let tick = exchange.tick();

        fast.queue_md(tick + cfg.fast_md_latency, l1);
        slow.queue_md(tick + cfg.slow_md_latency, l1);

        fast.process_md(tick);
        slow.process_md(tick);

        let fast_orders = fast.drain_arrived_orders(tick);
        let slow_orders = slow.drain_arrived_orders(tick);

        if tick.is_multiple_of(2) {
            execute_orders(&mut exchange, &mut fast, fast_orders);
            execute_orders(&mut exchange, &mut slow, slow_orders);
        } else {
            execute_orders(&mut exchange, &mut slow, slow_orders);
            execute_orders(&mut exchange, &mut fast, fast_orders);
        }
    }

    liquidate_position(&mut exchange, &mut fast);
    liquidate_position(&mut exchange, &mut slow);

    let mark_price = exchange.reference_mid();

    SimResult {
        ticks: cfg.ticks,
        seed: cfg.seed,
        num_events: exchange.num_events,
        expired_stale_quotes: exchange.expired_stale_quotes(),
        fast_md_latency: cfg.fast_md_latency,
        fast_order_latency: cfg.fast_order_latency,
        slow_md_latency: cfg.slow_md_latency,
        slow_order_latency: cfg.slow_order_latency,
        max_position: cfg.max_position,
        price_low: exchange.price_low(),
        price_high: exchange.price_high(),
        mark_price,
        fast_trades: fast.total_trades(),
        fast_buys: fast.num_buys,
        fast_sells: fast.num_sells,
        fast_position: fast.position,
        fast_pnl: fast.total_pnl(),
        fast_cash: fast.cash,
        fast_avg_buy: fast.avg_buy_price(),
        fast_avg_sell: fast.avg_sell_price(),
        fast_missed_orders: fast.missed_orders,
        fast_stale_fills: fast.stale_fills,
        fast_stale_misses: fast.stale_misses,
        fast_stale_decision_latency_ticks_sum: fast.stale_decision_latency_ticks_sum,
        fast_stale_arrival_lag_ticks_sum: fast.stale_arrival_lag_ticks_sum,
        slow_trades: slow.total_trades(),
        slow_buys: slow.num_buys,
        slow_sells: slow.num_sells,
        slow_position: slow.position,
        slow_pnl: slow.total_pnl(),
        slow_cash: slow.cash,
        slow_avg_buy: slow.avg_buy_price(),
        slow_avg_sell: slow.avg_sell_price(),
        slow_missed_orders: slow.missed_orders,
        slow_stale_fills: slow.stale_fills,
        slow_stale_misses: slow.stale_misses,
        slow_stale_decision_latency_ticks_sum: slow.stale_decision_latency_ticks_sum,
        slow_stale_arrival_lag_ticks_sum: slow.stale_arrival_lag_ticks_sum,
    }
}

fn execute_orders(exchange: &mut SimExchange, trader: &mut Trader, orders: Vec<(Action, u64)>) {
    for (action, decision_latency_ticks) in orders {
        let result =
            exchange.submit_ioc_limit_at(action.side(), action.limit_price(), action.qty(), 0);
        let fills = result.fills;
        if let Some(outcome) = result.stale_outcome {
            trader.record_stale_outcome(
                outcome.filled,
                decision_latency_ticks,
                outcome.arrival_lag_ticks,
            );
        }
        if fills.is_empty() {
            trader.record_miss();
        } else {
            trader.apply_fills(&fills, action.side());
        }
    }
}

fn liquidate_position(exchange: &mut SimExchange, trader: &mut Trader) {
    let position = trader.position;
    if position > 0 {
        let qty = position as Qty;
        let fills = exchange.submit_market(Side::Sell, qty);
        trader.apply_fills(&fills, Side::Sell);
    } else if position < 0 {
        let qty = (-position) as Qty;
        let fills = exchange.submit_market(Side::Buy, qty);
        trader.apply_fills(&fills, Side::Buy);
    }
}
