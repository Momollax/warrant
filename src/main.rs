mod analysis;
mod api;
mod config;
mod display;
mod indicators;
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
use api::client::build_client;
use api::discover::discover_by_underlying;
use api::orats::OratsClient;
use api::polygon::PolygonClient;
use config::{resolve_max_cycles, resolve_tickers, REFRESH_INTERVAL_SECS};
use futures::{stream, StreamExt};
use indicators::warrant::{rank_relative_value, OpportunitySignal, ValuationSide};
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
    if args.get(1).is_some_and(|arg| arg == "orats-test") {
        let ticker = args.get(2).map(String::as_str).unwrap_or("RMS.PA");
        return run_orats_test(ticker).await;
    }
    if args.get(1).is_some_and(|arg| arg == "polygon-test") {
        let ticker = args.get(2).map(String::as_str).unwrap_or("RMS.PA");
        return run_polygon_test(ticker).await;
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
    if let Ok(ticker) = std::env::var("ORATS_TEST_TICKER") {
        return run_orats_test(&ticker).await;
    }
    if let Ok(ticker) = std::env::var("POLYGON_TEST_TICKER") {
        return run_polygon_test(&ticker).await;
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

    let filtered_signals = filter_opportunity_signals(&signals, side_filter);
    trace::log(format!(
        "opportunities: display format={} filtered={} total={}",
        if format.is_empty() { "auto" } else { &format },
        filtered_signals.len(),
        signals.len()
    ));

    if format == "tui" && io::stdout().is_terminal() {
        return display::opportunities::run(
            &base.underlying_quote,
            base.products.len(),
            &signals,
            side_filter,
        );
    }

    print_opportunity_summary(
        &base.underlying_quote,
        base.products.len(),
        signals.len(),
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
        "[WARN] Ces signaux ne sont pas des arbitrages garantis: FX, frais, bid/ask executables, liquidite et statut temps reel restent a verifier."
    );
}

fn print_opportunity_table(signals: &[&OpportunitySignal]) {
    print_rank_section(
        "Top 10 sous-evalues vs pairs",
        signals,
        ValuationSide::Undervalued,
        10,
    );
    println!();
    print_rank_section(
        "Top 10 sur-evalues vs pairs",
        signals,
        ValuationSide::Overvalued,
        10,
    );
}

fn print_rank_section(
    title: &str,
    signals: &[&OpportunitySignal],
    valuation: ValuationSide,
    limit: usize,
) {
    println!("{title}");
    println!(
        "{:<7} {:<7} {:<8} {:<5} {:<24} {:<10} {:>10} {:>7} {:>5} {:>7} {:>10} {:>8} {:>9} {:>9} {:>9} {:>6}",
        "edge", "gap", "symbol", "side", "type", "maturity", "price", "spr%", "dq", "iv", "ref", "parity", "intr/w", "metric", "median", "peers"
    );
    println!("{}", "-".repeat(158));

    let mut printed = 0usize;
    for signal in signals
        .iter()
        .copied()
        .filter(|signal| signal.valuation == valuation)
        .take(limit)
    {
        printed += 1;
        println!(
            "{:>+6.1}% {:>+6.1}% {:<8} {:<5} {:<24} {:<10} {:>4} {:>5.4} {:>7} {:>5.0} {:>7} {:>10.2} {:>8} {:>9.4} {:>9.4} {:>9.4} {:>6}",
            signal.spread_adjusted_gap_pct,
            signal.peer_gap_pct,
            signal.symbol,
            signal.side,
            truncate_display(&signal.product_type, 24),
            signal.maturity,
            signal.price_currency,
            signal.last_price,
            signal
                .spread_pct
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "-".to_string()),
            signal.data_quality_score,
            signal
                .implied_volatility
                .map(|value| format!("{:.1}%", value * 100.0))
                .unwrap_or_else(|| "-".to_string()),
            signal.strike,
            signal
                .warrants_per_underlying
                .map(format_compact_float)
                .unwrap_or_else(|| "?".to_string()),
            signal.intrinsic_per_product,
            signal.relative_metric,
            signal.peer_median_relative_metric,
            signal.peer_count
        );
    }

    if printed == 0 {
        println!("Aucun signal dans cette categorie avec le filtre actuel.");
    }
}

fn print_opportunity_csv(
    underlying: &models::warrant::WarrantSnapshot,
    signals: &[&OpportunitySignal],
) {
    println!(
        "underlying_ticker,underlying_price,valuation,side,moneyness,symbol,boursorama_url,product_family,pricing_model,product_type,maturity,price_currency,last_price,price_source,bid_price,ask_price,mid_price,spread_pct,bid_size,ask_size,quote_volume,execution_status,data_quality_score,liquidity_score,payoff_reference,payoff_reference_currency,barrier,barrier_currency,barrier_distance_pct,raw_intrinsic,intrinsic_per_product,intrinsic_currency,fx_rate,fx_source_ticker,warrants_per_underlying,price_to_intrinsic,effective_gearing,premium_discount_pct,years_to_maturity,risk_free_rate,dividend_yield,implied_volatility,smile_median_iv,smile_gap_vol_points,volatility_signal,metric_kind,relative_metric,peer_median_relative_metric,peer_gap_pct,spread_adjusted_gap_pct,peer_count,score,note"
    );
    for signal in signals {
        println!(
            "{}",
            opportunity_signal_to_csv(underlying, signal)
        );
    }
}

fn filter_opportunity_signals<'a>(
    signals: &'a [OpportunitySignal],
    side_filter: &str,
) -> Vec<&'a OpportunitySignal> {
    let side_filter = normalize_side_filter(side_filter);
    signals
        .iter()
        .filter(|signal| match side_filter {
            "call" => signal.side == "call",
            "put" => signal.side == "put",
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

fn opportunity_signal_to_csv(
    underlying: &models::warrant::WarrantSnapshot,
    signal: &OpportunitySignal,
) -> String {
    [
        underlying.ticker.as_str().to_string(),
        format_float(underlying.price),
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
    .into_iter()
    .map(|value| csv_escape(&value))
    .collect::<Vec<_>>()
    .join(",")
}

fn default_underlying_ticker(underlying: &str) -> &'static str {
    match underlying.to_ascii_lowercase().as_str() {
        "hermes" | "hermès" | "rms" | "rms.pa" => "RMS.PA",
        _ => "RMS.PA",
    }
}

fn format_float(value: f64) -> String {
    format!("{value:.4}")
}

fn format_compact_float(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.4}")
    }
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
