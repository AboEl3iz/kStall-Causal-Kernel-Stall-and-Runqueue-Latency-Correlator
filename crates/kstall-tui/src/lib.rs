// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

pub mod app;
pub mod ui;

use anyhow::Result;
pub use app::{App, InputMode, SortOrder, Tab};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use kstall_core::model::Episode;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Launch the interactive ratatui TUI.
pub async fn run_tui(
    episode_rx: mpsc::Receiver<Episode>,
    stop_flag: Arc<AtomicBool>,
    window_duration_ns: u64,
) -> Result<()> {
    run_tui_filtered(episode_rx, stop_flag, window_duration_ns, false, None).await
}

/// Launch the interactive ratatui TUI with optional container filters.
pub async fn run_tui_filtered(
    mut episode_rx: mpsc::Receiver<Episode>,
    stop_flag: Arc<AtomicBool>,
    window_duration_ns: u64,
    containers_only: bool,
    container_filter: Option<String>,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(window_duration_ns);
    app.containers_only = containers_only;
    app.container_filter = container_filter;

    while !app.should_quit && !stop_flag.load(Ordering::Relaxed) {
        // Drain any pending episodes from channel
        while let Ok(ep) = episode_rx.try_recv() {
            app.on_episode(ep);
        }

        terminal.draw(|f| ui::render(f, &app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                handle_key_event(&mut app, key.code, key.modifiers, &stop_flag);
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

fn handle_key_event(
    app: &mut App,
    code: KeyCode,
    modifiers: KeyModifiers,
    stop_flag: &Arc<AtomicBool>,
) {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        app.quit();
        stop_flag.store(true, Ordering::Relaxed);
        return;
    }

    match app.input_mode {
        InputMode::Filter => match code {
            KeyCode::Enter => {
                app.input_mode = InputMode::Normal;
                app.selected_index = 0;
            }
            KeyCode::Esc => {
                app.filter_text.clear();
                app.input_mode = InputMode::Normal;
                app.selected_index = 0;
            }
            KeyCode::Backspace => {
                app.filter_text.pop();
                app.selected_index = 0;
            }
            KeyCode::Char(c) => {
                app.filter_text.push(c);
                app.selected_index = 0;
            }
            _ => {}
        },
        InputMode::Normal => match code {
            KeyCode::Char('q') => {
                if !app.close_overlays() {
                    app.quit();
                    stop_flag.store(true, Ordering::Relaxed);
                }
            }
            KeyCode::Esc => {
                app.close_overlays();
            }
            KeyCode::Tab => app.next_tab(),
            KeyCode::BackTab => app.prev_tab(),
            KeyCode::Char('1') => {
                app.current_tab = Tab::Waterfall;
                app.selected_index = 0;
            }
            KeyCode::Char('2') => {
                app.current_tab = Tab::Breakdown;
                app.selected_index = 0;
            }
            KeyCode::Char('3') => {
                app.current_tab = Tab::InterferenceMatrix;
                app.selected_index = 0;
            }
            KeyCode::Char('4') => {
                app.current_tab = Tab::EventLog;
                app.selected_index = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Enter => app.toggle_detail(),
            KeyCode::Char('/') => {
                app.input_mode = InputMode::Filter;
            }
            KeyCode::Char('c') => app.toggle_containers_only(),
            KeyCode::Char('s') => app.toggle_sort(),
            KeyCode::Char('p') => app.toggle_pause(),
            KeyCode::Char('e') => {
                if let Err(e) = app.export_snapshot() {
                    app.set_status(format!("Export failed: {}", e));
                }
            }
            KeyCode::Char('?') => app.toggle_help(),
            _ => {}
        },
    }
}
