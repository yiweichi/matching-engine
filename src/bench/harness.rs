use std::fs;
use std::process::Command;
use std::time::SystemTime;

use hdrhistogram::Histogram;

pub const MID: u64 = 10_000;
pub const SPREAD: u64 = 50;
pub const WARMUP: u64 = 2_000;
pub const ITERS: u64 = 200_000;
pub const SWEEP_ITERS: u64 = 50_000;

fn results_dir() -> &'static str {
    if cfg!(target_os = "macos") {
        "results/mac"
    } else if cfg!(target_os = "linux") {
        "results/linux"
    } else {
        "results/other"
    }
}

// ── Reporter (prints + collects text) ────────────────────────────

pub struct Reporter {
    output: String,
    combined: Histogram<u64>,
    total_ops: u64,
}

impl Reporter {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            combined: new_hist(),
            total_ops: 0,
        }
    }

    pub fn git_version(&mut self) {
        let hash = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let dirty = Command::new("git")
            .args(["status", "--porcelain"])
            .output()
            .ok()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);

        let version = format!(
            "    git: {}{}",
            hash.trim(),
            if dirty { " (dirty)" } else { "" }
        );
        self.header(&version);
    }

    pub fn header(&mut self, text: &str) {
        println!("{text}");
        self.output.push_str(text);
        self.output.push('\n');
    }

    pub fn section(&mut self, title: &str) {
        let inner_w: usize = 88;
        let title_w: usize = inner_w + 6;
        let title_prefix = format!("── {} (ns) ", title);
        let title_bar = format!(
            "{}{}",
            title_prefix,
            "─".repeat(title_w.saturating_sub(title_prefix.len()))
        );
        let line = format!(
            "\n{}\n  {:<22} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}\n  {}",
            title_bar,
            "",
            "p50",
            "p99",
            "p99.9",
            "p99.99",
            "min",
            "max",
            "─".repeat(inner_w),
        );
        println!("{line}");
        self.output.push_str(&line);
        self.output.push('\n');
    }

    pub fn row(&mut self, label: &str, hist: &Histogram<u64>) {
        self.combined.add(hist).ok();
        self.total_ops += hist.len();

        let line = format!(
            "  {:<22} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
            label,
            fmt_ns(hist.value_at_percentile(50.0)),
            fmt_ns(hist.value_at_percentile(99.0)),
            fmt_ns(hist.value_at_percentile(99.9)),
            fmt_ns(hist.value_at_percentile(99.99)),
            fmt_ns(hist.min()),
            fmt_ns(hist.max()),
        );
        println!("{line}");
        self.output.push_str(&line);
        self.output.push('\n');
    }

    pub fn summary(&mut self, elapsed: std::time::Duration) {
        let h = &self.combined;
        let throughput = if elapsed.as_nanos() > 0 {
            (self.total_ops as f64 / elapsed.as_secs_f64()) as u64
        } else {
            0
        };

        let line = format!(
            concat!(
                "\n── Summary ───────────────────────────────────────────\n",
                "  Total ops:          {}\n",
                "  Throughput:         {} ops/sec\n",
                "  Latency (all ops):  p50={} ns  p99={} ns  p99.9={} ns  max={} ns\n",
                "  Benchmark time:     {:.2?}",
            ),
            fmt_count(self.total_ops),
            fmt_count(throughput),
            fmt_ns(h.value_at_percentile(50.0)),
            fmt_ns(h.value_at_percentile(99.0)),
            fmt_ns(h.value_at_percentile(99.9)),
            fmt_ns(h.max()),
            elapsed,
        );
        println!("{line}");
        self.output.push_str(&line);
        self.output.push('\n');
    }

    pub fn save(self) {
        let dir = results_dir();
        fs::create_dir_all(dir).ok();
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let name = fmt_utc(ts);
        let path = format!("{dir}/{name}.txt");
        fs::write(&path, &self.output).unwrap();
    }
}

// ── Timestamp ────────────────────────────────────────────────────

/// Convert unix timestamp to "20260305T200748" (compact ISO-ish).
fn fmt_utc(secs: u64) -> String {
    let s = secs as i64;
    let days = s.div_euclid(86400);
    let time = s.rem_euclid(86400);
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let sec = time % 60;

    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if mo <= 2 { y + 1 } else { y };

    format!("{yr:04}{mo:02}{d:02}T{h:02}{m:02}{sec:02}")
}

// ── Formatting / printing ───────────────────────────────────────

pub fn new_hist() -> Histogram<u64> {
    Histogram::<u64>::new_with_bounds(1, 1_000_000_000, 3).unwrap()
}

/// Format a number with comma grouping: 1,234,567
fn fmt_count(n: u64) -> String {
    fmt_ns(n)
}

/// Format nanoseconds with comma grouping: 1,234,567
fn fmt_ns(ns: u64) -> String {
    let s = ns.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

pub fn fmt_depth(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}
