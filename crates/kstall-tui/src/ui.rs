// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use crate::app::{App, InputMode, Tab};
use kstall_core::model::{CauseKind, Episode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap},
    Frame,
};

pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header & status
            Constraint::Length(3), // Tab navigation
            Constraint::Min(0),    // Main workspace
            Constraint::Length(1), // Footer hotkeys & filter prompt
        ])
        .split(frame.area());

    render_header(frame, app, chunks[0]);
    render_tabs(frame, app, chunks[1]);
    render_body(frame, app, chunks[2]);
    render_footer(frame, app, chunks[3]);

    if app.show_detail {
        render_drilldown_modal(frame, app);
    } else if app.show_help {
        render_help_modal(frame);
    }
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let quantiles = app.window.quantiles();
    let state_label = if app.is_paused {
        Span::styled(
            "[PAUSED]",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "[LIVE]",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    };

    let sort_label = Span::styled(
        format!("[{}]", app.sort_order.label()),
        Style::default().fg(Color::Cyan),
    );

    let container_badge = if app.containers_only {
        Some(Span::styled(
            " [CONTAINERS ONLY]",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        None
    };

    let filter_label = if !app.filter_text.is_empty() {
        format!(" [Filter: \"{}\"]", app.filter_text)
    } else {
        String::new()
    };

    let stats_text = format!(
        " p50: {:.2}ms | p95: {:.2}ms | p99: {:.2}ms | Outliers: {} | Rate: {:.1} ep/s{}",
        quantiles.p50_ns as f64 / 1_000_000.0,
        quantiles.p95_ns as f64 / 1_000_000.0,
        quantiles.p99_ns as f64 / 1_000_000.0,
        quantiles.count,
        app.rate_per_sec(),
        filter_label
    );

    let mut header_spans = vec![state_label, Span::raw(" "), sort_label];
    if let Some(badge) = container_badge {
        header_spans.push(badge);
    }
    header_spans.push(Span::styled(stats_text, Style::default().fg(Color::White)));

    let header_line = Line::from(header_spans);

    let header_block = Block::default()
        .borders(Borders::ALL)
        .title(" kStall — Causal Kernel-Stall & Runqueue Latency Correlator ")
        .style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(header_line).block(header_block);
    frame.render_widget(paragraph, area);
}

fn render_tabs(frame: &mut Frame, app: &App, area: Rect) {
    let tab_titles: Vec<Line> = Tab::all()
        .iter()
        .enumerate()
        .map(|(i, t)| Line::from(format!(" [{}] {} ", i + 1, t.title())))
        .collect();

    let selected_idx = Tab::all()
        .iter()
        .position(|&t| t == app.current_tab)
        .unwrap_or(0);

    let tabs = Tabs::new(tab_titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Subsystem Views "),
        )
        .select(selected_idx)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );

    frame.render_widget(tabs, area);
}

fn render_body(frame: &mut Frame, app: &App, area: Rect) {
    match app.current_tab {
        Tab::Waterfall => render_waterfall_view(frame, app, area),
        Tab::Breakdown => render_breakdown_view(frame, app, area),
        Tab::InterferenceMatrix => render_interference_matrix(frame, app, area),
        Tab::EventLog => render_event_log(frame, app, area),
    }
}

fn cause_style(cause: CauseKind) -> Style {
    match cause {
        CauseKind::RunqueueWait => Style::default().fg(Color::LightRed),
        CauseKind::PreemptedWait => Style::default().fg(Color::Magenta),
        CauseKind::DirectReclaim => Style::default().fg(Color::Yellow),
        CauseKind::Compaction => Style::default().fg(Color::Blue),
        CauseKind::Throttled => Style::default().fg(Color::LightCyan),
        CauseKind::Unknown => Style::default().fg(Color::DarkGray),
    }
}

fn render_waterfall_bar(ep: &Episode, max_chars: usize) -> Vec<Span<'static>> {
    if ep.segments.is_empty() || max_chars < 6 {
        return vec![Span::raw("-")];
    }

    let mut spans = Vec::new();
    let total_ns = ep.total_delay_ns.max(1) as f64;

    for seg in &ep.segments {
        let seg_frac = seg.duration_ns as f64 / total_ns;
        let char_len = ((seg_frac * max_chars as f64).round() as usize).max(1);

        let tag = match seg.cause {
            CauseKind::RunqueueWait => "RQ",
            CauseKind::PreemptedWait => "PR",
            CauseKind::DirectReclaim => "REC",
            CauseKind::Compaction => "CMP",
            CauseKind::Throttled => "THR",
            CauseKind::Unknown => "UNK",
        };

        let duration_ms = seg.duration_ns as f64 / 1_000_000.0;
        let text = if char_len > 8 {
            format!("[{} {:.1}ms]", tag, duration_ms)
        } else if char_len > 3 {
            format!("[{}]", tag)
        } else {
            "█".to_string()
        };

        spans.push(Span::styled(
            text,
            cause_style(seg.cause).add_modifier(Modifier::BOLD),
        ));
    }

    spans
}

fn render_waterfall_view(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(65), // Episodes table with visual bar
            Constraint::Percentage(35), // Selected episode timeline inspection
        ])
        .split(area);

    let episodes = app.filtered_episodes();
    let selected_idx = app.selected_index.min(episodes.len().saturating_sub(1));

    let header = Row::new(vec![
        Cell::from(" "),
        Cell::from("TID"),
        Cell::from("COMM"),
        Cell::from("DELAY (ms)"),
        Cell::from("TIMELINE SEGMENTS"),
        Cell::from("PRIMARY CULPRIT"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let visible_rows = chunks[0].height.saturating_sub(4) as usize;
    let start_idx = if selected_idx >= visible_rows {
        selected_idx.saturating_sub(visible_rows - 1)
    } else {
        0
    };

    let rows: Vec<Row> = episodes
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(visible_rows)
        .map(|(idx, ep)| {
            let is_selected = idx == selected_idx;
            let marker = if is_selected { ">" } else { " " };

            let culprit_str = ep
                .culprits
                .first()
                .map(|c| format!("{} (PID {})", c.comm, c.tgid))
                .unwrap_or_else(|| "-".to_string());

            let timeline_spans = render_waterfall_bar(ep, 24);

            let row_style = if is_selected {
                Style::default().bg(Color::Rgb(30, 45, 60)).fg(Color::White)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(marker),
                Cell::from(ep.tid.to_string()),
                Cell::from(ep.comm.clone()),
                Cell::from(format!("{:.2}", ep.total_delay_ns as f64 / 1_000_000.0)),
                Cell::from(Line::from(timeline_spans)),
                Cell::from(culprit_str),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Length(30),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(format!(
        " Outlier Stall Episodes (Total: {}) ",
        episodes.len()
    )));

    frame.render_widget(table, chunks[0]);

    // Bottom inspection pane
    render_selected_preview(frame, app.selected_episode(), chunks[1]);
}

fn render_selected_preview(frame: &mut Frame, selected: Option<&Episode>, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Selected Episode Causal Inspection [Press Enter to Expand] ")
        .style(Style::default().fg(Color::Cyan));

    if let Some(ep) = selected {
        let bd = ep.breakdown();
        let cgroup_desc = ep.cgroup_path.as_deref().unwrap_or("-");
        let pod_desc = ep.pod_name.as_deref().unwrap_or("-");

        let culprits_desc = if ep.culprits.is_empty() {
            "None identified".to_string()
        } else {
            ep.culprits
                .iter()
                .map(|c| {
                    format!(
                        "{} (PID {}, Stolen: {:.2}ms)",
                        c.comm,
                        c.tgid,
                        c.stolen_ns as f64 / 1_000_000.0
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        let lines = vec![
            Line::from(vec![
                Span::styled("Victim: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{} (PID: {}, TID: {})", ep.comm, ep.tgid, ep.tid),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(" | "),
                Span::styled(
                    "Total Latency: ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:.2} ms", ep.total_delay_ns as f64 / 1_000_000.0),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | "),
                Span::styled("Pod: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(pod_desc),
                Span::raw(" | "),
                Span::styled("Cgroup: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(cgroup_desc),
            ]),
            Line::from(vec![
                Span::styled(
                    "Cause Breakdown: ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("Runqueue: {:.2}ms ", bd.runq_ns as f64 / 1_000_000.0),
                    Style::default().fg(Color::LightRed),
                ),
                Span::styled(
                    format!("Preempted: {:.2}ms ", bd.preempt_ns as f64 / 1_000_000.0),
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled(
                    format!("Reclaim: {:.2}ms ", bd.reclaim_ns as f64 / 1_000_000.0),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("Compaction: {:.2}ms ", bd.compact_ns as f64 / 1_000_000.0),
                    Style::default().fg(Color::Blue),
                ),
                Span::styled(
                    format!("Throttled: {:.2}ms", bd.throttle_ns as f64 / 1_000_000.0),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Interfering Culprits: ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(culprits_desc, Style::default().fg(Color::LightYellow)),
            ]),
        ];

        let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        frame.render_widget(p, area);
    } else {
        let p = Paragraph::new("No stall episode selected.")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
    }
}

fn render_breakdown_view(frame: &mut Frame, app: &App, area: Rect) {
    let episodes = app.filtered_episodes();
    let header = Row::new(vec![
        Cell::from("COMM"),
        Cell::from("PID"),
        Cell::from("DELAY (ms)"),
        Cell::from("RUNQ (ms)"),
        Cell::from("PREEMPT (ms)"),
        Cell::from("RECLAIM (ms)"),
        Cell::from("COMPACT (ms)"),
        Cell::from("THROTTLE (ms)"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = episodes
        .iter()
        .take(area.height.saturating_sub(3) as usize)
        .map(|ep| {
            let bd = ep.breakdown();
            Row::new(vec![
                Cell::from(ep.comm.clone()),
                Cell::from(ep.tgid.to_string()),
                Cell::from(format!("{:.2}", ep.total_delay_ns as f64 / 1_000_000.0)),
                Cell::from(format!("{:.2}", bd.runq_ns as f64 / 1_000_000.0)),
                Cell::from(format!("{:.2}", bd.preempt_ns as f64 / 1_000_000.0)),
                Cell::from(format!("{:.2}", bd.reclaim_ns as f64 / 1_000_000.0)),
                Cell::from(format!("{:.2}", bd.compact_ns as f64 / 1_000_000.0)),
                Cell::from(format!("{:.2}", bd.throttle_ns as f64 / 1_000_000.0)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Kernel Subsystem Latency Breakdown "),
    );

    frame.render_widget(table, area);
}

fn render_interference_matrix(frame: &mut Frame, app: &App, area: Rect) {
    let matrix = app.window.interference_matrix();
    let header = Row::new(vec![
        Cell::from("VICTIM COMM"),
        Cell::from("VICTIM PID"),
        Cell::from("CULPRIT COMM"),
        Cell::from("CULPRIT PID"),
        Cell::from("TOTAL STOLEN (ms)"),
        Cell::from("INCIDENTS"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = matrix
        .into_iter()
        .take(area.height.saturating_sub(3) as usize)
        .map(|entry| {
            Row::new(vec![
                Cell::from(entry.victim_comm),
                Cell::from(entry.victim_tgid.to_string()),
                Cell::from(entry.culprit_comm),
                Cell::from(entry.culprit_tgid.to_string()),
                Cell::from(format!("{:.2}", entry.total_stolen_ns as f64 / 1_000_000.0)),
                Cell::from(entry.incident_count.to_string()),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Length(18),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Interference Matrix (Victim x Culprit Stolen CPU) "),
    );

    frame.render_widget(table, area);
}

fn render_event_log(frame: &mut Frame, app: &App, area: Rect) {
    let episodes = app.filtered_episodes();
    let header = Row::new(vec![
        Cell::from("TIMESTAMP (s)"),
        Cell::from("TID"),
        Cell::from("COMM"),
        Cell::from("DURATION (ms)"),
        Cell::from("PRIMARY CAUSE"),
        Cell::from("CULPRIT"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = episodes
        .iter()
        .take(area.height.saturating_sub(3) as usize)
        .map(|ep| {
            let culprit = ep
                .culprits
                .first()
                .map(|c| c.comm.as_str())
                .unwrap_or("none");

            let cause_str = ep
                .segments
                .first()
                .map(|s| s.cause.as_str())
                .unwrap_or("unknown");

            Row::new(vec![
                Cell::from(format!("{:.3}", ep.start_ns as f64 / 1_000_000_000.0)),
                Cell::from(ep.tid.to_string()),
                Cell::from(ep.comm.clone()),
                Cell::from(format!("{:.2}", ep.total_delay_ns as f64 / 1_000_000.0)),
                Cell::from(cause_str),
                Cell::from(culprit),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Min(16),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Raw Outlier Event Stream "),
    );

    frame.render_widget(table, area);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if app.input_mode == InputMode::Filter {
        let filter_line = Line::from(vec![
            Span::styled(
                "Filter query: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&app.filter_text, Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Yellow)),
            Span::styled(
                " (Press Enter to apply, Esc to cancel)",
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        let p = Paragraph::new(filter_line);
        frame.render_widget(p, area);
        return;
    }

    if let Some(status) = app.get_active_status() {
        let line = Line::from(vec![
            Span::styled(
                ">> ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                status,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        let p = Paragraph::new(line);
        frame.render_widget(p, area);
        return;
    }

    let footer_text =
        " [Tab] View | [↑/↓] Select | [Enter] Drilldown | [c] Containers Only | [/] Filter | [s] Sort | [p] Pause | [e] Export | [?] Help | [q] Quit ";
    let footer = Paragraph::new(footer_text).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    );
    frame.render_widget(footer, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn render_drilldown_modal(frame: &mut Frame, app: &App) {
    let area = centered_rect(80, 75, frame.area());
    frame.render_widget(Clear, area);

    let Some(ep) = app.selected_episode() else {
        return;
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Episode Causal Timeline Detail [Esc / Enter to close] ")
        .style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Metadata
            Constraint::Min(6),    // Segments table
        ])
        .split(inner);

    let meta_lines = vec![
        Line::from(vec![
            Span::styled(
                "Process / Thread: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} (TGID: {}, TID: {})", ep.comm, ep.tgid, ep.tid),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" | "),
            Span::styled(
                "Total Stall: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:.2} ms", ep.total_delay_ns as f64 / 1_000_000.0),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Cgroup: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(ep.cgroup_path.as_deref().unwrap_or("-")),
            Span::raw(" | "),
            Span::styled(
                "Pod / Container: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{} / {}",
                ep.pod_name.as_deref().unwrap_or("-"),
                ep.container_id.as_deref().unwrap_or("-")
            )),
        ]),
        Line::from(vec![
            Span::styled(
                "Start Time: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{:.6} s", ep.start_ns as f64 / 1_000_000_000.0)),
            Span::raw(" | "),
            Span::styled("End Time: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("{:.6} s", ep.end_ns as f64 / 1_000_000_000.0)),
        ]),
    ];

    let meta_para = Paragraph::new(meta_lines);
    frame.render_widget(meta_para, chunks[0]);

    // Segments Table
    let header = Row::new(vec![
        Cell::from("#"),
        Cell::from("CAUSE"),
        Cell::from("START REL (ms)"),
        Cell::from("DURATION (ms)"),
        Cell::from("CPU"),
        Cell::from("CULPRIT COMM:PID"),
        Cell::from("STOLEN (ms)"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = ep
        .segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let rel_start_ms = (seg.start_ns.saturating_sub(ep.start_ns)) as f64 / 1_000_000.0;
            let duration_ms = seg.duration_ns as f64 / 1_000_000.0;
            let (culprit_str, stolen_str) = if let Some(c) = &seg.culprit {
                (
                    format!("{}:{}", c.comm, c.tgid),
                    format!("{:.2}", c.stolen_ns as f64 / 1_000_000.0),
                )
            } else {
                ("-".to_string(), "-".to_string())
            };

            Row::new(vec![
                Cell::from((i + 1).to_string()),
                Cell::from(Span::styled(seg.cause.as_str(), cause_style(seg.cause))),
                Cell::from(format!("+{:.2}", rel_start_ms)),
                Cell::from(format!("{:.2}", duration_ms)),
                Cell::from(seg.cpu.to_string()),
                Cell::from(culprit_str),
                Cell::from(stolen_str),
            ])
        })
        .collect();

    let seg_table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(16),
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(6),
            Constraint::Length(20),
            Constraint::Min(12),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::TOP)
            .title(" Causal Segments "),
    );

    frame.render_widget(seg_table, chunks[1]);
}

fn render_help_modal(frame: &mut Frame) {
    let area = centered_rect(65, 60, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" kStall Help & Shortcuts [Esc / ? to close] ")
        .style(Style::default().fg(Color::Yellow));

    let help_text = vec![
        Line::from(vec![Span::styled(
            "Navigation & Control",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(
            "  Tab / 1-4    : Switch views (Waterfall, Breakdown, Interference Matrix, Event Log)",
        ),
        Line::from("  ↑ / k, ↓ / j : Navigate episode list"),
        Line::from("  Enter        : Open causal timeline drilldown modal for selected episode"),
        Line::from(
            "  c            : Toggle Containers Only filter (show only Docker/CRI containers)",
        ),
        Line::from("  /            : Interactive filter by comm, PID, culprit, cgroup, or cause"),
        Line::from("  s            : Cycle sorting (Duration ↓, Time ↓, Duration ↑, Time ↑)"),
        Line::from("  p            : Pause / Resume live event stream"),
        Line::from(
            "  e            : Export current window to Perfetto-compatible Chrome Trace JSON",
        ),
        Line::from("  ?            : Toggle this help screen"),
        Line::from("  q / Esc      : Quit or close current modal/filter"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Causal Latency Classifications",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  • RunqueueWait  : Woken thread starved in runqueue by another task"),
        Line::from("  • PreemptedWait : Runnable task pre-empted by higher priority or CFS slice"),
        Line::from("  • DirectReclaim : Thread stalled synchronous memory page reclaim"),
        Line::from("  • Compaction    : Thread stalled memory compaction for high-order pages"),
        Line::from("  • Throttled     : Thread paused due to cgroup CFS quota bandwidth limit"),
    ];

    let p = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });

    frame.render_widget(p, area);
}
