mod api;
mod config;
mod display;
mod models;
mod parser;

use anyhow::Result;
use config::{resolve_max_cycles, resolve_tickers, REFRESH_INTERVAL_SECS};

#[tokio::main]
async fn main() -> Result<()> {
    let tickers = resolve_tickers();
    let max_cycles = resolve_max_cycles();
    display::tui::run(tickers, max_cycles, REFRESH_INTERVAL_SECS).await
}
