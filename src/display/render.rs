use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::display::app::{App, WarrantRow};
use crate::display::truncate::truncate;

/// Point d'entrée du rendu — appelé à chaque tick par la boucle TUI.
pub fn draw(f: &mut Frame, app: &App, refresh_secs: u64, secs_left: u64) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // en-tête
            Constraint::Min(5),    // tableau
            Constraint::Length(1), // pied de page
        ])
        .split(area);

    draw_header(f, chunks[0], app, refresh_secs, secs_left);
    draw_table(f, chunks[1], app);
    draw_footer(f, chunks[2]);
}

// ── En-tête ───────────────────────────────────────────────────────────────────

fn draw_header(
    f: &mut Frame,
    area: Rect,
    app: &App,
    refresh_secs: u64,
    secs_left: u64,
) {
    let bar = "█".repeat(progress_bar_len(refresh_secs, secs_left, 20));
    let empty = "░".repeat(20usize.saturating_sub(bar.len()));

    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled(&app.status, Style::default().fg(Color::White)),
        Span::raw("    "),
        Span::styled(
            format!("[{}{}] {}s", bar, empty, secs_left),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .title(Span::styled(
            " WARRANT FETCHER — Warrants Apple ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    f.render_widget(Paragraph::new(line).block(block), area);
}

fn progress_bar_len(refresh_secs: u64, secs_left: u64, width: usize) -> usize {
    if refresh_secs == 0 {
        return 0;
    }
    let elapsed = refresh_secs.saturating_sub(secs_left);
    ((elapsed as f64 / refresh_secs as f64) * width as f64) as usize
}

// ── Tableau ───────────────────────────────────────────────────────────────────

fn draw_table(f: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(
        ["TICKER", "NOM  /  52 SEMAINES", "PRIX", "VAR%", "VOLUME", "CLÔT.VEILLE"]
            .map(|h| {
                Cell::from(h).style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            }),
    )
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = app
        .rows
        .iter()
        .map(|row| match row {
            WarrantRow::Data(s) => make_data_row(s),
            WarrantRow::Error(e) => make_error_row(e),
        })
        .collect();

    let widths = [
        Constraint::Length(13),
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(12),
        Constraint::Length(13),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(
                    " Données en temps réel ",
                    Style::default().fg(Color::DarkGray),
                )),
        )
        .column_spacing(1);

    f.render_widget(table, area);
}

fn make_data_row(s: &crate::models::warrant::WarrantSnapshot) -> Row<'_> {
    let (color, trend) = if s.change_pct >= 0.0 {
        (Color::Green, "▲")
    } else {
        (Color::Red, "▼")
    };

    let candle_info = s.last_candle.as_ref().map(|c| {
        format!("  O:{:.3} H:{:.3} L:{:.3} C:{:.3}", c.open, c.high, c.low, c.close)
    });

    let detail = format!(
        "  └ 52w H:{:.3}  B:{:.3}  {}{}",
        s.high_52w,
        s.low_52w,
        s.currency,
        candle_info.as_deref().unwrap_or(""),
    );

    let name_text = Text::from(vec![
        Line::from(truncate(&s.name, 35)),
        Line::from(Span::styled(detail, Style::default().fg(Color::DarkGray))),
    ]);

    let var_text = Text::from(vec![
        Line::from(Span::styled(
            format!("{:+.2}% {}", s.change_pct, trend),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ]);

    Row::new(vec![
        Cell::from(Text::from(vec![
            Line::from(Span::styled(
                s.ticker.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ])),
        Cell::from(name_text),
        Cell::from(Text::from(vec![
            Line::from(format!("{:.4}", s.price)),
            Line::from(""),
        ])),
        Cell::from(var_text),
        Cell::from(Text::from(vec![
            Line::from(format!("{}", s.volume)),
            Line::from(""),
        ])),
        Cell::from(Text::from(vec![
            Line::from(format!("{:.4}", s.prev_close)),
            Line::from(""),
        ])),
    ])
    .height(2)
}

fn make_error_row(msg: &str) -> Row<'_> {
    let short = truncate(msg, 80);
    Row::new(vec![
        Cell::from(Span::styled("ERREUR", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))),
        Cell::from(Span::styled(short, Style::default().fg(Color::Red))),
        Cell::from(""),
        Cell::from(""),
        Cell::from(""),
        Cell::from(""),
    ])
    .height(1)
}

// ── Pied de page ──────────────────────────────────────────────────────────────

fn draw_footer(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" [q]", Style::default().fg(Color::Yellow)),
        Span::raw(" Quitter   "),
        Span::styled("[r]", Style::default().fg(Color::Yellow)),
        Span::raw(" Rafraîchir maintenant   "),
        Span::styled("[Ctrl+C]", Style::default().fg(Color::Yellow)),
        Span::raw(" Quitter "),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
