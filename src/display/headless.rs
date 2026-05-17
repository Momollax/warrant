use std::time::Duration;

use anyhow::Result;

use crate::api::client::build_client;
use crate::api::fetch_all::fetch_all;
use crate::models::warrant::WarrantSnapshot;

/// Runs without a terminal UI, suitable for Docker detached mode and log streaming.
pub async fn run(tickers: Vec<String>, max_cycles: u32, refresh_secs: u64) -> Result<()> {
    let client = build_client()?;
    let mut cycle = 0_u32;

    loop {
        cycle += 1;
        println!(
            "[INFO] Cycle #{cycle}: fetching {} ticker(s)",
            tickers.len()
        );

        let results = fetch_all(&client, &tickers).await;
        let mut errors = 0_usize;

        for result in results {
            match result {
                Ok(snapshot) => print_snapshot(&snapshot),
                Err(err) => {
                    errors += 1;
                    eprintln!("[ERR] {err:#}");
                }
            }
        }

        if errors == 0 {
            println!("[OK] Cycle #{cycle}: completed");
        } else {
            eprintln!("[WARN] Cycle #{cycle}: completed with {errors} error(s)");
        }

        if max_cycles > 0 && cycle >= max_cycles {
            break;
        }

        tokio::time::sleep(Duration::from_secs(refresh_secs)).await;
    }

    Ok(())
}

fn print_snapshot(snapshot: &WarrantSnapshot) {
    let candle = snapshot
        .last_candle
        .as_ref()
        .map(|c| {
            format!(
                " candle[o={:.4}, h={:.4}, l={:.4}, c={:.4}, v={}]",
                c.open, c.high, c.low, c.close, c.volume
            )
        })
        .unwrap_or_default();

    println!(
        "[DATA] {} | {} | {:.4} {} | prev={:.4} | change={:+.2}% | vol={} | 52w=[{:.4}, {:.4}]{}",
        snapshot.ticker,
        snapshot.name,
        snapshot.price,
        snapshot.currency,
        snapshot.prev_close,
        snapshot.change_pct,
        snapshot.volume,
        snapshot.low_52w,
        snapshot.high_52w,
        candle
    );
}
