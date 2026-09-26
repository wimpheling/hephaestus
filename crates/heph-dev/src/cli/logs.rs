use clap::{Args, ValueEnum};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LogComponent {
    Web,
    Daemon,
    Oidc,
    Zot,
}

#[derive(Debug, Args)]
pub struct LogArgs {
    /// Component to read; omit to show the end of every component log.
    #[arg(value_enum)]
    pub component: Option<LogComponent>,
    /// Continue following the selected log.
    #[arg(short, long, requires = "component")]
    pub follow: bool,
}
