use matching_engine::Price;
use std::fmt::Write as _;

pub struct SimResult {
    pub ticks: u64,
    pub seed: u64,
    pub num_events: u64,
    pub expired_stale_quotes: u64,
    pub fast_md_latency: u64,
    pub fast_order_latency: u64,
    pub slow_md_latency: u64,
    pub slow_order_latency: u64,
    pub max_position: i64,
    pub price_low: Price,
    pub price_high: Price,
    pub mark_price: Price,

    pub fast_trades: u64,
    pub fast_buys: u64,
    pub fast_sells: u64,
    pub fast_position: i64,
    pub fast_pnl: i64,
    pub fast_cash: i64,
    pub fast_avg_buy: f64,
    pub fast_avg_sell: f64,
    pub fast_missed_orders: u64,
    pub fast_stale_fills: u64,
    pub fast_stale_misses: u64,
    pub fast_stale_decision_latency_ticks_sum: u64,
    pub fast_stale_arrival_lag_ticks_sum: u64,

    pub slow_trades: u64,
    pub slow_buys: u64,
    pub slow_sells: u64,
    pub slow_position: i64,
    pub slow_pnl: i64,
    pub slow_cash: i64,
    pub slow_avg_buy: f64,
    pub slow_avg_sell: f64,
    pub slow_missed_orders: u64,
    pub slow_stale_fills: u64,
    pub slow_stale_misses: u64,
    pub slow_stale_decision_latency_ticks_sum: u64,
    pub slow_stale_arrival_lag_ticks_sum: u64,
}

impl SimResult {
    pub fn format_report(&self) -> String {
        let mut s = String::with_capacity(2048);

        let _ = writeln!(s, "=== Stale Quote Arbitrage Simulation Report ===");
        let _ = writeln!(s);
        let _ = writeln!(s, "Configuration:");
        let _ = writeln!(s, "  Ticks:           {:>12}", fmt_num(self.ticks));
        let _ = writeln!(s, "  Seed:            {:>12}", self.seed);
        let _ = writeln!(s, "  Max position:    {:>12}", self.max_position);
        let _ = writeln!(s, "  Ref jumps:       {:>12}", fmt_num(self.num_events));
        let _ = writeln!(s);

        let fast_rt = self.fast_md_latency + self.fast_order_latency;
        let slow_rt = self.slow_md_latency + self.slow_order_latency;
        let _ = writeln!(
            s,
            "  Trader A (fast): {} md + {} order = {} tick round-trip",
            self.fast_md_latency, self.fast_order_latency, fast_rt
        );
        let _ = writeln!(
            s,
            "  Trader B (slow): {} md + {} order = {} tick round-trip",
            self.slow_md_latency, self.slow_order_latency, slow_rt
        );
        let _ = writeln!(s);

        let _ = writeln!(s, "--- Market Summary ---");
        let _ = writeln!(
            s,
            "  Reference range: {} - {}",
            fmt_num(self.price_low),
            fmt_num(self.price_high)
        );
        let _ = writeln!(s, "  Final reference: {}", fmt_num(self.mark_price));
        let _ = writeln!(s);

        let stale_fills = self.fast_stale_fills + self.slow_stale_fills;
        let stale_misses = self.fast_stale_misses + self.slow_stale_misses;
        let stale_attempts = stale_fills + stale_misses;
        let capture_rate = if self.num_events > 0 {
            100.0 * stale_fills as f64 / self.num_events as f64
        } else {
            0.0
        };
        let avg_decision_latency = avg_u64(
            self.fast_stale_decision_latency_ticks_sum + self.slow_stale_decision_latency_ticks_sum,
            stale_attempts,
        );
        let avg_arrival_lag = avg_u64(
            self.fast_stale_arrival_lag_ticks_sum + self.slow_stale_arrival_lag_ticks_sum,
            stale_attempts,
        );
        let _ = writeln!(s, "--- Stale Quote Summary ---");
        let _ = writeln!(s, "  Stale events:          {}", fmt_num(self.num_events));
        let _ = writeln!(s, "  Stale fills:           {}", stale_fills);
        let _ = writeln!(s, "  Stale misses:          {}", stale_misses);
        let _ = writeln!(s, "  Expired stale quotes:  {}", self.expired_stale_quotes);
        let _ = writeln!(s, "  Stale capture rate:    {:.1}%", capture_rate);
        let _ = writeln!(
            s,
            "  Avg decision latency:  {:.2} ticks",
            avg_decision_latency
        );
        let _ = writeln!(s, "  Avg order arrival lag: {:.2} ticks", avg_arrival_lag);
        let _ = writeln!(s);

        self.format_trader(
            &mut s,
            "Trader A (fast)",
            self.fast_trades,
            self.fast_buys,
            self.fast_sells,
            self.fast_avg_buy,
            self.fast_avg_sell,
            self.fast_position,
            self.fast_cash,
            self.fast_pnl,
            self.fast_missed_orders,
            self.fast_stale_fills,
            self.fast_stale_misses,
            self.fast_stale_decision_latency_ticks_sum,
            self.fast_stale_arrival_lag_ticks_sum,
        );
        let _ = writeln!(s);
        self.format_trader(
            &mut s,
            "Trader B (slow)",
            self.slow_trades,
            self.slow_buys,
            self.slow_sells,
            self.slow_avg_buy,
            self.slow_avg_sell,
            self.slow_position,
            self.slow_cash,
            self.slow_pnl,
            self.slow_missed_orders,
            self.slow_stale_fills,
            self.slow_stale_misses,
            self.slow_stale_decision_latency_ticks_sum,
            self.slow_stale_arrival_lag_ticks_sum,
        );
        let _ = writeln!(s);

        let _ = writeln!(s, "--- Comparison ---");
        let pnl_diff = self.fast_pnl - self.slow_pnl;
        let _ = writeln!(s, "  PnL advantage (A - B): {:+}", pnl_diff);

        let latency_ratio = if fast_rt > 0 {
            slow_rt as f64 / fast_rt as f64
        } else {
            f64::INFINITY
        };
        let _ = writeln!(s, "  Latency ratio:         {:.1}x", latency_ratio);
        let _ = writeln!(
            s,
            "  Fill advantage:        {:+} fills",
            self.fast_trades as i64 - self.slow_trades as i64
        );
        let _ = writeln!(
            s,
            "  Miss difference:       {:+} missed IOC orders",
            self.fast_missed_orders as i64 - self.slow_missed_orders as i64
        );

        if self.fast_avg_buy > 0.0 && self.slow_avg_buy > 0.0 {
            let buy_slip = self.slow_avg_buy - self.fast_avg_buy;
            let _ = writeln!(
                s,
                "  Avg buy slippage:      B pays {:.1} ticks more per unit",
                buy_slip
            );
        }
        if self.fast_avg_sell > 0.0 && self.slow_avg_sell > 0.0 {
            let sell_slip = self.fast_avg_sell - self.slow_avg_sell;
            let _ = writeln!(
                s,
                "  Avg sell slippage:     B earns {:.1} ticks less per unit",
                sell_slip
            );
        }
        let _ = writeln!(s);

        s
    }

    #[allow(clippy::too_many_arguments)]
    fn format_trader(
        &self,
        s: &mut String,
        name: &str,
        trades: u64,
        buys: u64,
        sells: u64,
        avg_buy: f64,
        avg_sell: f64,
        position: i64,
        cash: i64,
        pnl: i64,
        missed_orders: u64,
        stale_fills: u64,
        stale_misses: u64,
        stale_decision_latency_ticks_sum: u64,
        stale_arrival_lag_ticks_sum: u64,
    ) {
        let stale_attempts = stale_fills + stale_misses;
        let capture_rate = if self.num_events > 0 {
            100.0 * stale_fills as f64 / self.num_events as f64
        } else {
            0.0
        };
        let _ = writeln!(s, "--- {} ---", name);
        let _ = writeln!(
            s,
            "  Fills:           {} ({} buys, {} sells)",
            trades, buys, sells
        );
        let _ = writeln!(s, "  Missed IOC:      {}", missed_orders);
        let _ = writeln!(
            s,
            "  Stale:           {} fills, {} misses, {:.1}% capture",
            stale_fills, stale_misses, capture_rate
        );
        let _ = writeln!(
            s,
            "  Stale latency:   {:.2} decision ticks, {:.2} arrival ticks",
            avg_u64(stale_decision_latency_ticks_sum, stale_attempts),
            avg_u64(stale_arrival_lag_ticks_sum, stale_attempts)
        );
        if buys > 0 {
            let _ = writeln!(s, "  Avg buy price:   {:.1}", avg_buy);
        }
        if sells > 0 {
            let _ = writeln!(s, "  Avg sell price:  {:.1}", avg_sell);
        }
        let _ = writeln!(s, "  Final position:  {:+}", position);
        let _ = writeln!(s, "  Cash:            {:+}", cash);
        let _ = writeln!(s, "  Total PnL:       {:+}", pnl);
    }

    pub fn save(&self) {
        let report = self.format_report();
        let dir = results_dir();
        mkdirs(&dir);
        let ts = fmt_utc_timestamp();
        let path = format!("{}/{}.txt", dir, ts);

        match std::fs::write(&path, &report) {
            Ok(()) => eprintln!("report saved to {}", path),
            Err(e) => eprintln!("warning: failed to save report: {}", e),
        }
    }
}

fn results_dir() -> String {
    let platform = if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    };
    format!("results/sim/{}", platform)
}

fn fmt_utc_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let secs = now % 60;
    let mins = (now / 60) % 60;
    let hours = (now / 3600) % 24;
    let days = now / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}",
        y, m, d, hours, mins, secs
    )
}

fn days_to_ymd(days_since_epoch: u64) -> (u64, u64, u64) {
    // Simplified civil calendar from days since 1970-01-01
    let z = days_since_epoch + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn mkdirs(path: &str) {
    let _ = std::fs::create_dir_all(path);
}

fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn avg_u64(sum: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        sum as f64 / count as f64
    }
}
