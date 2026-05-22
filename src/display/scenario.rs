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
    symbols,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table, TableState, Wrap},
    Frame, Terminal,
};

use crate::decision::config::DecisionConfig;
use crate::decision::scenario::{analyze_warrant_scenario, ScenarioCandidate, ScenarioConfig};
use crate::indicators::options::{black_scholes_price, BlackScholesInput, OptionKind};
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
                decision_config,
                today,
            )
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if let Some(input) = target_edit.as_mut() {
                    match key.code {
                        KeyCode::Enter => {
                            let parsed = parse_target_edit(input);
                            match parsed {
                                Some((target, min, max)) if target > 0.0 => {
                                    drop(visible);
                                    config.target_price = target;
                                    config.target_price_min = min;
                                    config.target_price_max = max;
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
                                        "Cible modifiee a {}; scenario recalcule en {} avec {} candidat(s).",
                                        scenario_target_label(&config, underlying.price),
                                        config.side,
                                        candidates.len()
                                    );
                                    target_edit = None;
                                    continue;
                                }
                                _ => {
                                    status_message =
                                        format!("Cible invalide: '{input}'. Exemple: 1300, 1700.5 ou 365..375");
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
                            if ch.is_ascii_digit() || matches!(ch, '.' | ',' | '_' | ':' | ';') =>
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
                        target_edit = Some(match (config.target_price_min, config.target_price_max) {
                            (Some(low), Some(high)) if high > low => {
                                format!("{}..{}", format_compact_float(low), format_compact_float(high))
                            }
                            _ => format_compact_float(config.target_price),
                        });
                        status_message = "Edition cible: tape une valeur ou une range 365..375, Enter valide, Esc annule."
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
                                        decision_config,
                                        today,
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
    decision_config: &DecisionConfig,
    today: NaiveDate,
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
    draw_table(frame, chunks[1], underlying, candidates, state, decision_config);
    draw_details(
        frame,
        chunks[2],
        underlying,
        candidates,
        state.selected(),
        config,
        decision_config,
        notes_scroll,
        detail_tab,
        llm_reviews,
        status_message,
        today,
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
    let target_text = scenario_target_label(config, underlying.price);

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
                    .unwrap_or_else(|| format!("{target_text} au {} ({:+.2}%)", config.target_date, target_move)),
                target_edit
                    .map(|_| Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))
                    .unwrap_or_else(|| target_style(target_move)),
            ),
        ]),
        Line::from(vec![
            Span::styled("These ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                side_label(config, underlying.price),
                side_style(&config.side),
            ),
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
            " Scenario products ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
}

fn draw_table(
    frame: &mut Frame,
    area: Rect,
    underlying: &WarrantSnapshot,
    candidates: &[&ScenarioCandidate],
    state: &mut TableState,
    decision_config: &DecisionConfig,
) {
    let header = Row::new([
        "Dec", "Score", "Net@D", "P/L@D", "Str@D", "BE mv", "Tch<=D", "KO%", "MCev",
        "Spr", "Symb", "Type", "Strike", "Maturite", "Par", "EntryAsk", "ExitBid@D",
        "IVout", "DQ",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(1);

    let rows = candidates.iter().map(|candidate| {
        let displayed_net_pct = scenario_display_net_pct(candidate);
        let displayed_stress_pct = candidate
            .target_range_stress_min_pct
            .or(candidate.stressed_net_return_pct);
        Row::new(vec![
            Cell::from(candidate.decision).style(decision_style(candidate.decision)),
            Cell::from(format!("{:.0}", candidate.score)).style(score_style(candidate.score)),
            Cell::from(format!("{:+.1}%", displayed_net_pct))
                .style(return_style(displayed_net_pct)),
            Cell::from(format_money_table_from_pct(
                displayed_net_pct,
                decision_config.fee_order_notional,
            ))
            .style(return_style(displayed_net_pct)),
            Cell::from(format_optional_signed_pct(displayed_stress_pct))
                .style(optional_return_style(displayed_stress_pct)),
            Cell::from(breakeven_move_cell(candidate, underlying.price))
                .style(breakeven_move_style(candidate, underlying.price)),
            Cell::from(format_optional_plain_pct(candidate.probability_target_pct)),
            Cell::from(format_optional_plain_pct(candidate.barrier_touch_probability_pct))
                .style(optional_risk_style(candidate.barrier_touch_probability_pct)),
            Cell::from(format_optional_signed_pct(candidate.monte_carlo_expected_return_pct))
                .style(optional_return_style(candidate.monte_carlo_expected_return_pct)),
            Cell::from(format_optional_plain_pct(candidate.spread_pct)),
            Cell::from(candidate.symbol.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(product_kind_label(candidate))
                .style(product_kind_style(candidate)),
            Cell::from(format_contract_level(candidate.strike))
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Cell::from(candidate.maturity_label.clone())
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Cell::from(format_compact_float(candidate.parity))
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Cell::from(format!("{:.4}", candidate.entry_price)),
            Cell::from(format!("{:.4}", candidate.projected_bid_price)),
            Cell::from(exit_volatility_cell(candidate)),
            Cell::from(format!("{:.0}", candidate.data_quality_score))
                .style(score_style(candidate.data_quality_score)),
        ])
    });

    let widths = [
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(4),
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
    underlying: &WarrantSnapshot,
    candidates: &[&ScenarioCandidate],
    selected: Option<usize>,
    config: &ScenarioConfig,
    decision_config: &DecisionConfig,
    notes_scroll: u16,
    detail_tab: DetailTab,
    llm_reviews: &HashMap<String, String>,
    status_message: &str,
    today: NaiveDate,
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
                        "{} target {} au {}, strike {:.4}, maturite {}",
                        candidate.side,
                        scenario_target_label(config, underlying.price),
                        config.target_date,
                        candidate.strike,
                        candidate.maturity_label
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Projection ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "entry ask {:.4} -> sortie bid estimee a la date cible {:.4} (modele {:.4}, hors frais); gross {:+.2}% -> net {:+.2}% ({}) apres frais {:.2}%",
                        candidate.entry_price,
                        candidate.projected_bid_price,
                        candidate.projected_price,
                        candidate.gross_return_pct,
                        candidate.net_return_pct,
                        format_money_from_pct(
                            candidate.net_return_pct,
                            decision_config.fee_order_notional,
                        ),
                        candidate.fee_drag_pct
                    )),
                ]),
                stress_line(candidate, decision_config),
                Line::from(vec![
                    Span::styled("Range cible ", Style::default().fg(Color::Yellow)),
                    Span::raw(target_range_sentence(candidate, decision_config)),
                ]),
                Line::from(vec![
                    Span::styled("Montant ", Style::default().fg(Color::Yellow)),
                    Span::raw(money_plan_sentence(candidate, underlying, config, decision_config, today)),
                ]),
                Line::from(vec![
                    Span::styled("Diagnostic ", Style::default().fg(Color::Yellow)),
                    Span::raw(target_vs_breakeven_sentence(candidate, underlying.price, config)),
                ]),
                Line::from(vec![
                    Span::styled("Point mort target ", Style::default().fg(Color::Yellow)),
                    Span::raw(breakeven_sentence(candidate, underlying.price)),
                ]),
                Line::from(vec![
                    Span::styled("Plan sortie ", Style::default().fg(Color::Yellow)),
                    Span::raw(exit_plan_sentence(candidate, underlying, config, decision_config, today)),
                ]),
                Line::from(vec![
                    Span::styled("Probabilites ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "FirstTouch reel: P(target) {}  P(BE) {}; Terminal Q: P(target) {} zTarget {}  P(BE) {} zBE {}; drift reel {:.2}% vs r-q {:.2}%",
                        format_optional_plain_pct(candidate.probability_target_pct),
                        format_optional_plain_pct(candidate.probability_breakeven_pct),
                        format_optional_plain_pct(candidate.terminal_probability_target_pct),
                        format_optional_fixed(candidate.target_zscore, 3),
                        format_optional_plain_pct(candidate.terminal_probability_breakeven_pct),
                        format_optional_fixed(candidate.breakeven_zscore, 3),
                        config.real_world_drift * 100.0,
                        (config.risk_free_rate - config.dividend_yield) * 100.0,
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Stats ordre ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "Fair-value Q {}  Sharpe-like TP/SL {}  Kelly TP/SL borne {}",
                        format_optional_signed_pct(candidate.expected_value_pct),
                        format_optional_fixed(candidate.sharpe_like, 3),
                        format_optional_plain_pct(candidate.kelly_fraction_pct)
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Greeks/warrant ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "entree delta {} gamma {} vega/pt {} theta/j {}; cible delta {} gamma {} vega/pt {} theta/j {}",
                        format_optional_fixed(candidate.delta, 5),
                        format_optional_fixed(candidate.gamma, 6),
                        format_optional_fixed(candidate.vega_per_vol_point, 5),
                        format_optional_fixed(candidate.theta_per_day, 5),
                        format_optional_fixed(candidate.target_delta, 5),
                        format_optional_fixed(candidate.target_gamma, 6),
                        format_optional_fixed(candidate.target_vega_per_vol_point, 5),
                        format_optional_fixed(candidate.target_theta_per_day, 5)
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
                    Span::raw(options_sentence(candidate, config)),
                ]),
                Line::from(vec![
                    Span::styled("Fin/KO ", Style::default().fg(Color::Yellow)),
                    Span::raw(financing_and_barrier_sentence(candidate)),
                ]),
                Line::from(vec![
                    Span::styled("Monte Carlo ", Style::default().fg(Color::Yellow)),
                    Span::raw(monte_carlo_sentence(candidate)),
                ]),
                Line::from(vec![
                    Span::styled("FX ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "entree {:.4}, sortie {:.4}, stress {}; si USD->EUR baisse, le warrant EUR est ampute meme si le spot USD monte.",
                        candidate.fx_rate,
                        candidate.fx_exit_rate,
                        candidate
                            .fx_stressed_exit_rate
                            .map(|value| format!("{value:.4}"))
                            .unwrap_or_else(|| "inactif".to_string())
                    )),
                ]),
                Line::from(vec![
                    Span::styled("Dividendes ", Style::default().fg(Color::Yellow)),
                    Span::raw(format!(
                        "PV discrete cible->maturite {:.4}; {}",
                        candidate.discrete_dividend_pv,
                        if config.dividends.is_empty() {
                            "aucun calendrier fourni, q continu utilise"
                        } else {
                            "calendrier discret applique, q force a 0 dans la projection"
                        }
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
                    Span::styled("Warnings ", Style::default().fg(Color::Yellow)),
                    Span::raw(human_warnings(&candidate.warnings)),
                ]),
                Line::from(vec![
                    Span::styled("URL ", Style::default().fg(Color::Yellow)),
                    Span::styled(candidate.url.clone(), Style::default().fg(Color::Cyan)),
                ]),
                Line::from(vec![
                    Span::styled("Action ", Style::default().fg(Color::Yellow)),
                    Span::raw(status_message.to_string()),
                ]),
                Line::from(format_exit_formula(candidate, config)),
                Line::from(format!(
                    "Calc return@date: ({:.4} - {:.4}) / {:.4} * 100 = {:+.2}% brut; net apres frais broker = {:+.2}%",
                    candidate.projected_bid_price,
                    candidate.entry_price,
                    candidate.entry_price,
                    candidate.gross_return_pct,
                    candidate.net_return_pct
                )),
                Line::from(format!(
                    "Calc proba: Tch<=D est une proba first-touch avant la date cible, pas le prix de sortie. ExitBid@D reste calcule exactement a la date cible. Touch target={} Touch BE={} | Terminal target={} Terminal BE={}",
                    format_optional_plain_pct(candidate.probability_target_pct),
                    format_optional_plain_pct(candidate.probability_breakeven_pct),
                    format_optional_plain_pct(candidate.terminal_probability_target_pct),
                    format_optional_plain_pct(candidate.terminal_probability_breakeven_pct)
                )),
                Line::from(format!(
                    "Calc stats: Fair-value Q=valeur modele risk-neutral portee a l'horizon vs entree; ce n'est pas une vraie EV monde reel. Kelly TP/SL=max(0,(b*p-q)/b), b=gain_target/stop_loss, p=FirstTouch target, q=1-p; p requis {}",
                    kelly_required_probability(candidate, decision_config)
                )),
            ]
        })
        .unwrap_or_else(|| {
            no_candidate_lines(config, underlying, status_message)
        })
    };

    if detail_tab == DetailTab::Notes && area.width >= 110 && area.height >= 12 {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        draw_notes(frame, chunks[0], lines, notes_scroll, detail_tab);
        draw_payoff_chart(
            frame,
            chunks[1],
            selected.and_then(|index| candidates.get(index)).copied(),
            underlying,
            config,
            decision_config,
            today,
        );
        return;
    }

    draw_notes(frame, area, lines, notes_scroll, detail_tab);
}

fn draw_notes(
    frame: &mut Frame,
    area: Rect,
    lines: Vec<Line<'static>>,
    notes_scroll: u16,
    detail_tab: DetailTab,
) {
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

fn draw_payoff_chart(
    frame: &mut Frame,
    area: Rect,
    candidate: Option<&ScenarioCandidate>,
    underlying: &WarrantSnapshot,
    config: &ScenarioConfig,
    decision_config: &DecisionConfig,
    today: NaiveDate,
) {
    let Some(candidate) = candidate else {
        frame.render_widget(
            Paragraph::new("Aucun produit selectionne.")
                .block(Block::default().title(" Decision line ").borders(Borders::ALL)),
            area,
        );
        return;
    };
    let Some(data) = build_payoff_chart(candidate, underlying, config, decision_config, today) else {
        frame.render_widget(
            Paragraph::new("Decision line indisponible: donnees insuffisantes.")
                .block(Block::default().title(" Decision line ").borders(Borders::ALL)),
            area,
        );
        return;
    };

    let mut datasets = vec![Dataset::default()
        .name("plan lineaire")
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Cyan))
        .data(&data.plan_line)];
    let spot_bar = vertical_marker_line(0.0, data.y_bounds);
    datasets.push(
        Dataset::default()
            .name("S entree")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
            .data(&spot_bar),
    );
    let target_bar = vertical_marker_line(data.target_move_pct, data.y_bounds);
    datasets.push(
        Dataset::default()
            .name("T target")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            .data(&target_bar),
    );
    let mut be_bar = None;
    if let Some(be_move) = data.breakeven_move_pct {
        be_bar = Some(vertical_marker_line(be_move, data.y_bounds));
    }
    if let Some(points) = be_bar.as_ref() {
        datasets.push(
            Dataset::default()
                .name("B breakeven")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                .data(points),
        );
    }
    let mut stop_bar = None;
    if let Some(stop_move) = data.stop_move_pct {
        stop_bar = Some(vertical_marker_line(stop_move, data.y_bounds));
    }
    if let Some(points) = stop_bar.as_ref() {
        datasets.push(
            Dataset::default()
                .name("X stop")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                .data(points),
        );
    }

    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .title(format!(
                    " Plan {} | S 0/0 | X {} | B {} | T {} ",
                    candidate.symbol,
                    format_optional_signed_pct(data.stop_move_pct),
                    breakeven_move_cell(candidate, underlying.price),
                    format_signed_pct(data.target_net_pct)
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .x_axis(
            Axis::default()
                .title("mouvement sous-jacent %")
                .style(Style::default().fg(Color::DarkGray))
                .bounds(data.x_bounds)
                .labels(vec![
                    Span::raw(format!("{:.0}", data.x_bounds[0])),
                    Span::styled("S", Style::default().fg(Color::White)),
                    Span::raw(format!("{:.0}", data.x_bounds[1])),
                ]),
        )
        .y_axis(
            Axis::default()
                .title("P/L net %")
                .style(Style::default().fg(Color::DarkGray))
                .bounds(data.y_bounds)
                .labels(vec![
                    Span::raw(format!("{:.0}", data.y_bounds[0])),
                    Span::styled("0", Style::default().fg(Color::White)),
                    Span::raw(format!("{:.0}", data.y_bounds[1])),
                ]),
        );
    frame.render_widget(
        chart,
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

struct PayoffChartData {
    plan_line: Vec<(f64, f64)>,
    target_move_pct: f64,
    breakeven_move_pct: Option<f64>,
    target_net_pct: f64,
    stop_move_pct: Option<f64>,
    stop_net_pct: Option<f64>,
    x_bounds: [f64; 2],
    y_bounds: [f64; 2],
}

fn build_payoff_chart(
    candidate: &ScenarioCandidate,
    underlying: &WarrantSnapshot,
    config: &ScenarioConfig,
    decision_config: &DecisionConfig,
    today: NaiveDate,
) -> Option<PayoffChartData> {
    let spot = underlying.price;
    if spot <= 0.0 || candidate.entry_price <= 0.0 || candidate.parity <= 0.0 {
        return None;
    }
    years_between(config.target_date, candidate.maturity)?;
    let kind = option_kind(&candidate.side)?;
    let target_move = signed_move_pct(spot, config.target_price);
    let breakeven_move = candidate
        .breakeven_underlying
        .map(|level| signed_move_pct(spot, level));
    let mut x_min = -20.0_f64.min(target_move - 3.0);
    let mut x_max = 20.0_f64.max(target_move + 3.0);
    if let Some(be) = breakeven_move {
        x_min = x_min.min(be - 3.0);
        x_max = x_max.max(be + 3.0);
    }
    x_min = x_min.min(-decision_config.max_loss_pct_per_trade.abs() / 2.0);
    x_max = x_max.max(decision_config.max_loss_pct_per_trade.abs() / 2.0);
    x_min = x_min.max(-70.0);
    x_max = x_max.min(70.0);
    if x_max - x_min < 10.0 {
        x_min -= 5.0;
        x_max += 5.0;
    }

    let mut current_curve = Vec::with_capacity(61);
    let years_to_maturity = years_between(today, candidate.maturity)?;
    for index in 0..=60 {
        let move_pct = x_min + (x_max - x_min) * index as f64 / 60.0;
        let level = spot * (1.0 + move_pct / 100.0);
        let current_dividend_pv =
            present_value_dividends(today, candidate.maturity, config.risk_free_rate, config);
        let current_exit_price = scenario_price_at_level(
            kind,
            (level - current_dividend_pv).max(0.01),
            candidate,
            years_to_maturity,
            config.risk_free_rate,
            scenario_dividend_yield(config),
            candidate.volatility_used,
            candidate.fx_rate,
        )?;
        let current_exit_price = apply_exit_spread_penalty(
            current_exit_price,
            candidate.spread_pct,
            candidate.effective_exit_spread_multiplier,
        );
        let current_net =
            net_return_after_fees(candidate.entry_price, current_exit_price, decision_config)?;
        current_curve.push((move_pct, current_net));
    }

    let target_net = net_return_after_fees(
        candidate.entry_price,
        candidate.projected_bid_price,
        decision_config,
    )?;
    let stressed_target_net = candidate.stressed_projected_price.and_then(|price| {
        net_return_after_fees(candidate.entry_price, price, decision_config)
    });
    let stop_loss_pct = -decision_config.max_loss_pct_per_trade.abs();
    let stop_move_pct = find_stop_move(&current_curve, kind, stop_loss_pct);
    let plan_line = straight_plan_line(x_min, x_max, target_move, target_net);
    let mut y_min = plan_line
        .iter()
        .map(|(_, y)| *y)
        .fold(0.0_f64, f64::min)
        .min(stop_loss_pct);
    let mut y_max = plan_line
        .iter()
        .map(|(_, y)| *y)
        .fold(0.0_f64, f64::max)
        .max(target_net);
    if let Some(value) = stressed_target_net {
        y_min = y_min.min(value);
        y_max = y_max.max(value);
    }
    let y_padding = ((y_max - y_min).abs() * 0.15).max(5.0);
    y_min = (y_min - y_padding).max(-100.0);
    y_max += y_padding;
    if y_max - y_min < 10.0 {
        y_min -= 5.0;
        y_max += 5.0;
    }

    Some(PayoffChartData {
        plan_line,
        target_move_pct: target_move,
        breakeven_move_pct: breakeven_move,
        target_net_pct: target_net,
        stop_move_pct,
        stop_net_pct: stop_move_pct.map(|_| stop_loss_pct),
        x_bounds: [x_min, x_max],
        y_bounds: [y_min, y_max],
    })
}

fn straight_plan_line(x_min: f64, x_max: f64, target_move_pct: f64, target_net_pct: f64) -> Vec<(f64, f64)> {
    if target_move_pct.abs() < f64::EPSILON {
        return vec![(x_min, 0.0), (x_max, 0.0)];
    }
    let slope = target_net_pct / target_move_pct;
    (0..=40)
        .map(|index| {
            let x = x_min + (x_max - x_min) * index as f64 / 40.0;
            (x, slope * x)
        })
        .collect()
}

fn vertical_marker_line(x: f64, y_bounds: [f64; 2]) -> Vec<(f64, f64)> {
    (0..=24)
        .map(|index| {
            let y = y_bounds[0] + (y_bounds[1] - y_bounds[0]) * index as f64 / 24.0;
            (x, y)
        })
        .collect()
}

fn find_stop_move(curve: &[(f64, f64)], kind: OptionKind, stop_net_pct: f64) -> Option<f64> {
    let mut adverse_points = curve
        .iter()
        .copied()
        .filter(|(x, _)| match kind {
            OptionKind::Call => *x <= 0.0,
            OptionKind::Put => *x >= 0.0,
        })
        .collect::<Vec<_>>();
    match kind {
        OptionKind::Call => adverse_points.sort_by(|left, right| right.0.total_cmp(&left.0)),
        OptionKind::Put => adverse_points.sort_by(|left, right| left.0.total_cmp(&right.0)),
    }

    let mut previous: Option<(f64, f64)> = None;
    for point in adverse_points {
        if point.1 <= stop_net_pct {
            if let Some(previous) = previous {
                return Some(interpolate_x_for_y(previous, point, stop_net_pct));
            }
            return Some(point.0);
        }
        if let Some(previous) = previous {
            let prev_distance = previous.1 - stop_net_pct;
            let point_distance = point.1 - stop_net_pct;
            if prev_distance * point_distance <= 0.0 {
                return Some(interpolate_x_for_y(previous, point, stop_net_pct));
            }
        }
        previous = Some(point);
    }
    None
}

fn interpolate_x_for_y(left: (f64, f64), right: (f64, f64), target_y: f64) -> f64 {
    let dy = right.1 - left.1;
    if dy.abs() < f64::EPSILON {
        return right.0;
    }
    let ratio = (target_y - left.1) / dy;
    left.0 + (right.0 - left.0) * ratio
}

fn scenario_price_at_level(
    kind: OptionKind,
    level: f64,
    candidate: &ScenarioCandidate,
    years_remaining: f64,
    risk_free_rate: f64,
    dividend_yield: f64,
    volatility: f64,
    fx_rate: f64,
) -> Option<f64> {
    if candidate.pricing_model != "warrant_intrinsic" {
        let intrinsic = match kind {
            OptionKind::Call => (level - candidate.strike).max(0.0),
            OptionKind::Put => (candidate.strike - level).max(0.0),
        };
        return Some(intrinsic / candidate.parity * fx_rate);
    }
    let price = black_scholes_price(BlackScholesInput {
        kind,
        spot: level,
        strike: candidate.strike,
        years_to_maturity: years_remaining,
        risk_free_rate,
        dividend_yield,
        volatility,
    })?;
    Some(price / candidate.parity * fx_rate)
}

fn apply_exit_spread_penalty(price: f64, spread_pct: Option<f64>, multiplier: f64) -> f64 {
    let spread = spread_pct.unwrap_or(0.0).max(0.0);
    (price * (1.0 - spread * multiplier.max(0.0) / 100.0)).max(0.0)
}

fn scenario_dividend_yield(config: &ScenarioConfig) -> f64 {
    if config.dividends.is_empty() {
        config.dividend_yield
    } else {
        0.0
    }
}

fn present_value_dividends(
    from_date: NaiveDate,
    to_date: NaiveDate,
    risk_free_rate: f64,
    config: &ScenarioConfig,
) -> f64 {
    config
        .dividends
        .iter()
        .filter(|dividend| {
            dividend.amount > 0.0
                && dividend.ex_date > from_date
                && dividend.ex_date <= to_date
        })
        .filter_map(|dividend| {
            years_between(from_date, dividend.ex_date)
                .map(|years| dividend.amount * (-risk_free_rate * years).exp())
        })
        .sum()
}

fn net_return_after_fees(
    entry_price: f64,
    exit_price: f64,
    config: &DecisionConfig,
) -> Option<f64> {
    if entry_price <= 0.0 || exit_price < 0.0 || config.fee_order_notional <= 0.0 {
        return None;
    }
    let notional = config.fee_order_notional;
    let buy_fee = config.fee_buy_fixed + notional * config.fee_buy_pct / 100.0;
    let deposit_fee = config.fee_deposit_fixed + notional * config.fee_deposit_pct / 100.0;
    let exit_notional = notional * exit_price / entry_price;
    let sell_fee = config.fee_sell_fixed + exit_notional * config.fee_sell_pct / 100.0;
    let cost_basis = notional + buy_fee + deposit_fee;
    if cost_basis <= 0.0 {
        return None;
    }
    Some(((exit_notional - sell_fee) - cost_basis) / cost_basis * 100.0)
}

fn years_between(start: NaiveDate, end: NaiveDate) -> Option<f64> {
    let days = (end - start).num_days();
    (days > 0).then_some(days as f64 / 365.0)
}

fn option_kind(side: &str) -> Option<OptionKind> {
    match side {
        "call" => Some(OptionKind::Call),
        "put" => Some(OptionKind::Put),
        _ => None,
    }
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
        Line::from("Le contexte envoye contient les champs decisionnels: net, stress IV/FX, BE, first-touch, bid/ask, stale pricing, dividendes, greeks projetes et raisons."),
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

fn resolve_scenario_side(requested_side: &str, _target_price: f64, _spot: f64) -> &'static str {
    match requested_side {
        "call" | "calls" | "force-call" | "force-calls" => "call",
        "put" | "puts" | "force-put" | "force-puts" => "put",
        "all" | "both" | "mixed" => "mixed",
        _ => "auto",
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
                    "Gemini indisponible pour {}\n\n{}\n\nConfig attendue dans .env:\nGEMINI_API_KEY=...\nGEMINI_MODEL=gemini-3-pro-preview\nLLM_ENABLE=1\nLLM_CACHE=1",
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
    let drivers = format_review_list(&review.decision_drivers);
    let red_flags = format_review_list(&review.red_flags);
    let risks = if review.main_risks.is_empty() {
        "-".to_string()
    } else {
        review.main_risks.join(" | ")
    };
    let execution_checks = format_review_list(&review.execution_checks);
    let data_issues = if review.data_issues.is_empty() {
        "-".to_string()
    } else {
        review.data_issues.join(" | ")
    };
    let invalidation = format_review_list(&review.invalidation_conditions);
    let questions = if review.questions_before_entry.is_empty() {
        "-".to_string()
    } else {
        review.questions_before_entry.join(" | ")
    };

    format!(
        "Modele Gemini: {}\nVerdict LLM: {}  confiance {}/100\n\nResume\n{}\n\nPourquoi ce verdict\n{}\n\nRed flags\n{}\n\nRisques principaux\n{}\n\nChecks execution\n{}\n\nProblemes de donnees\n{}\n\nReview du plan\n{}\n\nInvalidation\n{}\n\nQuestions avant entree\n{}\n\nNote: avis qualitatif uniquement; le moteur quantitatif reste la source des calculs.",
        config.model,
        review.verdict,
        review.confidence,
        review.summary,
        drivers,
        red_flags,
        risks,
        execution_checks,
        data_issues,
        review.trade_plan_review,
        invalidation,
        questions
    )
}

fn format_review_list(items: &[String]) -> String {
    if items.is_empty() {
        "-".to_string()
    } else {
        items.join(" | ")
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
        _ => Style::default().fg(Color::Cyan),
    }
}

fn side_label(config: &ScenarioConfig, spot: f64) -> String {
    match config.side.as_str() {
        "call" => "call".to_string(),
        "put" => "put".to_string(),
        "mixed" | "all" | "both" => "mixed calls+puts".to_string(),
        "auto" => match thesis_direction(config, spot) {
            Some("up") => "auto hausse".to_string(),
            Some("down") => "auto baisse".to_string(),
            _ => "auto directionnel".to_string(),
        },
        _ => "auto directionnel".to_string(),
    }
}

fn thesis_direction(config: &ScenarioConfig, spot: f64) -> Option<&'static str> {
    let target = scenario_success_level(config, spot);
    if target > spot {
        Some("up")
    } else if target < spot {
        Some("down")
    } else {
        None
    }
}

fn scenario_success_level(config: &ScenarioConfig, spot: f64) -> f64 {
    match (config.target_price_min, config.target_price_max) {
        (Some(low), Some(high)) if high > low && config.target_price >= spot => low,
        (Some(low), Some(high)) if high > low && config.target_price < spot => high,
        _ => config.target_price,
    }
}

fn no_candidate_lines(
    config: &ScenarioConfig,
    underlying: &WarrantSnapshot,
    status_message: &str,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(
        "Aucun produit ne correspond au scenario avec ce filtre.",
    )];
    if config.side == "auto" {
        let direction = match thesis_direction(config, underlying.price) {
            Some("up") => "hausse",
            Some("down") => "baisse",
            _ => "neutre",
        };
        let kept = match direction {
            "hausse" => "produits haussiers: calls, turbos/mini-futures longs",
            "baisse" => "produits baissiers: puts, turbos/mini-futures shorts",
            _ => "produits coherents avec la direction",
        };
        lines.push(Line::from(format!(
            "Mode auto directionnel: cible {direction}; le moteur garde seulement les {kept}."
        )));
        lines.push(Line::from(
            "Ici, les produits recuperes ne contiennent probablement aucun produit de ce cote avec bid/ask executable.",
        ));
        lines.push(Line::from(
            "Pour auditer ce que Boursorama renvoie quand meme, relance avec le cote mixed/all ou SCENARIO_ALLOW_OPPOSITE_SIDE=1.",
        ));
    } else {
        lines.push(Line::from(
            "Essaie d'elargir la maturite, d'augmenter la limite, ou de verifier que Boursorama renvoie un bid/ask executable.",
        ));
    }
    lines.push(Line::from(status_message.to_string()));
    lines
}

fn product_kind_label(candidate: &ScenarioCandidate) -> &'static str {
    match (candidate.product_family.as_str(), candidate.pricing_model.as_str()) {
        ("warrant", "warrant_intrinsic") => "Warrant",
        ("turbo", "financing_level") => "Turbo",
        ("mini_future", "financing_level") => "MiniF",
        ("open_end_knock_out", "financing_level") => "KO-fin",
        ("open_end_knock_out", "barrier_only") => "KO-bar",
        ("certificate", _) => "Certif",
        (_, "financing_level") => "Fin",
        (_, "barrier_only") => "Barrier",
        _ => "Autre",
    }
}

fn product_kind_style(candidate: &ScenarioCandidate) -> Style {
    match candidate.product_family.as_str() {
        "warrant" => Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        "turbo" | "mini_future" => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        "open_end_knock_out" => Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
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

fn optional_return_style(return_pct: Option<f64>) -> Style {
    return_pct
        .map(return_style)
        .unwrap_or_else(|| Style::default().fg(Color::DarkGray))
}

fn optional_risk_style(risk_pct: Option<f64>) -> Style {
    match risk_pct {
        Some(value) if value >= 50.0 => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        Some(value) if value >= 25.0 => Style::default().fg(Color::Yellow),
        Some(_) => Style::default().fg(Color::Green),
        None => Style::default().fg(Color::DarkGray),
    }
}

fn breakeven_move_cell(candidate: &ScenarioCandidate, spot: f64) -> String {
    candidate
        .breakeven_underlying
        .map(|level| format!("{:+.2}%", signed_move_pct(spot, level)))
        .unwrap_or_else(|| "-".to_string())
}

fn breakeven_move_style(candidate: &ScenarioCandidate, spot: f64) -> Style {
    let Some(level) = candidate.breakeven_underlying else {
        return Style::default().fg(Color::DarkGray);
    };
    let abs_move = signed_move_pct(spot, level).abs();
    if abs_move <= 3.0 {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else if abs_move <= 7.0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Red)
    }
}

fn exit_volatility_cell(candidate: &ScenarioCandidate) -> String {
    if is_linear_product(candidate) {
        "linear".to_string()
    } else {
        format!("{:.1}%", candidate.exit_volatility * 100.0)
    }
}

fn is_linear_product(candidate: &ScenarioCandidate) -> bool {
    candidate.pricing_model != "warrant_intrinsic"
}

fn breakeven_sentence(candidate: &ScenarioCandidate, spot: f64) -> String {
    let Some(level) = candidate.breakeven_underlying else {
        return "indisponible: impossible de resoudre le niveau rentable.".to_string();
    };
    let move_pct = signed_move_pct(spot, level);
    let direction = if move_pct >= 0.0 { "monter" } else { "baisser" };
    format!(
        "a la date cible, P/L net nul a partir de {:.4}: le sous-jacent doit {} de {:.2}% depuis le spot (BE move {:+.2}%). Ce seuil inclut le temps restant jusqu'a maturite a la date cible, les frais, l'IV de sortie ajustee et le spread de sortie estime.",
        level,
        direction,
        move_pct.abs(),
        move_pct
    )
}

fn exit_plan_sentence(
    candidate: &ScenarioCandidate,
    underlying: &WarrantSnapshot,
    config: &ScenarioConfig,
    decision_config: &DecisionConfig,
    today: NaiveDate,
) -> String {
    let spot = underlying.price;
    let target_move = signed_move_pct(spot, config.target_price);
    let stop_text = build_payoff_chart(candidate, underlying, config, decision_config, today)
    .and_then(|data| data.stop_move_pct.zip(data.stop_net_pct))
    .map(|(stop_move, stop_net)| {
        let stop_level = spot * (1.0 + stop_move / 100.0);
        format!(
            "stop mark-to-market vers {:.4} ({:+.2}% sous-jacent) ~= {:+.2}% net ({})",
            stop_level,
            stop_move,
            stop_net,
            format_money_from_pct(stop_net, decision_config.fee_order_notional)
        )
    })
    .unwrap_or_else(|| {
        format!(
            "stop mark-to-market non resolu dans la fenetre du graphe (seuil {:.2}% net)",
            -decision_config.max_loss_pct_per_trade.abs()
        )
    });
    format!(
        "entree maintenant spot {:.4}, warrant {:.4}; sortie objectif {:.4} ({:+.2}% sous-jacent) ~= {:+.2}% net; {}.",
        spot,
        candidate.entry_price,
        config.target_price,
        target_move,
        candidate.net_return_pct,
        stop_text
    )
}

fn money_plan_sentence(
    candidate: &ScenarioCandidate,
    underlying: &WarrantSnapshot,
    config: &ScenarioConfig,
    decision_config: &DecisionConfig,
    today: NaiveDate,
) -> String {
    let notional = decision_config.fee_order_notional;
    let target = format_money_from_pct(scenario_display_net_pct(candidate), notional);
    let stress = if is_linear_product(candidate) {
        "n/a".to_string()
    } else {
        format_optional_money_from_pct(candidate.stressed_net_return_pct, notional)
    };
    let stop = build_payoff_chart(candidate, underlying, config, decision_config, today)
        .and_then(|data| data.stop_net_pct)
        .map(|stop_net| format_money_from_pct(stop_net, notional))
        .unwrap_or_else(|| "-".to_string());
    format!(
        "pour {:.2} EUR engages: target {}, stress IV {}, stop {}. Au point mort, le P/L net estime est proche de 0 EUR apres frais.",
        notional, target, stress, stop
    )
}

fn scenario_display_net_pct(candidate: &ScenarioCandidate) -> f64 {
    candidate.net_return_pct
}

fn scenario_target_label(config: &ScenarioConfig, spot: f64) -> String {
    match (config.target_price_min, config.target_price_max) {
        (Some(low), Some(high)) if high > low => {
            let low_move = signed_move_pct(spot, low);
            let high_move = signed_move_pct(spot, high);
            format!(
                "{:.4}..{:.4} (mid {:.4}, {:+.2}%..{:+.2}%)",
                low, high, config.target_price, low_move, high_move
            )
        }
        _ => format!("{:.4}", config.target_price),
    }
}

fn parse_target_edit(input: &str) -> Option<(f64, Option<f64>, Option<f64>)> {
    let normalized = input.replace(',', ".");
    let value = normalized.trim();
    for separator in ["..", ":", ";"] {
        if let Some((left, right)) = value.split_once(separator) {
            let a = left.trim().parse::<f64>().ok()?;
            let b = right.trim().parse::<f64>().ok()?;
            if a <= 0.0 || b <= 0.0 || !a.is_finite() || !b.is_finite() {
                return None;
            }
            let low = a.min(b);
            let high = a.max(b);
            return Some(((low + high) / 2.0, Some(low), Some(high)));
        }
    }
    let target = value.parse::<f64>().ok()?;
    (target > 0.0 && target.is_finite()).then_some((target, None, None))
}

fn target_range_sentence(
    candidate: &ScenarioCandidate,
    decision_config: &DecisionConfig,
) -> String {
    let (Some(low), Some(high), Some(min), Some(avg), Some(max)) = (
        candidate.target_range_low,
        candidate.target_range_high,
        candidate.target_range_net_min_pct,
        candidate.target_range_net_avg_pct,
        candidate.target_range_net_max_pct,
    ) else {
        return "cible unique: pas de range configuree.".to_string();
    };
    format!(
        "range {:.4}..{:.4}: net min/moy/max {:+.2}% / {:+.2}% / {:+.2}% ({} / {} / {}); stress min {}. Par defaut, decision et tri restent bases sur le mid target pour eviter les regressions; mets SCENARIO_RANGE_DECISION_MODE=avg ou worst pour optimiser sur la zone.",
        low,
        high,
        min,
        avg,
        max,
        format_money_from_pct(min, decision_config.fee_order_notional),
        format_money_from_pct(avg, decision_config.fee_order_notional),
        format_money_from_pct(max, decision_config.fee_order_notional),
        format_optional_signed_pct(candidate.target_range_stress_min_pct)
    )
}

fn stress_line(candidate: &ScenarioCandidate, decision_config: &DecisionConfig) -> Line<'static> {
    if is_linear_product(candidate) {
        return Line::from(vec![
            Span::styled("Stress IV ", Style::default().fg(Color::Magenta)),
            Span::raw(format!(
                "n/a pour produit lineaire {}; le calcul utilise intrinsic/parite/FX. Risques restants: financement futur, barriere, spread et FX.",
                candidate.pricing_model
            )),
        ]);
    }

    Line::from(vec![
        Span::styled("Stress IV ", Style::default().fg(Color::Magenta)),
        Span::raw(format!(
            "vol dynamique {:+.2}pt -> IV sortie {:.2}%; shock extra {:+.2}pt: targetPx {} -> net {} ({}). FX stress: {}.",
            candidate.dynamic_volatility_shift_points,
            candidate.exit_volatility * 100.0,
            candidate.volatility_shock_points,
            candidate
                .stressed_projected_price
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "-".to_string()),
            format_optional_signed_pct(candidate.stressed_net_return_pct),
            format_optional_money_from_pct(
                candidate.stressed_net_return_pct,
                decision_config.fee_order_notional,
            ),
            candidate
                .fx_stressed_net_return_pct
                .map(|value| format!(
                    "fx {:.4} -> net {} ({})",
                    candidate.fx_stressed_exit_rate.unwrap_or(candidate.fx_exit_rate),
                    format_optional_signed_pct(Some(value)),
                    format_money_from_pct(value, decision_config.fee_order_notional)
                ))
                .unwrap_or_else(|| "inactif".to_string())
        )),
    ])
}

fn options_sentence(candidate: &ScenarioCandidate, config: &ScenarioConfig) -> String {
    if is_linear_product(candidate) {
        return format!(
            "IV n/a  thetaH 0.0000%  vega n/a  spread {}  spread sortie x{:.2}; produit lineaire, pas Black-Scholes.",
            format_optional_plain_pct(candidate.spread_pct),
            candidate.effective_exit_spread_multiplier
        );
    }

    format!(
        "IV entree {}  IV sortie {:.2}%  thetaH {}  spread {}  spread sortie x{:.2} (base x{:.2})",
        format_optional_factor_pct(candidate.implied_volatility),
        candidate.exit_volatility * 100.0,
        candidate
            .theta_horizon_pct
            .map(|value| format!("{value:.4}%"))
            .unwrap_or_else(|| "-".to_string()),
        format_optional_plain_pct(candidate.spread_pct),
        candidate.effective_exit_spread_multiplier,
        config.exit_spread_multiplier
    )
}

fn financing_and_barrier_sentence(candidate: &ScenarioCandidate) -> String {
    if !is_linear_product(candidate) {
        return format!(
            "reference payoff {:.4}; barriere {}; KO% {}.",
            candidate.projected_reference,
            candidate
                .projected_barrier
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "-".to_string()),
            format_optional_plain_pct(candidate.barrier_touch_probability_pct)
        );
    }
    format!(
        "reference projetee {:.4}; barriere projetee {}; cout financement vs reference actuelle {}; KO% {}.",
        candidate.projected_reference,
        candidate
            .projected_barrier
            .map(|value| format!("{value:.4}"))
            .unwrap_or_else(|| "-".to_string()),
        format_optional_signed_pct(candidate.financing_drag_pct),
        format_optional_plain_pct(candidate.barrier_touch_probability_pct)
    )
}

fn monte_carlo_sentence(candidate: &ScenarioCandidate) -> String {
    format!(
        "T avant SL/KO {}  SL MTM avant T {}  KO {}  MCev {}  P/L p05/p50/p95 {}/{}/{}",
        format_optional_plain_pct(candidate.monte_carlo_target_first_pct),
        format_optional_plain_pct(candidate.monte_carlo_stop_first_pct),
        format_optional_plain_pct(candidate.monte_carlo_ko_pct),
        format_optional_signed_pct(candidate.monte_carlo_expected_return_pct),
        format_optional_signed_pct(candidate.monte_carlo_p05_return_pct),
        format_optional_signed_pct(candidate.monte_carlo_p50_return_pct),
        format_optional_signed_pct(candidate.monte_carlo_p95_return_pct)
    )
}

fn target_vs_breakeven_sentence(
    candidate: &ScenarioCandidate,
    spot: f64,
    config: &ScenarioConfig,
) -> String {
    let target_move = signed_move_pct(spot, config.target_price);
    let Some(breakeven) = candidate.breakeven_underlying else {
        return "point mort indisponible: impossible de comparer le target au seuil de rentabilite."
            .to_string();
    };
    let breakeven_move = signed_move_pct(spot, breakeven);
    let target_before_breakeven = match candidate.side.as_str() {
        "call" => config.target_price < breakeven,
        "put" => config.target_price > breakeven,
        _ => false,
    };
    if target_before_breakeven {
        format!(
            "target {:+.2}% avant BE target {:+.2}%: meme si le scenario arrive, le warrant reste en perte nette ({:+.2}%).",
            target_move, breakeven_move, candidate.net_return_pct
        )
    } else {
        format!(
            "target {:+.2}% apres BE target {:+.2}%: le scenario depasse le seuil de rentabilite nette.",
            target_move, breakeven_move
        )
    }
}

fn signed_move_pct(spot: f64, level: f64) -> f64 {
    if spot <= 0.0 {
        0.0
    } else {
        (level - spot) / spot * 100.0
    }
}

fn format_compact_float(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.4}")
    }
}

fn format_contract_level(value: f64) -> String {
    if value.fract().abs() < 0.005 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
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

fn format_signed_pct(value: f64) -> String {
    format!("{value:+.2}%")
}

fn format_money_from_pct(pct: f64, notional: f64) -> String {
    let amount = notional * pct / 100.0;
    format!("{amount:+.2} EUR")
}

fn format_optional_money_from_pct(pct: Option<f64>, notional: f64) -> String {
    pct.map(|pct| format_money_from_pct(pct, notional))
        .unwrap_or_else(|| "-".to_string())
}

fn format_money_table_from_pct(pct: f64, notional: f64) -> String {
    let amount = notional * pct / 100.0;
    format!("{amount:+.2}")
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

fn kelly_required_probability(candidate: &ScenarioCandidate, decision_config: &DecisionConfig) -> String {
    if candidate.net_return_pct <= 0.0 {
        return "-".to_string();
    }
    let gain = candidate.net_return_pct.max(0.0);
    let loss = decision_config.max_loss_pct_per_trade.abs().max(0.01);
    let required = loss / (gain + loss);
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
            "target_range_has_negative_edge" => {
                "range cible fragile: une partie de la zone donne un rendement net negatif"
            }
            "entry_price_not_executable" => "prix d'entree non executable: bid/ask absent ou invalide",
            "maturity_before_preferred_window" => "maturite avant la fenetre souhaitee",
            "maturity_after_preferred_window" => "maturite apres la fenetre souhaitee",
            "data_quality_too_low" => "qualite de donnee trop faible",
            "liquidity_too_low" => "liquidite trop faible",
            "spread_too_wide" => "spread trop large",
            "breakeven_beyond_target" => "breakeven au-dela du target",
            "scenario_return_below_buy_threshold" => "rendement net positif mais sous le seuil BUY scenario",
            "target_probability_too_low" => "probabilite d'atteindre la cible trop faible",
            "breakeven_probability_too_low" => "probabilite d'atteindre le breakeven trop faible",
            "expected_value_negative" => {
                "fair-value Q negative: le prix actuel est cher selon le modele risk-neutral"
            }
            "monte_carlo_ev_negative" => {
                "EV Monte Carlo negative: les trajectoires simulees ne compensent pas le risque"
            }
            "target_before_stop_not_favored" => {
                "Monte Carlo defavorable: le stop mark-to-market/KO arrive au moins aussi souvent que le target"
            }
            "barrier_touch_probability_too_high" => "probabilite de toucher la barriere trop elevee",
            "linear_financing_unmodeled_long_horizon" => {
                "produit lineaire open-end sur horizon long: financement futur non modelise, BUY bloque"
            }
            "volatility_crush_erases_return" => "stress de baisse d'IV efface le gain projete",
            "fx_stress_erases_return" => "stress FX efface le gain projete",
            _ => "raison non documentee",
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn human_warnings(warnings: &[String]) -> String {
    if warnings.is_empty() {
        return "-".to_string();
    }

    warnings
        .iter()
        .map(|warning| match warning.as_str() {
            "possible_us_premarket_spot_mismatch" => {
                "risque stale pricing: warrant europeen actif alors que spot US officiel peut etre decale"
            }
            "discrete_dividends_applied" => "dividendes discrets appliques dans le pricing de sortie",
            "dynamic_exit_spread_penalty" => {
                "spread de sortie elargi car le delta projete est proche d'une zone extreme"
            }
            "opposite_side_scenario" => {
                "side nominal oppose a la these, garde car le pricing projete reste analyse"
            }
            "linear_product_projection" => {
                "produit lineaire: projection par valeur intrinseque/parite/FX, pas Black-Scholes"
            }
            "open_end_no_expiry_model" => {
                "open-end: pas d'echeance contractuelle, theta force a 0 dans le scenario"
            }
            "future_financing_not_projected" => {
                "niveau de financement/barriere futur non projete: projection basee sur la reference actuelle"
            }
            "future_financing_projected" => {
                "niveau de financement/barriere projete avec SCENARIO_LINEAR_FINANCING_RATE_PCT"
            }
            _ => "warning non documente",
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn format_exit_formula(candidate: &ScenarioCandidate, config: &ScenarioConfig) -> String {
    if candidate.pricing_model == "warrant_intrinsic" {
        return format!(
            "Calc exit@date: Black-Scholes({}, S_eff={:.4}, K={:.4}, T cible->maturite, IV_sortie={:.2}%, r={:.2}%, q={:.2}%) / parity {} * fx_sortie {:.4}, puis penalite bid/spread. Les frais ne sont pas dans ExitBid@D; ils sont dans Net@D/P/L@D.",
            candidate.side,
            (config.target_price - candidate.discrete_dividend_pv).max(0.01),
            candidate.strike,
            candidate.exit_volatility * 100.0,
            config.risk_free_rate * 100.0,
            config.dividend_yield * 100.0,
            format_compact_float(candidate.parity),
            candidate.fx_exit_rate
        );
    }

    format!(
        "Calc exit@date: modele lineaire {}: intrinsic=max(direction*(S_eff-K),0) / parity {} * fx_sortie {:.4}, avec barriere si fournie, puis penalite bid/spread. Pas de theta/vega Black-Scholes pour ce produit.",
        candidate.pricing_model,
        format_compact_float(candidate.parity),
        candidate.fx_exit_rate
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_stop_is_found_on_adverse_downside_move() {
        let curve = vec![(-10.0, -40.0), (-5.0, -20.0), (0.0, 0.0), (5.0, 20.0)];

        let stop = find_stop_move(&curve, OptionKind::Call, -25.0).unwrap();

        assert!((stop - -6.25).abs() < 1e-9);
    }

    #[test]
    fn put_stop_is_found_on_adverse_upside_move() {
        let curve = vec![(-5.0, 20.0), (0.0, 0.0), (5.0, -20.0), (10.0, -40.0)];

        let stop = find_stop_move(&curve, OptionKind::Put, -25.0).unwrap();

        assert!((stop - 6.25).abs() < 1e-9);
    }

    #[test]
    fn stop_is_none_when_loss_threshold_is_not_reached() {
        let curve = vec![(-10.0, -10.0), (-5.0, -5.0), (0.0, 0.0), (5.0, 10.0)];

        assert!(find_stop_move(&curve, OptionKind::Call, -25.0).is_none());
    }

    #[test]
    fn money_format_converts_percent_to_notional_pl() {
        assert_eq!(format_money_from_pct(12.5, 1000.0), "+125.00 EUR");
        assert_eq!(format_money_from_pct(-3.8, 1000.0), "-38.00 EUR");
    }
}
