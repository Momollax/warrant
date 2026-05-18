mod analysis;
mod api;
mod config;
#[allow(dead_code)]
mod decision;
mod display;
mod indicators;
mod llm;
mod models;
mod parser;
mod pricing;
mod trace;

use std::collections::HashSet;
use std::io::{self, IsTerminal};

use anyhow::Result;
use analysis::build_market_base;
use api::boursorama::fetch_boursorama_quote_for_product;
use api::boursorama_discover::discover_boursorama_by_underlying;
use api::candle_cache::{candle_cache_path, load_or_fetch_candles, CacheMode};
use api::client::build_client;
use api::discover::discover_by_underlying;
use api::orats::OratsClient;
use api::polygon::PolygonClient;
use config::{resolve_max_cycles, resolve_tickers, REFRESH_INTERVAL_SECS};
use chrono::{NaiveDate, Utc};
use decision::config::DecisionConfig;
use decision::market_indicators::compute_market_indicators;
use decision::models::{DecisionAction, DecisionSignal, MarketContext};
use decision::scenario::{
    analyze_warrant_scenario, default_max_maturity_for_early_next_year, ScenarioCandidate,
    ScenarioConfig,
};
use decision::trade_plan::build_trade_plan;
use futures::{stream, StreamExt};
use indicators::warrant::{
    rank_relative_value, scenario_warrant_signals, OpportunitySignal, ValuationSide,
};
use models::euronext::EuronextProduct;
use models::structured::StructuredProduct;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "discover") {
        let underlying = args.get(2).map(String::as_str).unwrap_or("hermes");
        let limit = args.get(3).and_then(|value| value.parse().ok());
        return run_discovery(underlying, limit).await;
    }
    if args.get(1).is_some_and(|arg| arg == "analyze") {
        let underlying = args.get(2).map(String::as_str).unwrap_or("hermes");
        let underlying_ticker = args
            .get(3)
            .map(String::as_str)
            .unwrap_or_else(|| default_underlying_ticker(underlying));
        let limit = args.get(4).and_then(|value| value.parse().ok());
        return run_analysis(underlying, underlying_ticker, limit).await;
    }
    if args.get(1).is_some_and(|arg| arg == "opportunities") {
        let underlying = args.get(2).map(String::as_str).unwrap_or("hermes");
        let underlying_ticker = args
            .get(3)
            .map(String::as_str)
            .unwrap_or_else(|| default_underlying_ticker(underlying));
        let limit = args.get(4).and_then(|value| value.parse().ok());
        let side_filter = args.get(5).map(String::as_str).unwrap_or("all");
        return run_opportunities(underlying, underlying_ticker, limit, side_filter).await;
    }
    if args.get(1).is_some_and(|arg| arg == "scenario") {
        let underlying = args.get(2).map(String::as_str).unwrap_or("hermes");
        let underlying_ticker = args
            .get(3)
            .map(String::as_str)
            .unwrap_or_else(|| default_underlying_ticker(underlying));
        let target_price = args
            .get(4)
            .and_then(|value| value.parse().ok())
            .or_else(|| read_env_f64("SCENARIO_TARGET_PRICE"))
            .unwrap_or(1800.0);
        let target_date = args
            .get(5)
            .map(String::as_str)
            .and_then(parse_date_arg)
            .or_else(|| read_env_date("SCENARIO_TARGET_DATE"))
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
        let min_maturity = args
            .get(6)
            .map(String::as_str)
            .and_then(parse_date_arg);
        let min_maturity = min_maturity.or_else(|| read_env_date("SCENARIO_MIN_MATURITY"));
        let max_maturity = args
            .get(7)
            .map(String::as_str)
            .and_then(parse_date_arg);
        let max_maturity = max_maturity.or_else(|| read_env_date("SCENARIO_MAX_MATURITY"));
        let side_arg = args.get(8).cloned();
        let side_arg_is_limit = side_arg
            .as_deref()
            .is_some_and(|value| value.parse::<usize>().is_ok());
        let side = if side_arg_is_limit {
            std::env::var("SCENARIO_SIDE").unwrap_or_else(|_| "auto".to_string())
        } else {
            side_arg
                .clone()
                .or_else(|| std::env::var("SCENARIO_SIDE").ok())
                .unwrap_or_else(|| "auto".to_string())
        };
        let limit = if side_arg_is_limit {
            side_arg.as_deref().and_then(|value| value.parse().ok())
        } else {
            args.get(9).and_then(|value| value.parse().ok())
        };
        return run_scenario(
            underlying,
            underlying_ticker,
            target_price,
            target_date,
            min_maturity,
            max_maturity,
            &side,
            limit,
        )
        .await;
    }
    if args.get(1).is_some_and(|arg| arg == "orats-test") {
        let ticker = args.get(2).map(String::as_str).unwrap_or("RMS.PA");
        return run_orats_test(ticker).await;
    }
    if args.get(1).is_some_and(|arg| arg == "polygon-test") {
        let ticker = args.get(2).map(String::as_str).unwrap_or("RMS.PA");
        return run_polygon_test(ticker).await;
    }
    if args.get(1).is_some_and(|arg| arg == "candles") {
        let ticker = args.get(2).map(String::as_str).unwrap_or("RMS.PA");
        let range = args.get(3).map(String::as_str).unwrap_or("6mo");
        let interval = args.get(4).map(String::as_str).unwrap_or("1d");
        let mode = args.get(5).map(String::as_str).unwrap_or("cache");
        return run_candles(ticker, range, interval, mode).await;
    }

    if let Ok(underlying) = std::env::var("DISCOVER_UNDERLYING") {
        let limit = std::env::var("DISCOVER_LIMIT")
            .ok()
            .and_then(|value| value.parse().ok());
        return run_discovery(&underlying, limit).await;
    }
    if let Ok(underlying) = std::env::var("ANALYZE_UNDERLYING") {
        let underlying_ticker =
            std::env::var("UNDERLYING_TICKER").unwrap_or_else(|_| default_underlying_ticker(&underlying).to_string());
        let limit = std::env::var("ANALYZE_LIMIT")
            .ok()
            .and_then(|value| value.parse().ok());
        return run_analysis(&underlying, &underlying_ticker, limit).await;
    }
    if let Ok(underlying) = std::env::var("OPPORTUNITY_UNDERLYING") {
        let underlying_ticker =
            std::env::var("UNDERLYING_TICKER").unwrap_or_else(|_| default_underlying_ticker(&underlying).to_string());
        let limit = std::env::var("OPPORTUNITY_LIMIT")
            .ok()
            .and_then(|value| value.parse().ok());
        let side_filter = std::env::var("OPPORTUNITY_SIDE").unwrap_or_else(|_| "all".to_string());
        return run_opportunities(&underlying, &underlying_ticker, limit, &side_filter).await;
    }
    if let Ok(underlying) = std::env::var("SCENARIO_UNDERLYING") {
        let underlying_ticker =
            std::env::var("UNDERLYING_TICKER").unwrap_or_else(|_| default_underlying_ticker(&underlying).to_string());
        let target_price = read_env_f64("SCENARIO_TARGET_PRICE").unwrap_or(1800.0);
        let target_date = read_env_date("SCENARIO_TARGET_DATE")
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 10, 31).unwrap());
        let min_maturity = read_env_date("SCENARIO_MIN_MATURITY");
        let max_maturity = read_env_date("SCENARIO_MAX_MATURITY");
        let side = std::env::var("SCENARIO_SIDE").unwrap_or_else(|_| "auto".to_string());
        let limit = std::env::var("SCENARIO_LIMIT")
            .ok()
            .and_then(|value| value.parse().ok());
        return run_scenario(
            &underlying,
            &underlying_ticker,
            target_price,
            target_date,
            min_maturity,
            max_maturity,
            &side,
            limit,
        )
        .await;
    }
    if let Ok(ticker) = std::env::var("ORATS_TEST_TICKER") {
        return run_orats_test(&ticker).await;
    }
    if let Ok(ticker) = std::env::var("POLYGON_TEST_TICKER") {
        return run_polygon_test(&ticker).await;
    }
    if let Ok(ticker) = std::env::var("CANDLES_TICKER") {
        let range = std::env::var("CANDLES_RANGE").unwrap_or_else(|_| "6mo".to_string());
        let interval = std::env::var("CANDLES_INTERVAL").unwrap_or_else(|_| "1d".to_string());
        let mode = std::env::var("CANDLES_MODE")
            .or_else(|_| std::env::var("MARKET_DATA_REFRESH"))
            .unwrap_or_else(|_| "cache".to_string());
        return run_candles(&ticker, &range, &interval, &mode).await;
    }

    let tickers = resolve_tickers();
    let max_cycles = resolve_max_cycles();
    let display_mode = std::env::var("DISPLAY_MODE").unwrap_or_default();

    match display_mode.as_str() {
        "headless" | "logs" => {
            display::headless::run(tickers, max_cycles, REFRESH_INTERVAL_SECS).await
        }
        "tui" => display::tui::run(tickers, max_cycles, REFRESH_INTERVAL_SECS).await,
        _ if io::stdout().is_terminal() => {
            display::tui::run(tickers, max_cycles, REFRESH_INTERVAL_SECS).await
        }
        _ => display::headless::run(tickers, max_cycles, REFRESH_INTERVAL_SECS).await,
    }
}

async fn run_orats_test(ticker: &str) -> Result<()> {
    let client = build_client()?;
    let orats = OratsClient::from_env();
    eprintln!(
        "[INFO] Test ORATS pour {ticker} avec {} cle(s) configuree(s)",
        orats.key_count()
    );
    let payload = orats.delayed_cores(&client, ticker).await?;
    let row_count = payload
        .get("data")
        .and_then(|data| data.as_array())
        .map(|rows| rows.len())
        .unwrap_or(0);
    println!("orats_ticker,row_count");
    println!("{ticker},{row_count}");
    Ok(())
}

async fn run_candles(ticker: &str, range: &str, interval: &str, mode: &str) -> Result<()> {
    let client = build_client()?;
    let cache_dir = std::env::var("MARKET_DATA_CACHE_DIR")
        .unwrap_or_else(|_| "data/cache/candles".to_string());
    let cache_mode = CacheMode::from_str(mode);
    let cache_path = candle_cache_path(std::path::Path::new(&cache_dir), ticker, range, interval);
    let data_source = if cache_mode == CacheMode::UseCache && cache_path.exists() {
        "cache"
    } else {
        "network"
    };
    let series = load_or_fetch_candles(
        &client,
        ticker,
        range,
        interval,
        std::path::Path::new(&cache_dir),
        cache_mode,
    )
    .await?;

    eprintln!(
        "[INFO] Bougies {} {} {}: {} ligne(s), data {}, source {}, cache {}",
        series.ticker,
        series.range,
        series.interval,
        series.candles.len(),
        data_source,
        series.source,
        cache_dir
    );
    println!("ticker,currency,range,interval,timestamp,open,high,low,close,volume");
    for candle in &series.candles {
        println!(
            "{},{},{},{},{},{:.6},{:.6},{:.6},{:.6},{}",
            csv_escape(&series.ticker),
            csv_escape(&series.currency),
            csv_escape(&series.range),
            csv_escape(&series.interval),
            candle.timestamp,
            candle.open,
            candle.high,
            candle.low,
            candle.close,
            candle.volume
        );
    }

    Ok(())
}

async fn run_polygon_test(ticker: &str) -> Result<()> {
    let client = build_client()?;
    let polygon = PolygonClient::from_env();
    eprintln!(
        "[INFO] Test Polygon pour {ticker} avec {} cle(s) configuree(s)",
        polygon.key_count()
    );
    let payload = polygon.previous_aggregate(&client, ticker).await?;
    let row_count = payload
        .get("results")
        .and_then(|results| results.as_array())
        .map(|rows| rows.len())
        .unwrap_or(0);
    println!("polygon_ticker,row_count");
    println!("{ticker},{row_count}");
    Ok(())
}

async fn run_opportunities(
    underlying: &str,
    underlying_ticker: &str,
    limit: Option<usize>,
    side_filter: &str,
) -> Result<()> {
    trace::log(format!(
        "opportunities: start underlying={underlying} spot={underlying_ticker} limit={limit:?} side={side_filter}"
    ));
    let client = build_client()?;
    trace::log("opportunities: HTTP client ready");
    let mut base = build_market_base(&client, underlying, underlying_ticker, limit).await?;
    trace::log(format!(
        "opportunities: market base ready products={} fx_rates_ready",
        base.products.len()
    ));
    trace::log("opportunities: ranking relative value");
    let mut signals = rank_relative_value(
        &base.products,
        base.underlying_quote.price,
        &base.underlying_quote.currency,
        &base.fx_rates,
    );
    trace::log(format!("opportunities: ranking done signals={}", signals.len()));
    if signals.is_empty() && opportunity_validation_enabled() {
        trace::log("opportunities: empty ranking, validating products before retry");
        let validated_count = validate_unquoted_products(&client, &mut base.products).await;
        trace::log(format!(
            "opportunities: empty-ranking validation done validated={validated_count}"
        ));
        if validated_count > 0 {
            trace::log("opportunities: reranking after empty-ranking validation");
            signals = rank_relative_value(
                &base.products,
                base.underlying_quote.price,
                &base.underlying_quote.currency,
                &base.fx_rates,
            );
            trace::log(format!(
                "opportunities: empty-ranking rerank done signals={}",
                signals.len()
            ));
            eprintln!(
                "[INFO] {} produit(s) valide(s) via Boursorama avant ranking",
                validated_count
            );
        }
    }
    trace::log("opportunities: Boursorama candidate validation start");
    let validated_count =
        validate_opportunity_candidates(&client, &mut base.products, &signals).await;
    trace::log(format!(
        "opportunities: Boursorama candidate validation done validated={validated_count}"
    ));
    if validated_count > 0 {
        trace::log("opportunities: reranking after Boursorama validation");
        signals = rank_relative_value(
            &base.products,
            base.underlying_quote.price,
            &base.underlying_quote.currency,
            &base.fx_rates,
        );
        trace::log(format!(
            "opportunities: reranking done signals={}",
            signals.len()
        ));
        eprintln!(
            "[INFO] {} candidat(s) valide(s) via Boursorama, ranking recalcule",
            validated_count
        );
    }
    if opportunity_require_validated_price() {
        let removed_count = remove_unverified_price_signals(&mut signals);
        if removed_count > 0 {
            eprintln!(
                "[INFO] {} signal(aux) retire(s): prix non executable/non valide via Boursorama",
                removed_count
            );
        }
    }
    let min_gap_pct = opportunity_min_gap_pct();
    let removed_small_gap_count = remove_small_gap_signals(&mut signals, min_gap_pct);
    if removed_small_gap_count > 0 {
        eprintln!(
            "[INFO] {} signal(aux) retire(s): edge net inferieur a {:.2}%",
            removed_small_gap_count, min_gap_pct
        );
    }
    let format = std::env::var("OPPORTUNITY_FORMAT").unwrap_or_default();
    let market_context = build_decision_market_context(&client, &base).await;
    let decision_config = DecisionConfig::from_env();
    let mut decision_signals = build_decision_signals(&signals, &market_context, &decision_config);
    sort_decision_signals(&mut decision_signals);

    let filtered_signals = filter_decision_signals(&decision_signals, side_filter);
    trace::log(format!(
        "opportunities: display format={} filtered={} total={}",
        if format.is_empty() { "auto" } else { &format },
        filtered_signals.len(),
        decision_signals.len()
    ));

    if format == "tui" && io::stdout().is_terminal() {
        return display::opportunities::run(
            &base.underlying_quote,
            base.products.len(),
            &decision_signals,
            side_filter,
        );
    }

    print_opportunity_summary(
        &base.underlying_quote,
        base.products.len(),
        decision_signals.len(),
        filtered_signals.len(),
        side_filter,
    );

    if format == "csv" {
        print_opportunity_csv(&base.underlying_quote, &filtered_signals);
    } else {
        print_opportunity_table(&filtered_signals);
    }

    Ok(())
}

async fn run_scenario(
    underlying: &str,
    underlying_ticker: &str,
    target_price: f64,
    target_date: NaiveDate,
    min_maturity: Option<NaiveDate>,
    max_maturity: Option<NaiveDate>,
    side: &str,
    limit: Option<usize>,
) -> Result<()> {
    let client = build_client()?;
    let mut base = build_market_base(&client, underlying, underlying_ticker, limit).await?;
    let mut signals = scenario_warrant_signals(
        &base.products,
        base.underlying_quote.price,
        &base.underlying_quote.currency,
        &base.fx_rates,
    );
    trace::log(format!(
        "scenario: initial signals={} call={} put={} products={}",
        signals.len(),
        signals.iter().filter(|signal| signal.side == "call").count(),
        signals.iter().filter(|signal| signal.side == "put").count(),
        base.products.len()
    ));

    let validated_count =
        validate_opportunity_candidates(&client, &mut base.products, &signals).await;
    if validated_count > 0 {
        signals = scenario_warrant_signals(
            &base.products,
            base.underlying_quote.price,
            &base.underlying_quote.currency,
            &base.fx_rates,
        );
        trace::log(format!(
            "scenario: signals after validation={} call={} put={}",
            signals.len(),
            signals.iter().filter(|signal| signal.side == "call").count(),
            signals.iter().filter(|signal| signal.side == "put").count()
        ));
        eprintln!(
            "[INFO] {} candidat(s) valide(s) via Boursorama, scenario recalcule",
            validated_count
        );
    }
    if opportunity_require_validated_price() {
        let unverified_count = signals
            .iter()
            .filter(|signal| signal.price_source == "last_unverified")
            .count();
        if unverified_count > 0 {
            eprintln!(
                "[INFO] {} produit(s) scenario gardes avec prix non executable: ils seront classes AVOID tant que le bid/ask n'est pas valide",
                unverified_count
            );
        }
    }

    let decision_config = DecisionConfig::from_env();
    let market_context = build_decision_market_context(&client, &base).await;
    let scenario_side = scenario_side(side, target_price, base.underlying_quote.price);
    let scenario_config = ScenarioConfig {
        target_price,
        target_date,
        min_maturity,
        max_maturity: max_maturity
            .or_else(|| default_max_maturity_for_early_next_year(target_date)),
        side: scenario_side.to_string(),
        risk_free_rate: market_context.risk_free_rate,
        dividend_yield: market_context.dividend_yield,
        fallback_volatility: market_context.realized_volatility_20d.unwrap_or(0.35),
    };
    let today = Utc::now().date_naive();
    let candidates = analyze_warrant_scenario(
        &signals,
        base.underlying_quote.price,
        &scenario_config,
        &decision_config,
        today,
    );
    trace::log(format!(
        "scenario: analyzed candidates={} side={} target={}",
        candidates.len(),
        scenario_config.side,
        scenario_config.target_price
    ));

    let format = std::env::var("SCENARIO_FORMAT").unwrap_or_else(|_| {
        if io::stdout().is_terminal() {
            "tui".to_string()
        } else {
            "table".to_string()
        }
    });
    if format == "tui" && io::stdout().is_terminal() {
        return display::scenario::run(
            &base.underlying_quote,
            base.products.len(),
            &signals,
            &decision_config,
            &scenario_config,
            side,
            today,
        );
    }

    print_scenario_summary(&base.underlying_quote, &scenario_config, candidates.len());
    print_scenario_table(&candidates, 20);
    Ok(())
}

async fn build_decision_market_context(
    client: &reqwest::Client,
    base: &analysis::MarketBase,
) -> MarketContext {
    let cache_dir = std::env::var("MARKET_DATA_CACHE_DIR")
        .unwrap_or_else(|_| "data/cache/candles".to_string());
    let range = std::env::var("CANDLES_RANGE").unwrap_or_else(|_| "6mo".to_string());
    let interval = std::env::var("CANDLES_INTERVAL").unwrap_or_else(|_| "1d".to_string());
    let mode = std::env::var("MARKET_DATA_REFRESH")
        .or_else(|_| std::env::var("CANDLES_MODE"))
        .unwrap_or_else(|_| "cache".to_string());
    let cache_mode = CacheMode::from_str(&mode);
    let indicators = match load_or_fetch_candles(
        client,
        &base.underlying_quote.ticker,
        &range,
        &interval,
        std::path::Path::new(&cache_dir),
        cache_mode,
    )
    .await
    {
        Ok(series) => {
            trace::log(format!(
                "market indicators: candles loaded ticker={} rows={} range={} interval={}",
                series.ticker,
                series.candles.len(),
                series.range,
                series.interval
            ));
            compute_market_indicators(&series)
        }
        Err(err) => {
            eprintln!(
                "[WARN] Bougies indisponibles pour {}: {err:#}. Stops/targets utilisent les fallbacks.",
                base.underlying_quote.ticker
            );
            Default::default()
        }
    };

    MarketContext {
        underlying_ticker: base.underlying_quote.ticker.clone(),
        spot: base.underlying_quote.price,
        spot_currency: base.underlying_quote.currency.clone(),
        change_pct: base.underlying_quote.change_pct,
        fx_rates: base.fx_rates.clone(),
        realized_volatility_20d: indicators.realized_volatility_20d,
        atr_14d: indicators.atr_14d,
        support_1: indicators.support_1,
        support_2: indicators.support_2,
        resistance_1: indicators.resistance_1,
        resistance_2: indicators.resistance_2,
        risk_free_rate: read_env_f64("OPTION_RISK_FREE_RATE").unwrap_or(0.045),
        dividend_yield: read_env_f64("OPTION_DIVIDEND_YIELD").unwrap_or(0.005),
    }
}

fn build_decision_signals(
    signals: &[OpportunitySignal],
    market: &MarketContext,
    config: &DecisionConfig,
) -> Vec<DecisionSignal> {
    signals
        .iter()
        .map(|signal| build_trade_plan(signal, market, config))
        .collect()
}

fn sort_decision_signals(signals: &mut [DecisionSignal]) {
    signals.sort_by(|left, right| {
        decision_rank(left.decision)
            .cmp(&decision_rank(right.decision))
            .then_with(|| {
                confidence(right)
                    .partial_cmp(&confidence(left))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                right
                    .opportunity
                    .score
                    .partial_cmp(&left.opportunity.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| left.opportunity.symbol.cmp(&right.opportunity.symbol))
    });
}

fn decision_rank(decision: DecisionAction) -> u8 {
    match decision {
        DecisionAction::BuyCandidate => 0,
        DecisionAction::Watch | DecisionAction::Hold => 1,
        DecisionAction::TakeProfit => 2,
        DecisionAction::ExitLoss => 3,
        DecisionAction::Avoid => 4,
    }
}

fn confidence(signal: &DecisionSignal) -> f64 {
    signal
        .trade_plan
        .as_ref()
        .map(|plan| plan.confidence_score)
        .unwrap_or(0.0)
}

async fn validate_opportunity_candidates(
    client: &reqwest::Client,
    products: &mut [StructuredProduct],
    signals: &[OpportunitySignal],
) -> usize {
    if !opportunity_validation_enabled() {
        trace::log("validation: disabled by OPPORTUNITY_VALIDATE_BOURSORAMA");
        return 0;
    }

    let limit = opportunity_validation_limit();
    if limit == 0 {
        trace::log("validation: skipped because OPPORTUNITY_VALIDATE_LIMIT=0");
        return 0;
    }

    let mut seen = HashSet::new();
    let symbols = signals
        .iter()
        .filter(|signal| signal.price_source == "last_unverified")
        .filter_map(|signal| {
            if seen.insert(signal.symbol.clone()) {
                Some(signal.symbol.clone())
            } else {
                None
            }
        })
        .take(limit)
        .collect::<Vec<_>>();

    trace::log(format!(
        "validation: {} candidate(s) selected with limit={limit}",
        symbols.len()
    ));
    let mut validated_count = 0usize;
    let total = symbols.len();
    for (index, symbol) in symbols.into_iter().enumerate() {
        trace::log_progress(index + 1, total, "validation Boursorama", &symbol);
        let Some(product) = products.iter_mut().find(|product| product.symbol == symbol) else {
            continue;
        };
        if product.boursorama_quote.is_some() {
            continue;
        }
        match fetch_boursorama_quote_for_product(client, product).await {
            Ok(quote) => {
                product.set_boursorama_quote(quote);
                validated_count += 1;
            }
            Err(err) => {
                trace::log(format!(
                    "validation Boursorama: {} failed: {err:#}",
                    product.symbol
                ));
            }
        }
    }

    validated_count
}

async fn validate_unquoted_products(
    client: &reqwest::Client,
    products: &mut [StructuredProduct],
) -> usize {
    let limit = opportunity_validation_limit();
    if limit == 0 {
        trace::log("validation fallback: skipped because OPPORTUNITY_VALIDATE_LIMIT=0");
        return 0;
    }

    let targets = products
        .iter()
        .filter(|product| product.boursorama_quote.is_none())
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let total = targets.len();
    if total == 0 {
        return 0;
    }

    let concurrency = opportunity_validation_concurrency();
    let results = stream::iter(targets.into_iter().enumerate().map(|(index, product)| {
        let client = client.clone();
        async move {
            trace::log_progress(
                index + 1,
                total,
                "validation fallback Boursorama",
                &product.symbol,
            );
            let symbol = product.symbol.clone();
            match fetch_boursorama_quote_for_product(&client, &product).await {
                Ok(quote) => Some((symbol, quote)),
                Err(err) => {
                    trace::log(format!(
                        "validation fallback Boursorama: {} failed: {err:#}",
                        product.symbol
                    ));
                    None
                }
            }
        }
    }))
    .buffer_unordered(concurrency)
    .collect::<Vec<_>>()
    .await;

    let mut validated_count = 0usize;
    for (symbol, quote) in results.into_iter().flatten() {
        if let Some(product) = products.iter_mut().find(|product| product.symbol == symbol) {
            if product.boursorama_quote.is_none() {
                product.set_boursorama_quote(quote);
                validated_count += 1;
            }
        }
    }

    validated_count
}

fn opportunity_validation_enabled() -> bool {
    std::env::var("OPPORTUNITY_VALIDATE_BOURSORAMA")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

fn opportunity_validation_limit() -> usize {
    std::env::var("OPPORTUNITY_VALIDATE_LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(500usize)
}

fn opportunity_validation_concurrency() -> usize {
    std::env::var("OPPORTUNITY_VALIDATE_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(8usize)
}

fn opportunity_require_validated_price() -> bool {
    std::env::var("OPPORTUNITY_REQUIRE_VALIDATED_PRICE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(true)
}

fn opportunity_min_gap_pct() -> f64 {
    std::env::var("OPPORTUNITY_MIN_GAP_PCT")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(1.0)
}

fn remove_unverified_price_signals(signals: &mut Vec<OpportunitySignal>) -> usize {
    let before = signals.len();
    signals.retain(|signal| signal.price_source != "last_unverified");
    before - signals.len()
}

fn remove_small_gap_signals(signals: &mut Vec<OpportunitySignal>, min_gap_pct: f64) -> usize {
    if min_gap_pct <= 0.0 {
        return 0;
    }

    let before = signals.len();
    signals.retain(|signal| match signal.valuation {
        ValuationSide::Undervalued => signal.spread_adjusted_gap_pct >= min_gap_pct,
        ValuationSide::Overvalued => signal.spread_adjusted_gap_pct <= -min_gap_pct,
    });
    before - signals.len()
}

fn print_opportunity_summary(
    underlying: &models::warrant::WarrantSnapshot,
    product_count: usize,
    total_signal_count: usize,
    visible_signal_count: usize,
    side_filter: &str,
) {
    eprintln!(
        "[INFO] Spot {}: {:.4} {} ({:+.2}%)",
        underlying.ticker, underlying.price, underlying.currency, underlying.change_pct
    );
    eprintln!(
        "[INFO] {} signal(aux) visible(s) / {} total sur {} produit(s), filtre={}",
        visible_signal_count, total_signal_count, product_count, normalize_side_filter(side_filter)
    );
    eprintln!(
        "[WARN] Ces signaux ne sont pas des arbitrages garantis: FX, frais, bid/ask executables, market maker et statut temps reel restent a verifier."
    );
}

fn print_opportunity_table(signals: &[&DecisionSignal]) {
    print_rank_section(
        "Top 10 plans sous-evalues",
        signals,
        ValuationSide::Undervalued,
        10,
    );
    println!();
    print_rank_section(
        "Top 10 plans sur-evalues",
        signals,
        ValuationSide::Overvalued,
        10,
    );
}

fn print_rank_section(
    title: &str,
    signals: &[&DecisionSignal],
    valuation: ValuationSide,
    limit: usize,
) {
    println!("{title}");
    println!(
        "{:<6} {:>5} {:>7} {:>6} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>6} {:>5} {:>5} {:<8} {:<5} {:<22} {:<10} {:>10}",
        "dec", "conf", "edge", "rr2", "size%", "stop%", "t1%", "t2%", "be%", "fee%", "spr%", "dq", "flow", "symbol", "side", "type", "maturity", "price"
    );
    println!("{}", "-".repeat(150));

    let mut printed = 0usize;
    for decision in signals
        .iter()
        .copied()
        .filter(|decision| decision.opportunity.valuation == valuation)
        .take(limit)
    {
        let signal = &decision.opportunity;
        let plan = decision.trade_plan.as_ref();
        printed += 1;
        println!(
            "{:<6} {:>5} {:>+6.1}% {:>6} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>6} {:>5.0} {:>5.0} {:<8} {:<5} {:<22} {:<10} {:>4} {:>5.4}",
            decision_action_label(decision.decision),
            plan.map(|plan| format!("{:.0}", plan.confidence_score)).unwrap_or_else(|| "-".to_string()),
            signal.spread_adjusted_gap_pct,
            plan.and_then(|plan| plan.reward_risk_2).map(|value| format!("{value:.2}")).unwrap_or_else(|| "-".to_string()),
            plan.map(|plan| format!("{:.1}", plan.position.suggested_notional_pct)).unwrap_or_else(|| "-".to_string()),
            plan.map(|plan| format!("{:.1}", plan.stop.loss_pct)).unwrap_or_else(|| "-".to_string()),
            plan.map(|plan| format!("{:+.1}", plan.target_1.gain_pct)).unwrap_or_else(|| "-".to_string()),
            plan.map(|plan| format!("{:+.1}", plan.target_2.gain_pct)).unwrap_or_else(|| "-".to_string()),
            plan.and_then(|plan| plan.breakeven_move_pct).map(|value| format!("{value:.2}")).unwrap_or_else(|| "-".to_string()),
            plan.map(|plan| format!("{:.2}", plan.fees.roundtrip_target_2_fee_pct)).unwrap_or_else(|| "-".to_string()),
            signal.spread_pct.map(|value| format!("{value:.2}")).unwrap_or_else(|| "-".to_string()),
            signal.data_quality_score,
            signal.liquidity_score,
            signal.symbol,
            signal.side,
            truncate_display(&signal.product_type, 22),
            signal.maturity,
            signal.price_currency,
            signal.last_price
        );
    }

    if printed == 0 {
        println!("Aucun signal dans cette categorie avec le filtre actuel.");
    }
}

fn print_scenario_summary(
    underlying: &models::warrant::WarrantSnapshot,
    config: &ScenarioConfig,
    candidate_count: usize,
) {
    eprintln!(
        "[INFO] Scenario {} {:.4} {} -> target {:.4} au {} | side {} | maturite {}..{} | candidats {}",
        underlying.ticker,
        underlying.price,
        underlying.currency,
        config.target_price,
        config.target_date,
        config.side,
        config
            .min_maturity
            .map(|date| date.to_string())
            .unwrap_or_else(|| "-".to_string()),
        config
            .max_maturity
            .map(|date| date.to_string())
            .unwrap_or_else(|| "-".to_string()),
        candidate_count
    );
}

fn print_scenario_table(candidates: &[ScenarioCandidate], limit: usize) {
    println!(
        "{:<6} {:>5} {:<10} {:<4} {:>8} {:<10} {:>8} {:>8} {:>8} {:>7} {:>7} {:>7} {:>7} {:>8} {:>6} {:>5} {:>5} {:<}",
        "dec", "score", "symbol", "side", "strike", "maturity", "entry", "targetPx", "net%", "pBE%", "pTgt%", "EV%", "Kelly%", "Delta", "iv%", "dq", "flow", "url"
    );
    println!("{}", "-".repeat(190));
    for candidate in candidates.iter().take(limit) {
        println!(
            "{:<6} {:>5.0} {:<10} {:<4} {:>8.2} {:<10} {:>8.4} {:>8.4} {:>+8.1} {:>7} {:>7} {:>7} {:>7} {:>8} {:>6} {:>5.0} {:>5.0} {}",
            candidate.decision,
            candidate.score,
            truncate_display(&candidate.symbol, 10),
            candidate.side,
            candidate.strike,
            candidate.maturity,
            candidate.entry_price,
            candidate.projected_price,
            candidate.net_return_pct,
            candidate.probability_breakeven_pct.map(|value| format!("{value:.1}")).unwrap_or_else(|| "-".to_string()),
            candidate.probability_target_pct.map(|value| format!("{value:.1}")).unwrap_or_else(|| "-".to_string()),
            candidate.expected_value_pct.map(|value| format!("{value:+.1}")).unwrap_or_else(|| "-".to_string()),
            candidate.kelly_fraction_pct.map(|value| format!("{value:.1}")).unwrap_or_else(|| "-".to_string()),
            candidate.delta.map(|value| format!("{value:.4}")).unwrap_or_else(|| "-".to_string()),
            candidate
                .implied_volatility
                .map(|value| format!("{:.1}", value * 100.0))
                .unwrap_or_else(|| "-".to_string()),
            candidate.data_quality_score,
            candidate.liquidity_score,
            candidate.url
        );
        println!(
            "       type={} parity={} vol_used={:.1}% spread={} BE={} BE_dist={} zTgt={} zBE={} Sharpe={} theta/d={} vega/pt={} rho/1%={} reasons={}",
            truncate_display(&candidate.product_type, 34),
            format_float(candidate.parity),
            candidate.volatility_used * 100.0,
            candidate
                .spread_pct
                .map(|value| format!("{value:.2}%"))
                .unwrap_or_else(|| "-".to_string()),
            candidate
                .breakeven_underlying
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "-".to_string()),
            candidate
                .breakeven_distance_pct
                .map(|value| format!("{value:+.2}%"))
                .unwrap_or_else(|| "-".to_string()),
            candidate.target_zscore.map(|value| format!("{value:.3}")).unwrap_or_else(|| "-".to_string()),
            candidate.breakeven_zscore.map(|value| format!("{value:.3}")).unwrap_or_else(|| "-".to_string()),
            candidate.sharpe_like.map(|value| format!("{value:.3}")).unwrap_or_else(|| "-".to_string()),
            candidate
                .theta_horizon_pct
                .map(|value| format!("{value:.3}%"))
                .unwrap_or_else(|| "-".to_string()),
            candidate.vega_per_vol_point.map(|value| format!("{value:.5}")).unwrap_or_else(|| "-".to_string()),
            candidate.rho_per_rate_point.map(|value| format!("{value:.5}")).unwrap_or_else(|| "-".to_string()),
            if candidate.reasons.is_empty() {
                "-".to_string()
            } else {
                candidate.reasons.join("|")
            }
        );
    }
    if candidates.is_empty() {
        println!("Aucun warrant ne correspond au scenario. Essaie d'elargir la maturite ou d'augmenter la limite.");
    }
}

fn print_opportunity_csv(
    underlying: &models::warrant::WarrantSnapshot,
    signals: &[&DecisionSignal],
) {
    println!(
        "underlying_ticker,underlying_price,decision,decision_reasons,decision_warnings,confidence_score,entry_price,entry_price_source,underlying_entry_price,entry_edge_pct,breakeven_move_pct,underlying_stop_price,product_stop_price,stop_loss_pct,distance_to_stop_pct,stop_reason,target_1_underlying_price,target_1_product_price,target_1_gain_pct,target_1_distance_pct,target_2_underlying_price,target_2_product_price,target_2_gain_pct,target_2_distance_pct,reward_risk_1,reward_risk_2,holding_days,max_holding_days,maturity_days,theta_daily_pct,theta_to_horizon_pct,time_risk_score,barrier_risk_score,spread_cost_underlying_pct,account_risk_pct,suggested_position_notional_pct,max_position_notional_pct,estimated_account_loss_pct,products_per_1000_account,fee_profile,fee_order_notional,fee_buy,fee_deposit,fee_sell_stop,fee_sell_target_1,fee_sell_target_2,fee_roundtrip_stop_pct,fee_roundtrip_target_1_pct,fee_roundtrip_target_2_pct,raw_stop_loss_pct,raw_target_1_gain_pct,raw_target_2_gain_pct,valuation,side,moneyness,symbol,boursorama_url,product_family,pricing_model,product_type,maturity,price_currency,last_price,price_source,bid_price,ask_price,mid_price,spread_pct,bid_size,ask_size,quote_volume,execution_status,data_quality_score,liquidity_score,payoff_reference,payoff_reference_currency,barrier,barrier_currency,barrier_distance_pct,raw_intrinsic,intrinsic_per_product,intrinsic_currency,fx_rate,fx_source_ticker,warrants_per_underlying,price_to_intrinsic,effective_gearing,premium_discount_pct,years_to_maturity,risk_free_rate,dividend_yield,implied_volatility,smile_median_iv,smile_gap_vol_points,volatility_signal,metric_kind,relative_metric,peer_median_relative_metric,peer_gap_pct,spread_adjusted_gap_pct,peer_count,score,note"
    );
    for decision in signals {
        println!(
            "{}",
            decision_signal_to_csv(underlying, decision)
        );
    }
}

fn filter_decision_signals<'a>(
    signals: &'a [DecisionSignal],
    side_filter: &str,
) -> Vec<&'a DecisionSignal> {
    let side_filter = normalize_side_filter(side_filter);
    signals
        .iter()
        .filter(|signal| match side_filter {
            "call" => signal.opportunity.side == "call",
            "put" => signal.opportunity.side == "put",
            _ => true,
        })
        .collect()
}

fn normalize_side_filter(side_filter: &str) -> &'static str {
    match side_filter {
        "call" | "calls" => "call",
        "put" | "puts" => "put",
        _ => "all",
    }
}

async fn run_analysis(
    underlying: &str,
    underlying_ticker: &str,
    limit: Option<usize>,
) -> Result<()> {
    let client = build_client()?;
    let base = build_market_base(&client, underlying, underlying_ticker, limit).await?;

    eprintln!(
        "[INFO] Spot {}: {:.4} {} ({:+.2}%)",
        base.underlying_quote.ticker,
        base.underlying_quote.price,
        base.underlying_quote.currency,
        base.underlying_quote.change_pct
    );
    eprintln!(
        "[INFO] {} produit(s) normalise(s) pour '{}'",
        base.products.len(),
        underlying
    );

    println!(
        "underlying_ticker,underlying_price,underlying_currency,side,moneyness,symbol,boursorama_symbol,boursorama_url,yahoo_symbol,isin,mic,name,product_underlying,product_type,product_family,pricing_model,marketing_product_name,issuer,strike,strike_currency,second_strike,second_strike_currency,distance_to_spot_pct,maturity,price_currency,last_price,bid_ask,last_trade_time,boursorama_bid,boursorama_ask,boursorama_bid_size,boursorama_ask_size,boursorama_mid,boursorama_last,boursorama_previous_close,boursorama_high,boursorama_low,boursorama_volume,boursorama_trade_date,issue_date,issue_price,issue_currency,introduction_date,parity_warrant_underlying,parity_underlying_warrant,parity_first_warrant_underlying,warrants_per_underlying,leverage,lower_threshold,open_price,close_price,previous_close_price,high_price,low_price,valorization,volume,trade_count,traded_amount,quotation_state,trading_status,last_update,last_quote_datetime,opening_time,closing_time,trading_open_time,trading_close_time,underlying_designation,underlying_group,underlying_isin,kid_url"
    );
    for product in base.products {
        println!("{}", structured_product_to_csv(&base.underlying_quote, &product));
    }

    Ok(())
}

async fn run_discovery(underlying: &str, limit: Option<usize>) -> Result<()> {
    let client = build_client()?;
    let mut products = discover_by_underlying(&client, underlying, limit).await?;
    if products.is_empty() {
        trace::log(format!(
            "discover: Euronext returned 0, trying Boursorama fallback for {underlying}"
        ));
        products = discover_boursorama_by_underlying(&client, underlying, limit).await?;
    }

    eprintln!(
        "[INFO] {} produit(s) trouve(s) pour le sous-jacent '{}'",
        products.len(),
        underlying
    );
    println!(
        "symbol,yahoo_symbol,isin,mic,name,underlying,product_type,strike,maturity,bid_ask,last_price,last_trade_time"
    );
    for product in products {
        println!("{}", product_to_csv(&product));
    }

    Ok(())
}

fn product_to_csv(product: &EuronextProduct) -> String {
    [
        product.symbol.as_str(),
        product.yahoo_symbol.as_deref().unwrap_or_default(),
        product.isin.as_deref().unwrap_or_default(),
        product.mic.as_deref().unwrap_or_default(),
        product.name.as_deref().unwrap_or_default(),
        product.underlying.as_str(),
        product.product_type.as_str(),
        product.strike.as_str(),
        product.maturity.as_str(),
        product.bid_ask.as_str(),
        product.last_price.as_str(),
        product.last_trade_time.as_str(),
    ]
    .into_iter()
    .map(csv_escape)
    .collect::<Vec<_>>()
    .join(",")
}

fn structured_product_to_csv(
    underlying: &models::warrant::WarrantSnapshot,
    product: &StructuredProduct,
) -> String {
    let detail = product.detail.as_ref();
    let boursorama = product.boursorama_quote.as_ref();
    let product_family = pricing::classify_product(product).as_str();
    let pricing_model = pricing::pricing_spec(product)
        .map(|spec| spec.model.as_str())
        .unwrap_or("unsupported");
    [
        underlying.ticker.as_str().to_string(),
        format!("{:.4}", underlying.price),
        underlying.currency.as_str().to_string(),
        product.direction.as_str().to_string(),
        product.moneyness.as_str().to_string(),
        product.symbol.as_str().to_string(),
        product.boursorama_symbol.as_str().to_string(),
        product.boursorama_url.as_str().to_string(),
        product.yahoo_symbol.as_deref().unwrap_or_default().to_string(),
        product.isin.as_deref().unwrap_or_default().to_string(),
        product.mic.as_deref().unwrap_or_default().to_string(),
        product.name.as_deref().unwrap_or_default().to_string(),
        product.underlying.as_str().to_string(),
        product.product_type.as_str().to_string(),
        product_family.to_string(),
        pricing_model.to_string(),
        detail
            .and_then(|detail| detail.marketing_product_name.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.issuer_name.clone())
            .unwrap_or_default(),
        product.strike.map(format_float).unwrap_or_default(),
        detail
            .and_then(|detail| detail.strike_currency.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.second_strike_price)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.second_strike_currency.clone())
            .unwrap_or_default(),
        product
            .distance_to_spot_pct
            .map(format_float)
            .unwrap_or_default(),
        product.maturity.as_string(),
        product.last_price.currency.as_deref().unwrap_or_default().to_string(),
        product.last_price.value.map(format_float).unwrap_or_default(),
        product.bid_ask.as_str().to_string(),
        product.last_trade_time.as_str().to_string(),
        boursorama
            .and_then(|quote| quote.bid)
            .map(format_float)
            .unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.ask)
            .map(format_float)
            .unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.bid_size)
            .map(format_float)
            .unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.ask_size)
            .map(format_float)
            .unwrap_or_default(),
        boursorama_mid(boursorama).map(format_float).unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.last)
            .map(format_float)
            .unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.previous_close)
            .map(format_float)
            .unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.high)
            .map(format_float)
            .unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.low)
            .map(format_float)
            .unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.total_volume)
            .map(format_float)
            .unwrap_or_default(),
        boursorama
            .and_then(|quote| quote.trade_date.clone())
            .unwrap_or_default(),
        detail.and_then(|detail| detail.issue_date.clone()).unwrap_or_default(),
        detail
            .and_then(|detail| detail.issue_price)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.issue_price_currency.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.introduction_date.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.parity_warrant_underlying)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.parity_underlying_warrant)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.parity_first_warrant_underlying)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.warrants_per_underlying)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.leverage_level)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.lower_threshold)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.open_price)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.close_price)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.previous_close_price)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.high_price)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.low_price)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.valorization)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.traded_quantity)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.trade_count)
            .map(|value| value.to_string())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.traded_amount)
            .map(format_float)
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.quotation_state.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.trading_status.clone())
            .unwrap_or_default(),
        detail.and_then(|detail| detail.last_update.clone()).unwrap_or_default(),
        detail
            .and_then(|detail| detail.last_quote_datetime.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.opening_time.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.closing_time.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.trading_open_time.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.trading_close_time.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.underlying_designation.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.underlying_group_name.clone())
            .unwrap_or_default(),
        detail
            .and_then(|detail| detail.underlying_isin.clone())
            .unwrap_or_default(),
        detail.and_then(|detail| detail.kid_url.clone()).unwrap_or_default(),
    ]
    .into_iter()
    .map(|value| csv_escape(&value))
    .collect::<Vec<_>>()
    .join(",")
}

fn decision_signal_to_csv(
    underlying: &models::warrant::WarrantSnapshot,
    decision: &DecisionSignal,
) -> String {
    let signal = &decision.opportunity;
    let plan = decision.trade_plan.as_ref();
    let mut fields = vec![
        underlying.ticker.as_str().to_string(),
        format_float(underlying.price),
        decision_action_label(decision.decision).to_string(),
        decision.reasons.join("|"),
        decision.warnings.join("|"),
        plan.map(|plan| format_float(plan.confidence_score)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.entry_price)).unwrap_or_default(),
        plan.map(|plan| plan.entry_price_source.clone()).unwrap_or_default(),
        plan.map(|plan| format_float(plan.entry_underlying_price)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.entry_edge_pct)).unwrap_or_default(),
        plan.and_then(|plan| plan.breakeven_move_pct).map(format_float).unwrap_or_default(),
        plan.map(|plan| format_float(plan.stop.underlying_stop_price)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.stop.estimated_product_stop_price)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.stop.loss_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.stop.distance_to_stop_pct)).unwrap_or_default(),
        plan.map(|plan| format!("{:?}", plan.stop.stop_reason)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.target_1.underlying_target_price)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.target_1.estimated_product_target_price)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.target_1.gain_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.target_1.distance_to_target_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.target_2.underlying_target_price)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.target_2.estimated_product_target_price)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.target_2.gain_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.target_2.distance_to_target_pct)).unwrap_or_default(),
        plan.and_then(|plan| plan.reward_risk_1).map(format_float).unwrap_or_default(),
        plan.and_then(|plan| plan.reward_risk_2).map(format_float).unwrap_or_default(),
        plan.map(|plan| plan.horizon.holding_days.to_string()).unwrap_or_default(),
        plan.map(|plan| plan.horizon.max_holding_days.to_string()).unwrap_or_default(),
        plan.and_then(|plan| plan.horizon.maturity_days).map(|value| value.to_string()).unwrap_or_default(),
        plan.and_then(|plan| plan.horizon.theta_daily_pct).map(format_float).unwrap_or_default(),
        plan.and_then(|plan| plan.horizon.theta_to_horizon_pct).map(format_float).unwrap_or_default(),
        plan.map(|plan| format_float(plan.horizon.time_risk_score)).unwrap_or_default(),
        plan.and_then(|plan| plan.risk.barrier_risk_score).map(format_float).unwrap_or_default(),
        plan.and_then(|plan| plan.risk.spread_cost_underlying_pct).map(format_float).unwrap_or_default(),
        plan.map(|plan| format_float(plan.position.account_risk_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.position.suggested_notional_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.position.max_notional_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.position.estimated_account_loss_pct)).unwrap_or_default(),
        plan.and_then(|plan| plan.position.products_per_1000_account).map(format_float).unwrap_or_default(),
        plan.map(|plan| plan.fees.profile.clone()).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.order_notional)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.buy_fee)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.deposit_fee)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.sell_stop_fee)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.sell_target_1_fee)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.sell_target_2_fee)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.roundtrip_stop_fee_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.roundtrip_target_1_fee_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.roundtrip_target_2_fee_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.raw_stop_loss_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.raw_target_1_gain_pct)).unwrap_or_default(),
        plan.map(|plan| format_float(plan.fees.raw_target_2_gain_pct)).unwrap_or_default(),
    ];
    fields.extend(opportunity_signal_fields(signal));
    fields
        .into_iter()
        .map(|value| csv_escape(&value))
        .collect::<Vec<_>>()
        .join(",")
}

fn opportunity_signal_fields(signal: &OpportunitySignal) -> Vec<String> {
    vec![
        signal.valuation.as_str().to_string(),
        signal.side.as_str().to_string(),
        signal.moneyness.as_str().to_string(),
        signal.symbol.as_str().to_string(),
        signal.web_url.as_str().to_string(),
        signal.product_family.as_str().to_string(),
        signal.pricing_model.as_str().to_string(),
        signal.product_type.as_str().to_string(),
        signal.maturity.as_str().to_string(),
        signal.price_currency.as_str().to_string(),
        format_float(signal.last_price),
        signal.price_source.as_str().to_string(),
        signal.bid_price.map(format_float).unwrap_or_default(),
        signal.ask_price.map(format_float).unwrap_or_default(),
        signal.mid_price.map(format_float).unwrap_or_default(),
        signal.spread_pct.map(format_float).unwrap_or_default(),
        signal.bid_size.map(format_float).unwrap_or_default(),
        signal.ask_size.map(format_float).unwrap_or_default(),
        signal.quote_volume.map(format_float).unwrap_or_default(),
        signal.execution_status.as_str().to_string(),
        format_float(signal.data_quality_score),
        format_float(signal.liquidity_score),
        format_float(signal.strike),
        signal.strike_currency.as_str().to_string(),
        signal.barrier.map(format_float).unwrap_or_default(),
        signal.barrier_currency.as_deref().unwrap_or_default().to_string(),
        signal.barrier_distance_pct.map(format_float).unwrap_or_default(),
        format_float(signal.raw_intrinsic),
        format_float(signal.intrinsic_per_product),
        signal.intrinsic_currency.as_str().to_string(),
        format_float(signal.fx_rate),
        signal.fx_source_ticker.as_str().to_string(),
        signal
            .warrants_per_underlying
            .map(format_float)
            .unwrap_or_default(),
        format_float(signal.price_to_intrinsic),
        signal.effective_gearing.map(format_float).unwrap_or_default(),
        format_float(signal.premium_discount_pct),
        signal.years_to_maturity.map(format_float).unwrap_or_default(),
        signal.risk_free_rate.map(format_float).unwrap_or_default(),
        signal.dividend_yield.map(format_float).unwrap_or_default(),
        signal.implied_volatility.map(format_float).unwrap_or_default(),
        signal.smile_median_iv.map(format_float).unwrap_or_default(),
        signal.smile_gap_vol_points.map(format_float).unwrap_or_default(),
        signal.volatility_signal.as_str().to_string(),
        signal.metric_kind.as_str().to_string(),
        format_float(signal.relative_metric),
        format_float(signal.peer_median_relative_metric),
        format_float(signal.peer_gap_pct),
        format_float(signal.spread_adjusted_gap_pct),
        signal.peer_count.to_string(),
        format_float(signal.score),
        signal.note.as_str().to_string(),
    ]
}

fn default_underlying_ticker(underlying: &str) -> &'static str {
    match underlying.to_ascii_lowercase().as_str() {
        "hermes" | "hermès" | "rms" | "rms.pa" => "RMS.PA",
        _ => "RMS.PA",
    }
}

fn decision_action_label(decision: DecisionAction) -> &'static str {
    match decision {
        DecisionAction::BuyCandidate => "BUY",
        DecisionAction::Watch => "WATCH",
        DecisionAction::Avoid => "AVOID",
        DecisionAction::ExitLoss => "EXIT",
        DecisionAction::TakeProfit => "TAKE",
        DecisionAction::Hold => "HOLD",
    }
}

fn scenario_side(side: &str, target_price: f64, spot: f64) -> &'static str {
    match side {
        "force-call" | "force-calls" => "call",
        "force-put" | "force-puts" => "put",
        _ if target_price >= spot => "call",
        _ => "put",
    }
}

fn parse_date_arg(value: &str) -> Option<NaiveDate> {
    if value.trim().is_empty() || value == "-" {
        return None;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn read_env_date(name: &str) -> Option<NaiveDate> {
    std::env::var(name)
        .ok()
        .as_deref()
        .and_then(parse_date_arg)
}

fn read_env_f64(name: &str) -> Option<f64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn format_float(value: f64) -> String {
    format!("{value:.4}")
}

fn boursorama_mid(quote: Option<&models::structured::BoursoramaQuote>) -> Option<f64> {
    let quote = quote?;
    Some((quote.bid? + quote.ask?) / 2.0)
}

fn truncate_display(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    output.push('~');
    output
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
