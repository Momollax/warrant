use std::io;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};

use crate::indicators::warrant::{OpportunitySignal, ValuationSide};
use crate::models::warrant::WarrantSnapshot;

pub fn run(
    underlying: &WarrantSnapshot,
    product_count: usize,
    signals: &[OpportunitySignal],
    initial_side_filter: &str,
) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut state = TableState::default();
    if !signals.is_empty() {
        state.select(Some(0));
    }
    let mut side_filter = SideFilter::from_str(initial_side_filter);
    let mut valuation_filter = ValuationFilter::Undervalued;
    let mut status_message = String::from("Entree/Espace: ouvrir Boursorama pour la ligne selectionnee");

    let result = loop {
        let visible_signals = filter_signals(signals, side_filter, valuation_filter);
        clamp_selection(&mut state, visible_signals.len());

        terminal.draw(|frame| {
            draw(
                frame,
                underlying,
                product_count,
                signals.len(),
                &visible_signals,
                &mut state,
                side_filter,
                valuation_filter,
                &status_message,
            )
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => select_next(&mut state, visible_signals.len()),
                    KeyCode::Up | KeyCode::Char('k') => select_previous(&mut state, visible_signals.len()),
                    KeyCode::Home => select_first(&mut state, visible_signals.len()),
                    KeyCode::End => select_last(&mut state, visible_signals.len()),
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        status_message = selected_action_message(state.selected(), &visible_signals);
                    }
                    KeyCode::Char('a') => {
                        side_filter = SideFilter::All;
                        select_first(&mut state, filter_signals(signals, side_filter, valuation_filter).len());
                    }
                    KeyCode::Char('c') => {
                        side_filter = SideFilter::Call;
                        select_first(&mut state, filter_signals(signals, side_filter, valuation_filter).len());
                    }
                    KeyCode::Char('p') => {
                        side_filter = SideFilter::Put;
                        select_first(&mut state, filter_signals(signals, side_filter, valuation_filter).len());
                    }
                    KeyCode::Char('u') => {
                        valuation_filter = ValuationFilter::Undervalued;
                        select_first(&mut state, filter_signals(signals, side_filter, valuation_filter).len());
                    }
                    KeyCode::Char('o') => {
                        valuation_filter = ValuationFilter::Overvalued;
                        select_first(&mut state, filter_signals(signals, side_filter, valuation_filter).len());
                    }
                    _ => {}
                }
            }
        }
    };

    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

fn draw(
    frame: &mut Frame,
    underlying: &WarrantSnapshot,
    product_count: usize,
    total_signal_count: usize,
    visible_signals: &[&OpportunitySignal],
    state: &mut TableState,
    side_filter: SideFilter,
    valuation_filter: ValuationFilter,
    status_message: &str,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(8),
            Constraint::Length(14),
        ])
        .split(frame.area());

    draw_header(
        frame,
        chunks[0],
        underlying,
        product_count,
        total_signal_count,
        visible_signals.len(),
        side_filter,
        valuation_filter,
    );
    draw_table(frame, chunks[1], visible_signals, state);
    draw_details(
        frame,
        chunks[2],
        visible_signals,
        state.selected(),
        side_filter,
        valuation_filter,
        status_message,
    );
}

fn draw_header(
    frame: &mut Frame,
    area: Rect,
    underlying: &WarrantSnapshot,
    product_count: usize,
    total_signal_count: usize,
    visible_signal_count: usize,
    side_filter: SideFilter,
    valuation_filter: ValuationFilter,
) {
    let lines = vec![
        Line::from(vec![
            Span::styled("Spot ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "{} {:.4} {}",
                    underlying.ticker, underlying.price, underlying.currency
                ),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            colored_change(underlying.change_pct),
            Span::raw("    "),
            Span::styled("Produits ", Style::default().fg(Color::DarkGray)),
            Span::styled(product_count.to_string(), Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Signaux ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{visible_signal_count}/{total_signal_count}"),
                Style::default()
                    .fg(if total_signal_count > 0 { Color::Yellow } else { Color::Green })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Filtre ", Style::default().fg(Color::DarkGray)),
            Span::styled(side_filter.label(), Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::styled("Vue ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                valuation_filter.label(),
                valuation_style(valuation_filter.side()).add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled("Tri ", Style::default().fg(Color::DarkGray)),
            Span::styled("edge net", Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Seuil ", Style::default().fg(Color::DarkGray)),
            Span::styled("OPPORTUNITY_MIN_GAP_PCT", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Decision ", Style::default().fg(Color::DarkGray)),
            Span::raw("Edge = gap relatif - cout du spread; "),
            Span::styled("Execution ", Style::default().fg(Color::DarkGray)),
            Span::raw("DQ/Liq + bid/ask; "),
            Span::styled("Risque ", Style::default().fg(Color::DarkGray)),
            Span::raw("levier + distance barriere; "),
            Span::styled("Options ", Style::default().fg(Color::DarkGray)),
            Span::raw("IV/smile si disponibles"),
        ]),
        Line::from(vec![
            Span::styled("[a]", Style::default().fg(Color::Yellow)),
            Span::raw(" all  "),
            Span::styled("[c]", Style::default().fg(Color::Yellow)),
            Span::raw(" calls  "),
            Span::styled("[p]", Style::default().fg(Color::Yellow)),
            Span::raw(" puts  "),
            Span::styled("[u]", Style::default().fg(Color::Yellow)),
            Span::raw(" sous-evalues  "),
            Span::styled("[o]", Style::default().fg(Color::Yellow)),
            Span::raw(" sur-evalues  "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Boursorama  "),
            Span::styled("[q]", Style::default().fg(Color::Yellow)),
            Span::raw(" quitter  "),
            Span::styled("Edge", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw(" = signal net"),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Decorrelation candidates ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn colored_change(change_pct: f64) -> Span<'static> {
    let color = if change_pct >= 0.0 {
        Color::Green
    } else {
        Color::Red
    };
    Span::styled(
        format!("{change_pct:+.2}%"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn draw_table(
    frame: &mut Frame,
    area: Rect,
    signals: &[&OpportunitySignal],
    state: &mut TableState,
) {
    let header = Row::new([
        "Edge",
        "Gap",
        "Spr",
        "DQ",
        "Liq",
        "Symbol",
        "Side",
        "Mny",
        "Mat.",
        "Price",
        "Ref.",
        "Bar%",
        "Lev",
        "IV",
        "IVd",
        "Parity",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(1);

    let rows = signals.iter().map(|signal| {
        Row::new(vec![
            Cell::from(format!("{:+.1}%", signal.spread_adjusted_gap_pct))
                .style(edge_style(signal)),
            Cell::from(format!("{:+.1}%", signal.peer_gap_pct))
                .style(valuation_style(signal.valuation)),
            Cell::from(
                signal
                    .spread_pct
                    .map(|value| format!("{value:.1}%"))
                    .unwrap_or_else(|| "-".to_string()),
            )
            .style(spread_style(signal.spread_pct)),
            Cell::from(format!("{:.0}", signal.data_quality_score))
                .style(score_style(signal.data_quality_score)),
            Cell::from(format!("{:.0}", signal.liquidity_score))
                .style(score_style(signal.liquidity_score)),
            Cell::from(signal.symbol.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(signal.side.clone()).style(side_style(&signal.side)),
            Cell::from(signal.moneyness.clone()),
            Cell::from(signal.maturity.clone()),
            Cell::from(format!(
                "{} {:.4}",
                signal.price_currency, signal.last_price
            )),
            Cell::from(format!("{:.2}", signal.strike)),
            Cell::from(
                signal
                    .barrier_distance_pct
                    .map(|value| format!("{value:.1}%"))
                    .unwrap_or_else(|| "-".to_string()),
            )
            .style(barrier_style(signal.barrier_distance_pct)),
            Cell::from(format_optional(signal.effective_gearing)),
            Cell::from(format_optional_pct(signal.implied_volatility)),
            Cell::from(
                signal
                    .smile_gap_vol_points
                    .map(|value| format!("{value:+.1}pt"))
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(
                signal
                    .warrants_per_underlying
                    .map(format_compact_float)
                    .unwrap_or_else(|| "?".to_string()),
            ),
        ])
    });

    let widths = [
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(" Candidates ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol("> ");

    frame.render_stateful_widget(table, area, state);
}

fn draw_details(
    frame: &mut Frame,
    area: Rect,
    signals: &[&OpportunitySignal],
    selected: Option<usize>,
    side_filter: SideFilter,
    valuation_filter: ValuationFilter,
    status_message: &str,
) {
    let text = selected
        .and_then(|index| signals.get(index))
        .map(|signal| {
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Selected ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        signal.symbol.clone(),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled("edge net ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{:+.2}%", signal.spread_adjusted_gap_pct),
                        valuation_style(signal.valuation),
                    ),
                    Span::raw("  "),
                    Span::styled("gap median ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{:+.2}%", signal.peer_gap_pct),
                        valuation_style(signal.valuation),
                    ),
                    Span::raw("  "),
                    Span::styled("model ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{}/{}", signal.product_family, signal.pricing_model),
                        Style::default().fg(Color::White),
                    ),
                    Span::raw("  "),
                    Span::styled("parity ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        signal
                            .warrants_per_underlying
                            .map(format_compact_float)
                            .unwrap_or_else(|| "?".to_string()),
                        Style::default().fg(Color::White),
                    ),
                    Span::raw("  "),
                    Span::styled("metric ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!(
                            "{} {:.4} / median {:.4}",
                            signal.metric_kind,
                            signal.relative_metric,
                            signal.peer_median_relative_metric
                        ),
                        Style::default().fg(Color::White),
                    ),
                    Span::raw("  "),
                    Span::styled("price ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        signal.price_source.clone(),
                        Style::default().fg(Color::White),
                    ),
                    Span::raw("  "),
                    Span::styled("fx ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!(
                            "{}->{} {:.4} ({})",
                            signal.strike_currency,
                            signal.intrinsic_currency,
                            signal.fx_rate,
                            signal.fx_source_ticker
                        ),
                        Style::default().fg(Color::White),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Decision ", Style::default().fg(Color::Yellow)),
                    Span::styled(signal_decision(signal), decision_style(signal)),
                    Span::raw("  "),
                    Span::styled("Score ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{:.2}", signal.score), Style::default().fg(Color::White)),
                    Span::raw("  "),
                    Span::styled("Pairs ", Style::default().fg(Color::DarkGray)),
                    Span::styled(signal.peer_count.to_string(), Style::default().fg(Color::White)),
                    Span::raw("  "),
                    Span::styled("Moneyness ", Style::default().fg(Color::DarkGray)),
                    Span::styled(signal.moneyness.clone(), Style::default().fg(Color::White)),
                    Span::raw("  "),
                    Span::styled("Type ", Style::default().fg(Color::DarkGray)),
                    Span::styled(signal.product_type.clone(), Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("URL ", Style::default().fg(Color::Yellow)),
                    Span::styled(signal.web_url.clone(), Style::default().fg(Color::Cyan)),
                ]),
            ];
            lines.extend(status_message.lines().map(|line| {
                Line::from(vec![
                    Span::styled("Action ", Style::default().fg(Color::Yellow)),
                    Span::raw(line.to_string()),
                ])
            }));
            lines.extend([
                Line::from(format!(
                    "Calc metric: {}",
                    metric_formula(signal)
                )),
                Line::from(format!(
                    "Calc gap: ({:.4} - {:.4}) / {:.4} * 100 = {:+.2}% avec {} pairs",
                    signal.peer_median_relative_metric,
                    signal.relative_metric,
                    signal.peer_median_relative_metric,
                    signal.peer_gap_pct,
                    signal.peer_count
                )),
                Line::from(format!(
                    "Calc edge: gap {:+.2}% - spread {:.2}% = {:+.2}% net",
                    signal.peer_gap_pct,
                    signal.spread_pct.unwrap_or(0.0),
                    signal.spread_adjusted_gap_pct
                )),
                Line::from(format!(
                    "Execution: {}  bid {}  ask {}  mid {}  spread {}  dataQ {:.0}/100  liq {:.0}/100  bidSize {}  askSize {}  vol {}",
                    signal.execution_status,
                    format_optional(signal.bid_price),
                    format_optional(signal.ask_price),
                    format_optional(signal.mid_price),
                    signal
                        .spread_pct
                        .map(|value| format!("{value:.2}%"))
                        .unwrap_or_else(|| "-".to_string()),
                    signal.data_quality_score,
                    signal.liquidity_score,
                    format_optional(signal.bid_size),
                    format_optional(signal.ask_size),
                    format_optional(signal.quote_volume)
                )),
                Line::from(format!(
                    "Risk/Options: gearing {}  barrierDist {}  IV {}  smile {}  gapIV {}  signal {}  T {}  r {}  q {}",
                    format_optional(signal.effective_gearing),
                    signal
                        .barrier_distance_pct
                        .map(|value| format!("{value:.2}%"))
                        .unwrap_or_else(|| "-".to_string()),
                    format_optional_pct(signal.implied_volatility),
                    format_optional_pct(signal.smile_median_iv),
                    signal
                        .smile_gap_vol_points
                        .map(|value| format!("{value:+.2}pt"))
                        .unwrap_or_else(|| "-".to_string()),
                    signal.volatility_signal,
                    format_optional(signal.years_to_maturity),
                    format_optional_pct(signal.risk_free_rate),
                    format_optional_pct(signal.dividend_yield)
                )),
                Line::from(format!(
                    "Value: ref {} {}  barrier {}  intr/w {} {}  metric {:.4}  median {:.4}",
                    format_optional(Some(signal.strike)),
                    signal.strike_currency,
                    signal
                        .barrier
                        .map(|value| format!("{value:.4}"))
                        .unwrap_or_else(|| "-".to_string()),
                    format_optional(Some(signal.intrinsic_per_product)),
                    signal.intrinsic_currency,
                    signal.relative_metric,
                    signal.peer_median_relative_metric
                )),
                Line::from(format!(
                    "Filtre: {}, vue {}. Signal brut seulement: FX, frais, bid/ask executable et statut temps reel sont requis. {}",
                    side_filter.label(),
                    valuation_filter.label(),
                    signal.note
                )),
                Line::from(vec![
                    Span::styled("Raccourcis ", Style::default().fg(Color::Yellow)),
                    Span::raw("Entree/Espace ouvre ou affiche le lien Boursorama"),
                ]),
            ]);
            lines
        })
        .unwrap_or_else(|| {
            vec![
                Line::from(
                    "Aucun candidat visible avec ce filtre. Utilise a/c/p, u/o ou augmente la limite.",
                ),
                Line::from(status_message.to_string()),
            ]
        });

    let block = Block::default()
        .title(" Notes ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }).block(block), area);
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SideFilter {
    All,
    Call,
    Put,
}

impl SideFilter {
    fn from_str(value: &str) -> Self {
        match value {
            "call" | "calls" => Self::Call,
            "put" | "puts" => Self::Put,
            _ => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Call => "calls",
            Self::Put => "puts",
        }
    }

    fn accepts(self, signal: &OpportunitySignal) -> bool {
        match self {
            Self::All => true,
            Self::Call => signal.side == "call",
            Self::Put => signal.side == "put",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ValuationFilter {
    Undervalued,
    Overvalued,
}

impl ValuationFilter {
    fn label(self) -> &'static str {
        match self {
            Self::Undervalued => "sous-evalues",
            Self::Overvalued => "sur-evalues",
        }
    }

    fn side(self) -> ValuationSide {
        match self {
            Self::Undervalued => ValuationSide::Undervalued,
            Self::Overvalued => ValuationSide::Overvalued,
        }
    }

    fn accepts(self, signal: &OpportunitySignal) -> bool {
        signal.valuation == self.side()
    }
}

fn filter_signals(
    signals: &[OpportunitySignal],
    side_filter: SideFilter,
    valuation_filter: ValuationFilter,
) -> Vec<&OpportunitySignal> {
    signals
        .iter()
        .filter(|signal| side_filter.accepts(signal) && valuation_filter.accepts(signal))
        .collect()
}

fn clamp_selection(state: &mut TableState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }

    match state.selected() {
        Some(index) if index < len => {}
        _ => state.select(Some(len - 1)),
    }
}

fn valuation_style(valuation: ValuationSide) -> Style {
    match valuation {
        ValuationSide::Undervalued => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        ValuationSide::Overvalued => Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::BOLD),
    }
}

fn edge_style(signal: &OpportunitySignal) -> Style {
    let useful = match signal.valuation {
        ValuationSide::Undervalued => signal.spread_adjusted_gap_pct > 0.0,
        ValuationSide::Overvalued => signal.spread_adjusted_gap_pct < 0.0,
    };

    if useful {
        valuation_style(signal.valuation)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn side_style(side: &str) -> Style {
    match side {
        "call" => Style::default().fg(Color::Green),
        "put" => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::DarkGray),
    }
}

fn spread_style(spread_pct: Option<f64>) -> Style {
    match spread_pct {
        Some(value) if value <= 2.0 => Style::default().fg(Color::Green),
        Some(value) if value <= 5.0 => Style::default().fg(Color::Yellow),
        Some(_) => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        None => Style::default().fg(Color::DarkGray),
    }
}

fn score_style(score: f64) -> Style {
    if score >= 80.0 {
        Style::default().fg(Color::Green)
    } else if score >= 60.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red)
    }
}

fn barrier_style(barrier_distance_pct: Option<f64>) -> Style {
    match barrier_distance_pct {
        Some(value) if value >= 15.0 => Style::default().fg(Color::Green),
        Some(value) if value >= 8.0 => Style::default().fg(Color::Yellow),
        Some(_) => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        None => Style::default().fg(Color::DarkGray),
    }
}

fn signal_decision(signal: &OpportunitySignal) -> &'static str {
    let edge_ok = match signal.valuation {
        ValuationSide::Undervalued => signal.spread_adjusted_gap_pct > 0.0,
        ValuationSide::Overvalued => signal.spread_adjusted_gap_pct < 0.0,
    };
    if !edge_ok {
        return "spread_eats_edge";
    }
    if signal.execution_status != "executable_bid_ask" {
        return "not_executable";
    }
    if signal.data_quality_score < 60.0 {
        return "weak_data";
    }
    if signal.spread_pct.unwrap_or(100.0) > 5.0 {
        return "wide_spread";
    }
    "candidate"
}

fn decision_style(signal: &OpportunitySignal) -> Style {
    match signal_decision(signal) {
        "candidate" => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        "wide_spread" | "weak_data" => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

fn format_compact_float(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.4}")
    }
}

fn format_optional(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_pct(value: Option<f64>) -> String {
    value
        .map(|value| format!("{:.2}%", value * 100.0))
        .unwrap_or_else(|| "-".to_string())
}

fn calculation_summary(signal: &OpportunitySignal) -> String {
    format!(
        "Calcul: {}; gap=({:.4}-{:.4})/{:.4}*100={:+.2}%",
        metric_formula(signal),
        signal.peer_median_relative_metric,
        signal.relative_metric,
        signal.peer_median_relative_metric,
        signal.peer_gap_pct
    )
}

fn selected_action_message(selected: Option<usize>, signals: &[&OpportunitySignal]) -> String {
    selected
        .and_then(|index| signals.get(index))
        .map(|signal| {
            let open_status = match open_url(&signal.web_url) {
                Ok(()) => "ouverture demandee".to_string(),
                Err(err) => format!("ouverture impossible depuis cet environnement ({err})"),
            };
            format!(
                "{} | URL: {} | {}",
                open_status,
                signal.web_url,
                calculation_summary(signal)
            )
        })
        .unwrap_or_else(|| "Aucun produit selectionne.".to_string())
}

fn metric_formula(signal: &OpportunitySignal) -> String {
    match signal.metric_kind.as_str() {
        "premium_pct" => warrant_premium_formula(signal),
        "price_to_intrinsic" => format!(
            "metric=price_to_intrinsic=prix / intr/w = {:.4} / {:.4} = {:.4}",
            signal.last_price, signal.intrinsic_per_product, signal.relative_metric
        ),
        _ => format!(
            "metric={}={:.4}",
            signal.metric_kind, signal.relative_metric
        ),
    }
}

fn warrant_premium_formula(signal: &OpportunitySignal) -> String {
    let parity = signal.warrants_per_underlying.unwrap_or(0.0);
    let price_in_reference = if signal.fx_rate > 0.0 {
        signal.last_price / signal.fx_rate
    } else {
        0.0
    };
    let spot = implied_spot(signal).unwrap_or(0.0);
    match signal.side.as_str() {
        "call" => format!(
            "premium_call=((K + prix_ref*parity - S)/S)*100; prix_ref={} {:.4}/fx {:.4}={} {:.4}; (({:.4} + {:.4}*{:.4} - {:.4})/{:.4})*100 = {:.4}%",
            signal.price_currency,
            signal.last_price,
            signal.fx_rate,
            signal.strike_currency,
            price_in_reference,
            signal.strike,
            price_in_reference,
            parity,
            spot,
            spot,
            signal.relative_metric
        ),
        "put" => format!(
            "premium_put=((S + prix_ref*parity - K)/S)*100; prix_ref={} {:.4}/fx {:.4}={} {:.4}; (({:.4} + {:.4}*{:.4} - {:.4})/{:.4})*100 = {:.4}%",
            signal.price_currency,
            signal.last_price,
            signal.fx_rate,
            signal.strike_currency,
            price_in_reference,
            spot,
            price_in_reference,
            parity,
            signal.strike,
            spot,
            signal.relative_metric
        ),
        _ => format!("premium_pct={:.4}%", signal.relative_metric),
    }
}

fn implied_spot(signal: &OpportunitySignal) -> Option<f64> {
    match signal.side.as_str() {
        "call" => Some(signal.strike + signal.raw_intrinsic),
        "put" => Some(signal.strike - signal.raw_intrinsic),
        _ => None,
    }
}

fn open_url(url: &str) -> std::result::Result<(), String> {
    if let Ok(browser) = std::env::var("BROWSER") {
        if !browser.trim().is_empty() {
            return Command::new(browser)
                .arg(url)
                .spawn()
                .map(|_| ())
                .map_err(|err| err.to_string());
        }
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}

fn select_next(state: &mut TableState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let next = state.selected().map_or(0, |index| (index + 1).min(len - 1));
    state.select(Some(next));
}

fn select_previous(state: &mut TableState, len: usize) {
    if len == 0 {
        state.select(None);
        return;
    }
    let previous = state.selected().map_or(0, |index| index.saturating_sub(1));
    state.select(Some(previous));
}

fn select_first(state: &mut TableState, len: usize) {
    state.select((len > 0).then_some(0));
}

fn select_last(state: &mut TableState, len: usize) {
    state.select((len > 0).then_some(len - 1));
}
