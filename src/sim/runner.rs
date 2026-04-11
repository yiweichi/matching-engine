use super::exchange::SimExchange;
use super::report::SimResult;
use super::trader::{Trader, TraderConfig};

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
            fast_md_latency: 1,
            fast_order_latency: 1,
            slow_md_latency: 10,
            slow_order_latency: 10,
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

        // Queue market data with respective latencies
        fast.queue_md(tick + cfg.fast_md_latency, l1);
        slow.queue_md(tick + cfg.slow_md_latency, l1);

        // Process arrived market data and run strategies
        fast.process_md(tick);
        slow.process_md(tick);

        // Execute arrived orders (fast trader goes first — realistic priority)
        for action in fast.drain_arrived_orders(tick) {
            let fills = exchange.submit_market(action.side(), action.qty());
            if !fills.is_empty() {
                fast.apply_fills(&fills, action.side());
            }
        }
        for action in slow.drain_arrived_orders(tick) {
            let fills = exchange.submit_market(action.side(), action.qty());
            if !fills.is_empty() {
                slow.apply_fills(&fills, action.side());
            }
        }
    }

    // Final snapshot for mark-to-market
    let final_l1 = exchange.step();
    let mark_price = if final_l1.valid() {
        final_l1.mid() as u64
    } else {
        10_000
    };

    SimResult {
        ticks: cfg.ticks,
        seed: cfg.seed,
        num_events: exchange.num_events,
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
        fast_pnl: fast.total_pnl(mark_price),
        fast_cash: fast.cash,
        fast_avg_buy: fast.avg_buy_price(),
        fast_avg_sell: fast.avg_sell_price(),
        slow_trades: slow.total_trades(),
        slow_buys: slow.num_buys,
        slow_sells: slow.num_sells,
        slow_position: slow.position,
        slow_pnl: slow.total_pnl(mark_price),
        slow_cash: slow.cash,
        slow_avg_buy: slow.avg_buy_price(),
        slow_avg_sell: slow.avg_sell_price(),
    }
}
