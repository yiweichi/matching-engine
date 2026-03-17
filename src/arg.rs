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
