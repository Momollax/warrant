use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use crate::api::client::build_client;
use crate::api::fetch_all::fetch_all;
use crate::display::app::{App, WarrantRow};
use crate::display::render::draw;

/// Initialise le terminal en mode TUI, lance la boucle, puis restaure le terminal.
pub async fn run(tickers: Vec<String>, max_cycles: u32, refresh_secs: u64) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let res = run_loop(&mut terminal, tickers, max_cycles, refresh_secs).await;

    // Restauration du terminal même en cas d'erreur
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    res
}

async fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    tickers: Vec<String>,
    max_cycles: u32,
    refresh_secs: u64,
) -> Result<()> {
    let mut app = App::new();
    let client = build_client()?;

    // Thread bloquant dédié à la lecture des événements clavier → channel
    let (key_tx, mut key_rx) = mpsc::unbounded_channel::<Event>();
    tokio::task::spawn_blocking(move || loop {
        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            if let Ok(ev) = event::read() {
                if key_tx.send(ev).is_err() {
                    break; // channel fermé → fin de l'app
                }
            }
        }
    });

    let tick_rate = Duration::from_millis(150);
    let refresh   = Duration::from_secs(refresh_secs);
    let mut last_render = Instant::now() - tick_rate;
    let mut next_fetch  = Instant::now(); // premier fetch immédiat

    loop {
        // ── Fetch ────────────────────────────────────────────────────────
        if app.force_refresh || Instant::now() >= next_fetch {
            app.force_refresh = false;
            app.cycle += 1;
            app.status = format!("Cycle #{} — récupération en cours…", app.cycle);
            terminal.draw(|f| draw(f, &app, refresh_secs, 0))?;

            let results = fetch_all(&client, &tickers).await;
            next_fetch = Instant::now() + refresh;

            app.rows = results
                .into_iter()
                .map(|r| match r {
                    Ok(s)  => WarrantRow::Data(s),
                    Err(e) => WarrantRow::Error(e.to_string()),
                })
                .collect();

            let errs = app.rows.iter().filter(|r| matches!(r, WarrantRow::Error(_))).count();
            app.status = if errs > 0 {
                format!("Cycle #{} — {} erreur(s)", app.cycle, errs)
            } else {
                format!("Cycle #{} — OK", app.cycle)
            };

            if max_cycles > 0 && app.cycle >= max_cycles {
                break;
            }
        }

        // ── Rendu ─────────────────────────────────────────────────────────
        if last_render.elapsed() >= tick_rate {
            let secs_left = next_fetch
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO)
                .as_secs();
            terminal.draw(|f| draw(f, &app, refresh_secs, secs_left))?;
            last_render = Instant::now();
        }

        // ── Événements clavier ────────────────────────────────────────────
        while let Ok(ev) = key_rx.try_recv() {
            if let Event::Key(key) = ev {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                    KeyCode::Char('c')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        app.should_quit = true;
                    }
                    KeyCode::Char('r') => app.force_refresh = true,
                    _ => {}
                }
            }
        }

        if app.should_quit {
            break;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Ok(())
}
