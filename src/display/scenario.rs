use std::collections::HashMap;
use std::io;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use chrono::NaiveDate;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame, Terminal,
};

use crate::decision::config::DecisionConfig;
use crate::decision::scenario::{analyze_warrant_scenario, ScenarioCandidate, ScenarioConfig};
use crate::indicators::warrant::OpportunitySignal;
use crate::llm::gemini::{analyze_scenario_candidate, GeminiConfig, LlmScenarioReview};
use crate::models::warrant::WarrantSnapshot;

pub fn run(
    underlying: &WarrantSnapshot,
    product_count: usize,
    signals: &[OpportunitySignal],
    decision_config: &DecisionConfig,
    initial_config: &ScenarioConfig,
    requested_side: &str,
    today: NaiveDate,
) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut state = TableState::default();
    let mut config = initial_config.clone();
    let mut candidates = analyze_warrant_scenario(
        signals,
        underlying.price,
        &config,
        decision_config,
        today,
    );
    if !candidates.is_empty() {
        state.select(Some(0));
    }
    let mut decision_filter = DecisionFilter::All;
    let mut notes_scroll = 0u16;
    let mut detail_tab = DetailTab::Notes;
    let mut target_edit = None::<String>;
    let mut llm_reviews = HashMap::<String, String>::new();
    let mut status_message =
        String::from("Entree/Espace: ouvrir Boursorama | l: onglet LLM | r: interroger Gemini");
    let llm_config = GeminiConfig::from_env();
    let llm_client = reqwest::Client::new();

    let result = loop {
        let visible = filter_candidates(&candidates, decision_filter);
        clamp_selection(&mut state, visible.len());

        terminal.draw(|frame| {
            draw(
                frame,
                underlying,
                product_count,
                candidates.len(),
                &visible,
                &config,
                decision_filter,
                &mut state,
                notes_scroll,
                detail_tab,
                target_edit.as_deref(),
                &llm_reviews,
                &status_message,
            )
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if let Some(input) = target_edit.as_mut() {
                    match key.code {
                        KeyCode::Enter => {
                            let parsed = input.replace(',', ".").parse::<f64>();
                            match parsed {
                                Ok(target) if target > 0.0 => {
                                    drop(visible);
                                    config.target_price = target;
                                    config.side = resolve_scenario_side(
                                        requested_side,
                                        config.target_price,
                                        underlying.price,
                                    )
                                    .to_string();
                                    candidates = analyze_warrant_scenario(
                                        signals,
                                        underlying.price,
                                        &config,
                                        decision_config,
                                        today,
                                    );
                                    select_first(&mut state, candidates.len());
                                    decision_filter = DecisionFilter::All;
                                    notes_scroll = 0;
                                    detail_tab = DetailTab::Notes;
                                    llm_reviews.clear();
                                    status_message = format!(
                                        "Cible modifiee a {:.4}; scenario recalcule en {} avec {} candidat(s).",
                                        config.target_price,
                                        config.side,
                                        candidates.len()
                                    );
                                    target_edit = None;
                                    continue;
                                }
                                _ => {
                                    status_message =
                                        format!("Cible invalide: '{input}'. Exemple: 1300 ou 1700.5");
                                    target_edit = None;
                                    continue;
                                }
                            }
                        }
                        KeyCode::Esc => {
                            target_edit = None;
                            status_message = "Edition cible annulee.".to_string();
                            continue;
                        }
                        KeyCode::Backspace => {
                            input.pop();
                            continue;
                        }
                        KeyCode::Char(ch)
                            if ch.is_ascii_digit() || matches!(ch, '.' | ',' | '_') =>
                        {
                            if ch != '_' {
                                input.push(ch);
                            }
                            continue;
                        }
                        _ => continue,
                    }
                }

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                    KeyCode::Char('m') => {
                        target_edit = Some(format_compact_float(config.target_price));
                        status_message =
                            "Edition cible: tape la nouvelle valeur, Enter valide, Esc annule."
                                .to_string();
                    }
                    KeyCode::Char('l') => {
                        detail_tab = detail_tab.toggle();
                        notes_scroll = 0;
                    }
                    KeyCode::Char('r') => {
                        detail_tab = DetailTab::Llm;
                        notes_scroll = 0;
                        if let Some(symbol) = selected_symbol(state.selected(), &visible) {
                            if !llm_reviews.contains_key(&symbol) {
                                status_message = format!("Gemini analyse en cours pour {symbol}...");
                                llm_reviews.insert(symbol.clone(), llm_in_progress_message(&symbol));
                                terminal.draw(|frame| {
                                    draw(
                                        frame,
                                        underlying,
                                        product_count,
                                        candidates.len(),
                                        &visible,
                                        &config,
                                        decision_filter,
                                        &mut state,
                                        notes_scroll,
                                        detail_tab,
                                        target_edit.as_deref(),
                                        &llm_reviews,
                                        &status_message,
                                    )
                                })?;
                            }
                        }
                        status_message = selected_llm_message(
                            state.selected(),
                            &visible,
                            underlying,
                            &config,
                            &llm_config,
                            &llm_client,
                            &mut llm_reviews,
                        );
                    }
                    KeyCode::Right | KeyCode::Char('n') => {
                        notes_scroll = notes_scroll.saturating_add(1);
                    }
                    KeyCode::Left | KeyCode::Char('p') => {
                        notes_scroll = notes_scroll.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        notes_scroll = notes_scroll.saturating_add(1);
                    }
                    KeyCode::Up | KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        notes_scroll = notes_scroll.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        select_next(&mut state, visible.len());
                        notes_scroll = 0;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        select_previous(&mut state, visible.len());
                        notes_scroll = 0;
                    }
                    KeyCode::Tab | KeyCode::Char('v') => {
                        notes_scroll = notes_scroll.saturating_add(6);
                    }
                    KeyCode::BackTab | KeyCode::Char('V') => {
                        notes_scroll = notes_scroll.saturating_sub(6);
                    }
                    KeyCode::Char('t') => {
                        notes_scroll = 0;
                    }
                    KeyCode::Char('e') => {
                        notes_scroll = u16::MAX;
                    }
                    KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        notes_scroll = 0;
                    }
                    KeyCode::Home => {
                        select_first(&mut state, visible.len());
                        notes_scroll = 0;
                    }
                    KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        notes_scroll = u16::MAX;
                    }
                    KeyCode::End => {
                        select_last(&mut state, visible.len());
                        notes_scroll = 0;
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        status_message = selected_action_message(state.selected(), &visible);
                    }
                    KeyCode::Char('b') => {
                        decision_filter = DecisionFilter::Buy;
                        select_first(&mut state, filter_candidates(&candidates, decision_filter).len());
                        notes_scroll = 0;
                    }
                    KeyCode::Char('w') => {
                        decision_filter = DecisionFilter::Watch;
                        select_first(&mut state, filter_candidates(&candidates, decision_filter).len());
                        notes_scroll = 0;
                    }
                    KeyCode::Char('x') => {
                        decision_filter = DecisionFilter::Avoid;
                        select_first(&mut state, filter_candidates(&candidates, decision_filter).len());
                        notes_scroll = 0;
                    }
                    KeyCode::Char('d') => {
                        decision_filter = DecisionFilter::All;
                        select_first(&mut state, filter_candidates(&candidates, decision_filter).len());
                        notes_scroll = 0;
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
    total_candidate_count: usize,
    candidates: &[&ScenarioCandidate],
    config: &ScenarioConfig,
    decision_filter: DecisionFilter,
    state: &mut TableState,
    notes_scroll: u16,
    detail_tab: DetailTab,
    target_edit: Option<&str>,
    llm_reviews: &HashMap<String, String>,
    status_message: &str,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Min(8),
            Constraint::Percentage(38),
        ])
        .split(frame.area());

    draw_header(
        frame,
        chunks[0],
        underlying,
        product_count,
        total_candidate_count,
        candidates.len(),
        config,
        decision_filter,
        target_edit,
    );
    draw_table(frame, chunks[1], candidates, state);
    draw_details(
        frame,
        chunks[2],
        candidates,
        state.selected(),
        config,
        notes_scroll,
        detail_tab,
        llm_reviews,
        status_message,
    );
}

fn draw_header(
    frame: &mut Frame,
    area: Rect,
    underlying: &WarrantSnapshot,
    product_count: usize,
    total_candidate_count: usize,
    visible_candidate_count: usize,
    config: &ScenarioConfig,
    decision_filter: DecisionFilter,
    target_edit: Option<&str>,
) {
    let maturity = format!(
        "{}..{}",
        config
            .min_maturity
            .map(|date| date.to_string())
            .unwrap_or_else(|| "-".to_string()),
        config
            .max_maturity
            .map(|date| date.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    let target_move = if underlying.price > 0.0 {
        (config.target_price - underlying.price) / underlying.price * 100.0
    } else {
        0.0
    };

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
            Span::styled("Target ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                target_edit
                    .map(|value| format!("edition {value}_ au {}", config.target_date))
                    .unwrap_or_else(|| {
                        format!(
                            "{:.4} au {} ({:+.2}%)",
                            config.target_price, config.target_date, target_move
                        )
                    }),
                target_edit
                    .map(|_| Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                    .unwrap_or_else(|| target_style(target_move)),
            ),
        ]),
        Line::from(vec![
            Span::styled("These ", Style::default().fg(Color::DarkGray)),
            Span::styled(config.side.clone(), side_style(&config.side)),
            Span::raw("  "),
            Span::styled("Maturite ", Style::default().fg(Color::DarkGray)),
            Span::styled(maturity, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Produits ", Style::default().fg(Color::DarkGray)),
            Span::styled(product_count.to_string(), Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Candidats ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{visible_candidate_count}/{total_candidate_count}"),
                Style::default()
                    .fg(if total_candidate_count > 0 { Color::Yellow } else { Color::Red })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Decision ", Style::default().fg(Color::DarkGray)),
            Span::styled(decision_filter.label(), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Mode scenario ", Style::default().fg(Color::Yellow)),
            Span::raw("classe les warrants selon le prix projete au target, net des frais; "),
            Span::styled("ce n'est pas une prediction", Style::default().fg(Color::Red)),
        ]),
        Line::from(vec![
            Span::styled("[b]", Style::default().fg(Color::Yellow)),
            Span::raw(" buy  "),
            Span::styled("[w]", Style::default().fg(Color::Yellow)),
            Span::raw(" watch  "),
            Span::styled("[x]", Style::default().fg(Color::Yellow)),
            Span::raw(" avoid  "),
            Span::styled("[d]", Style::default().fg(Color::Yellow)),
            Span::raw(" decisions all  "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Boursorama  "),
            Span::styled("[m]", Style::default().fg(Color::Yellow)),
            Span::raw(" modifier cible  "),
            Span::styled("[l]", Style::default().fg(Color::Yellow)),
            Span::raw(" Notes/LLM  "),
            Span::styled("[r]", Style::default().fg(Color::Yellow)),
            Span::raw(" Gemini  "),
            Span::styled("[<- / ->]", Style::default().fg(Color::Yellow)),
            Span::raw(" notes ligne  "),
            Span::styled("[Tab/Shift+Tab]", Style::default().fg(Color::Yellow)),
            Span::raw(" notes page  "),
            Span::styled("[n/p]", Style::default().fg(Color::Yellow)),
            Span::raw(" notes  "),
            Span::styled("[q]", Style::default().fg(Color::Yellow)),
            Span::raw(" quitter"),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(
            " Scenario warrants ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
}

fn draw_table(
    frame: &mut Frame,
    area: Rect,
    candidates: &[&ScenarioCandidate],
    state: &mut TableState,
) {
    let header = Row::new([
        "Dec", "Score", "Net", "PBE", "PTgt", "EV", "Kelly", "Delta", "Symb", "Side",
        "Strike", "Mat.", "Entry", "TargetPx", "BE", "IV", "Vol", "DQ", "Flow", "Parity",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(1);

    let rows = candidates.iter().map(|candidate| {
        Row::new(vec![
            Cell::from(candidate.decision).style(decision_style(candidate.decision)),
            Cell::from(format!("{:.0}", candidate.score)).style(score_style(candidate.score)),
            Cell::from(format!("{:+.1}%", candidate.net_return_pct))
                .style(return_style(candidate.net_return_pct)),
            Cell::from(format_optional_plain_pct(candidate.probability_breakeven_pct)),
            Cell::from(format_optional_plain_pct(candidate.probability_target_pct)),
            Cell::from(format_optional_signed_pct(candidate.expected_value_pct)),
            Cell::from(format_optional_plain_pct(candidate.kelly_fraction_pct)),
            Cell::from(format_optional_fixed(candidate.delta, 4)),
            Cell::from(candidate.symbol.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(candidate.side.clone()).style(side_style(&candidate.side)),
            Cell::from(format!("{:.2}", candidate.strike)),
            Cell::from(candidate.maturity.to_string()),
            Cell::from(format!("{:.4}", candidate.entry_price)),
            Cell::from(format!("{:.4}", candidate.projected_price)),
            Cell::from(
                candidate
                    .breakeven_underlying
                    .map(|value| format!("{value:.2}"))
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(format_optional_factor_pct(candidate.implied_volatility)),
            Cell::from(format!("{:.1}%", candidate.volatility_used * 100.0)),
            Cell::from(format!("{:.0}", candidate.data_quality_score))
                .style(score_style(candidate.data_quality_score)),
            Cell::from(format!("{:.0}", candidate.liquidity_score))
                .style(score_style(candidate.liquidity_score)),
            Cell::from(format_compact_float(candidate.parity)),
        ])
    });

    let widths = [
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(5),
        Constraint::Length(5),
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
    candidates: &[&ScenarioCandidate],
    selected: Option<usize>,
    config: &ScenarioConfig,
    notes_scroll: u16,
    detail_tab: DetailTab,
    llm_reviews: &HashMap<String, String>,
    status_message: &str,
) {
    let lines = if detail_tab == DetailTab::Llm {
        llm_lines(candidates, selected, llm_reviews, status_message)
    } else {
        selected
        .and_then(|index| candidates.get(index))
        .map(|candidate| {
            vec![
                Line::from(vec![
                    Span::styled("Decision ", Style::default().fg(Color::DarkGray)),
                    Span::styled(candidate.decision, decision_style(candidate.decision)),
                    Span::raw("  "),
                    Span::styled("Score ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{:.0}", candidate.score), score_style(candidate.score)),
                    Span::raw("  "),
                    Span::styled("Selected ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        candidate.symbol.clone(),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled("Type ", Style::default().fg(Color::DarkGray)),
                    Span::styled(candidate.product_type.clone(), Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("These ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "{} target {:.4} au {}, strike {:.4}, maturite {}",
                        candidate.side,
                        config.target_price,
                        config.target_date,
                        candidate.strike,
                        candidate.maturity
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Projection ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "entry {:.4} -> targetPx {:.4}; gross {:+.2}% -> net {:+.2}% apres frais {:.2}%",
                        candidate.entry_price,
                        candidate.projected_price,
                        candidate.gross_return_pct,
                        candidate.net_return_pct,
                        candidate.fee_drag_pct
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Probabilites ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "P(target) {}  zTarget {}  P(BE) {}  zBE {}  drift=r-q={:.2}%  vol={:.2}%",
                        format_optional_plain_pct(candidate.probability_target_pct),
                        format_optional_fixed(candidate.target_zscore, 3),
                        format_optional_plain_pct(candidate.probability_breakeven_pct),
                        format_optional_fixed(candidate.breakeven_zscore, 3),
                        (config.risk_free_rate - config.dividend_yield) * 100.0,
                        candidate.volatility_used * 100.0
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Stats ordre ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "EV risk-neutral {}  Sharpe-like {}  Kelly binaire {}",
                        format_optional_signed_pct(candidate.expected_value_pct),
                        format_optional_fixed(candidate.sharpe_like, 3),
                        format_optional_plain_pct(candidate.kelly_fraction_pct)
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Greeks/warrant ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "delta {}  gamma {}  vega/pt {}  theta/j {}  rho/1% {}  d1 {}  d2 {}",
                        format_optional_fixed(candidate.delta, 5),
                        format_optional_fixed(candidate.gamma, 6),
                        format_optional_fixed(candidate.vega_per_vol_point, 5),
                        format_optional_fixed(candidate.theta_per_day, 5),
                        format_optional_fixed(candidate.rho_per_rate_point, 5),
                        format_optional_fixed(candidate.d1, 3),
                        format_optional_fixed(candidate.d2, 3)
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Breakeven ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "{}  distance {}",
                        candidate
                            .breakeven_underlying
                            .map(|value| format!("{value:.4}"))
                            .unwrap_or_else(|| "-".to_string()),
                        candidate
                            .breakeven_distance_pct
                            .map(|value| format!("{value:+.2}%"))
                            .unwrap_or_else(|| "-".to_string())
                    )),
                    Span::raw("  "),
                    Span::styled("Parity ", Style::default().fg(Color::Yellow)),
                    Span::raw(format_compact_float(candidate.parity)),
                ]),
                Line::from(vec![
                    Span::styled("Options ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "IV {}  vol_used {:.2}%  thetaH/d {}  spread {}",
                        format_optional_factor_pct(candidate.implied_volatility),
                        candidate.volatility_used * 100.0,
                        candidate
                            .theta_horizon_pct
                            .map(|value| format!("{value:.4}%"))
                            .unwrap_or_else(|| "-".to_string()),
                        format_optional_plain_pct(candidate.spread_pct)
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Data ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "quality {:.0}/100  flow {:.0}/100 (informatif, non decisif)",
                        candidate.data_quality_score, candidate.liquidity_score
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Pourquoi ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw(human_reasons(&candidate.reasons)),
                ]),
                Line::from(vec![
                    Span::styled("URL ", Style::default().fg(Color::Yellow)),
                    Span::styled(candidate.url.clone(), Style::default().fg(Color::Cyan)),
                ]),
                Line::from(vec![
                    Span::styled("Action ", Style::default().fg(Color::Yellow)),
                    Span::raw(status_message.to_string()),
                ]),
                Line::from(format!(
                    "Calc scenario: Black-Scholes({}, S_target={:.4}, K={:.4}, T cible->maturite, vol={:.2}%, r={:.2}%, q={:.2}%) / parity {}",
                    candidate.side,
                    config.target_price,
                    candidate.strike,
                    candidate.volatility_used * 100.0,
                    config.risk_free_rate * 100.0,
                    config.dividend_yield * 100.0,
                    format_compact_float(candidate.parity)
                )),
                Line::from(format!(
                    "Calc return: ({:.4} - {:.4}) / {:.4} * 100 = {:+.2}% brut; net frais = {:+.2}%",
                    candidate.projected_price,
                    candidate.entry_price,
                    candidate.entry_price,
                    candidate.gross_return_pct,
                    candidate.net_return_pct
                )),
                Line::from(format!(
                    "Calc proba: z=(ln(level/spot)-(r-q-0.5*vol^2)*T)/(vol*sqrt(T)); call=1-Phi(z), put=Phi(z). P(target)={} P(BE)={}",
                    format_optional_plain_pct(candidate.probability_target_pct),
                    format_optional_plain_pct(candidate.probability_breakeven_pct)
                )),
                Line::from(format!(
                    "Calc stats: EV=risk-neutral model value at horizon vs entry; Sharpe-like=E[R]/std(R); Kelly target=max(0,(b*p-q)/b), b=gain_target, p=P(target), q=1-p, perte binaire -100%; p requis {}",
                    kelly_required_probability(candidate)
                )),
            ]
        })
        .unwrap_or_else(|| {
            vec![
                Line::from("Aucun warrant ne correspond au scenario avec ce filtre."),
                Line::from(
                    "Essaie d'elargir la maturite, d'augmenter la limite, ou de verifier que Boursorama renvoie un bid/ask executable.",
                ),
                Line::from(status_message.to_string()),
            ]
        })
    };

    let block = Block::default()
        .title(format!(
            " {}  scroll {}  <-/-> ligne  Tab/Shift+Tab page  t/e haut/bas ",
            detail_tab.title(),
            notes_scroll
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let max_scroll = visual_line_count(&lines, area.width.saturating_sub(2))
        .saturating_sub(area.height.saturating_sub(2) as usize) as u16;
    let scroll = notes_scroll.min(max_scroll);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .scroll((scroll, 0))
            .block(block),
        area,
    );
}

fn visual_line_count(lines: &[Line], width: u16) -> usize {
    let width = width.max(1) as usize;
    lines
        .iter()
        .map(|line| {
            let chars = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum::<usize>();
            chars.max(1).div_ceil(width)
        })
        .sum()
}

fn llm_lines(
    candidates: &[&ScenarioCandidate],
    selected: Option<usize>,
    llm_reviews: &HashMap<String, String>,
    status_message: &str,
) -> Vec<Line<'static>> {
    let Some(candidate) = selected.and_then(|index| candidates.get(index)) else {
        return vec![
            Line::from("Aucun produit selectionne."),
            Line::from("Selectionne un warrant puis appuie sur r pour interroger Gemini."),
        ];
    };

    if let Some(review) = llm_reviews.get(&candidate.symbol) {
        return review.lines().map(|line| Line::from(line.to_string())).collect();
    }

    vec![
        Line::from(vec![
            Span::styled("LLM Gemini ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("pret pour "),
            Span::styled(candidate.symbol.clone(), Style::default().fg(Color::Cyan)),
        ]),
        Line::from("Appuie sur r pour demander un audit Gemini du candidat selectionne."),
        Line::from("Le contexte envoye contient le scenario, les prix, les probas, les greeks, le flow informatif, les frais et les raisons BUY/WATCH/AVOID."),
        Line::from("Variables .env: GEMINI_API_KEY, GEMINI_MODEL, LLM_ENABLE, LLM_CACHE."),
        Line::from(format!("Action {status_message}")),
    ]
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DetailTab {
    Notes,
    Llm,
}

impl DetailTab {
    fn toggle(self) -> Self {
        match self {
            Self::Notes => Self::Llm,
            Self::Llm => Self::Notes,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Notes => "Notes",
            Self::Llm => "LLM Gemini",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DecisionFilter {
    All,
    Buy,
    Watch,
    Avoid,
}

impl DecisionFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Buy => "buy",
            Self::Watch => "watch",
            Self::Avoid => "avoid",
        }
    }

    fn accepts(self, candidate: &ScenarioCandidate) -> bool {
        match self {
            Self::All => true,
            Self::Buy => candidate.decision == "BUY",
            Self::Watch => candidate.decision == "WATCH",
            Self::Avoid => candidate.decision == "AVOID",
        }
    }
}

fn filter_candidates(
    candidates: &[ScenarioCandidate],
    decision_filter: DecisionFilter,
) -> Vec<&ScenarioCandidate> {
    candidates
        .iter()
        .filter(|candidate| decision_filter.accepts(candidate))
        .collect()
}

fn resolve_scenario_side(requested_side: &str, target_price: f64, spot: f64) -> &'static str {
    match requested_side {
        "force-call" | "force-calls" => "call",
        "force-put" | "force-puts" => "put",
        _ if target_price >= spot => "call",
        _ => "put",
    }
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

fn select_first(state: &mut TableState, len: usize) {
    state.select((len > 0).then_some(0));
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
    let next = state.selected().map_or(0, |index| index.saturating_sub(1));
    state.select(Some(next));
}

fn select_last(state: &mut TableState, len: usize) {
    state.select((len > 0).then_some(len - 1));
}

fn selected_action_message(selected: Option<usize>, candidates: &[&ScenarioCandidate]) -> String {
    selected
        .and_then(|index| candidates.get(index))
        .map(|candidate| {
            let open_status = match open_url(&candidate.url) {
                Ok(()) => "ouverture demandee".to_string(),
                Err(err) => format!("ouverture impossible depuis cet environnement ({err})"),
            };
            format!("{open_status} | URL: {}", candidate.url)
        })
        .unwrap_or_else(|| "Aucun produit selectionne.".to_string())
}

fn selected_symbol(selected: Option<usize>, candidates: &[&ScenarioCandidate]) -> Option<String> {
    selected
        .and_then(|index| candidates.get(index))
        .map(|candidate| candidate.symbol.clone())
}

fn selected_llm_message(
    selected: Option<usize>,
    candidates: &[&ScenarioCandidate],
    underlying: &WarrantSnapshot,
    config: &ScenarioConfig,
    llm_config: &GeminiConfig,
    client: &reqwest::Client,
    llm_reviews: &mut HashMap<String, String>,
) -> String {
    let Some(candidate) = selected.and_then(|index| candidates.get(index)) else {
        return "Aucun produit selectionne pour Gemini.".to_string();
    };
    if let Some(existing) = llm_reviews.get(&candidate.symbol) {
        if !is_llm_in_progress(existing) {
            return format!(
                "Analyse Gemini deja disponible pour {} (aucune nouvelle requete).",
                candidate.symbol
            );
        }
    }

    match tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(analyze_scenario_candidate(
            client,
            llm_config,
            underlying,
            config,
            candidate,
        ))
    }) {
        Ok(review) => {
            llm_reviews.insert(candidate.symbol.clone(), format_llm_review(llm_config, &review));
            format!("Gemini OK pour {}", candidate.symbol)
        }
        Err(err) => {
            let message = format!("Gemini indisponible: {err:#}");
            llm_reviews.insert(
                candidate.symbol.clone(),
                format!(
                    "Gemini indisponible pour {}\n\n{}\n\nConfig attendue dans .env:\nGEMINI_API_KEY=...\nGEMINI_MODEL=gemini-3.1-pro-preview\nLLM_ENABLE=1\nLLM_CACHE=1",
                    candidate.symbol, message
                ),
            );
            message
        }
    }
}

fn llm_in_progress_message(symbol: &str) -> String {
    format!(
        "Analyse Gemini en cours pour {symbol}...\n\nLa requete est deja lancee. Les appuis repetes sur r ne relanceront pas d'appel pour ce candidat tant que le resultat est memorise."
    )
}

fn is_llm_in_progress(review: &str) -> bool {
    review.starts_with("Analyse Gemini en cours")
}

fn format_llm_review(config: &GeminiConfig, review: &LlmScenarioReview) -> String {
    let risks = if review.main_risks.is_empty() {
        "-".to_string()
    } else {
        review.main_risks.join(" | ")
    };
    let data_issues = if review.data_issues.is_empty() {
        "-".to_string()
    } else {
        review.data_issues.join(" | ")
    };
    let questions = if review.questions_before_entry.is_empty() {
        "-".to_string()
    } else {
        review.questions_before_entry.join(" | ")
    };

    format!(
        "Modele Gemini: {}\nVerdict LLM: {}  confiance {}/100\n\nResume\n{}\n\nRisques principaux\n{}\n\nProblemes de donnees\n{}\n\nReview du plan\n{}\n\nQuestions avant entree\n{}\n\nNote: avis qualitatif uniquement; le moteur quantitatif reste la source des calculs.",
        config.model,
        review.verdict,
        review.confidence,
        review.summary,
        risks,
        data_issues,
        review.trade_plan_review,
        questions
    )
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

fn colored_change(change_pct: f64) -> Span<'static> {
    let color = if change_pct >= 0.0 { Color::Green } else { Color::Red };
    Span::styled(
        format!("{change_pct:+.2}%"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn target_style(value: f64) -> Style {
    if value >= 0.0 {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    }
}

fn decision_style(decision: &str) -> Style {
    match decision {
        "BUY" => Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        "WATCH" => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

fn side_style(side: &str) -> Style {
    match side {
        "call" => Style::default().fg(Color::Green),
        "put" => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::DarkGray),
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

fn return_style(return_pct: f64) -> Style {
    if return_pct >= 20.0 {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else if return_pct > 0.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red)
    }
}

fn format_compact_float(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.4}")
    }
}

fn format_optional_plain_pct(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}%"))
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_signed_pct(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.2}%"))
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_factor_pct(value: Option<f64>) -> String {
    value
        .map(|value| format!("{:.2}%", value * 100.0))
        .unwrap_or_else(|| "-".to_string())
}

fn format_optional_fixed(value: Option<f64>, decimals: usize) -> String {
    value
        .map(|value| format!("{value:.decimals$}"))
        .unwrap_or_else(|| "-".to_string())
}

fn kelly_required_probability(candidate: &ScenarioCandidate) -> String {
    if candidate.net_return_pct <= 0.0 {
        return "-".to_string();
    }
    let gain = candidate.net_return_pct / 100.0;
    let required = 1.0 / (1.0 + gain);
    format!("{:.2}%", required * 100.0)
}

fn human_reasons(reasons: &[String]) -> String {
    if reasons.is_empty() {
        return "conditions du scenario remplies".to_string();
    }

    reasons
        .iter()
        .map(|reason| match reason.as_str() {
            "scenario_return_negative" => "rendement net negatif au target",
            "entry_price_not_executable" => "prix d'entree non executable: bid/ask absent ou invalide",
            "maturity_before_preferred_window" => "maturite avant la fenetre souhaitee",
            "maturity_after_preferred_window" => "maturite apres la fenetre souhaitee",
            "data_quality_too_low" => "qualite de donnee trop faible",
            "liquidity_too_low" => "liquidite trop faible",
            "spread_too_wide" => "spread trop large",
            "breakeven_beyond_target" => "breakeven au-dela du target",
            _ => "raison non documentee",
        })
        .collect::<Vec<_>>()
        .join(" | ")
}
