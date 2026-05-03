use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum Scenario {
    PassiveInsert,
    AggressiveFill,
    MultiLevelSweep,
    MarketOrder,
    Cancel,
    CancelHotLevel,
    DrainSingleLevel,
    MixedWorkload,
    TimerOnly,
    TimerRdtsc,
    GapDetector,
}

#[derive(Debug, Parser)]
#[command(
    name = "matching-engine",
    about = "Order book latency benchmark runner"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the latency benchmark and report histograms.
    Bench(BenchArgs),
    /// Run a lightweight workload driver for perf/flamegraph.
    Profile(ProfileArgs),
    /// Run a trading simulation: two strategies compete, one deliberately slower.
    Sim(SimArgs),
    /// Run as an exchange server: broadcast market data via UDP, accept orders via TCP.
    Serve(ServeArgs),
}

#[derive(Debug, Args, Default)]
pub struct BenchArgs {
    #[arg(long, value_enum, help = "Only run a single scenario")]
    pub scenario: Option<Scenario>,

    #[arg(
        long,
        help = "Book depth for depth-based scenarios (default: scenario sweep)"
    )]
    pub depth: Option<u64>,

    #[arg(
        long,
        help = "Number of price levels to sweep in multi-level-sweep (default: 1, 5, 10, 50)"
    )]
    pub levels: Option<u64>,

    #[arg(
        long,
        help = "Orders per level / total orders for order-count scenarios (default: scenario sweep)"
    )]
    pub orders: Option<u64>,
}

#[derive(Debug, Args)]
pub struct ProfileArgs {
    #[arg(long, value_enum, help = "Scenario to drive under perf/flamegraph")]
    pub scenario: Scenario,

    #[arg(long, help = "Book depth for depth-based scenarios")]
    pub depth: Option<u64>,

    #[arg(
        long,
        help = "Number of price levels to sweep in multi-level-sweep (recommended: 1, 5, 10, 50)"
    )]
    pub levels: Option<u64>,

    #[arg(
        help = "Orders per level / total orders for order-count scenarios",
        long
    )]
    pub orders: Option<u64>,

    #[arg(
        long,
        default_value_t = 1,
        help = "Repeat the selected workload N times in one process"
    )]
    pub repeat: u64,
}

#[derive(Debug, Args)]
pub struct SimArgs {
    #[arg(long, default_value_t = 1_000_000, help = "Number of simulation ticks")]
    pub ticks: u64,

    #[arg(
        long,
        default_value_t = 0,
        help = "Fast trader: market data latency in ticks"
    )]
    pub fast_md_latency: u64,

    #[arg(
        long,
        default_value_t = 0,
        help = "Fast trader: order submission latency in ticks"
    )]
    pub fast_order_latency: u64,

    #[arg(
        long,
        default_value_t = 100,
        help = "Slow trader: market data latency in ticks"
    )]
    pub slow_md_latency: u64,

    #[arg(
        long,
        default_value_t = 100,
        help = "Slow trader: order submission latency in ticks"
    )]
    pub slow_order_latency: u64,

    #[arg(long, default_value_t = 1, help = "Maximum position per trader")]
    pub max_position: i64,
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    #[arg(
        long,
        default_value_t = 12345,
        help = "UDP port for target market data broadcast"
    )]
    pub md_port: u16,

    #[arg(
        long,
        default_value = "239.1.1.1",
        help = "IPv4 multicast group for target market data broadcast"
    )]
    pub md_group: String,

    #[arg(
        long,
        default_value_t = 12347,
        help = "UDP port for reference price broadcast"
    )]
    pub ref_port: u16,

    #[arg(
        long,
        default_value = "239.1.1.2",
        help = "IPv4 multicast group for reference price broadcast"
    )]
    pub ref_group: String,

    #[arg(long, default_value_t = 12346, help = "TCP port for order gateway")]
    pub order_port: u16,

    #[arg(long, default_value_t = 10_000, help = "Exchange ticks per second")]
    pub tick_rate: u64,

    #[arg(long, default_value_t = 0, help = "Total ticks to run (0 = unlimited)")]
    pub ticks: u64,

    #[arg(
        long,
        default_value_t = false,
        help = "Debug: log when one exchange loop exceeds the configured tick interval"
    )]
    pub debug_tick_overrun: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Debug: log every stale quote opportunity and whether it is filled or expired"
    )]
    pub debug_stale_quotes: bool,
}
