// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use anyhow::Result;
use kstall_core::model::Episode;
use kstall_core::stats::EpisodeWindow;
use kstall_export::ChromeTrace;
use std::fs::File;
use std::io::BufWriter;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Waterfall,
    Breakdown,
    InterferenceMatrix,
    EventLog,
}

impl Tab {
    pub fn all() -> &'static [Tab] {
        &[
            Tab::Waterfall,
            Tab::Breakdown,
            Tab::InterferenceMatrix,
            Tab::EventLog,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Waterfall => "Waterfall (Timeline)",
            Tab::Breakdown => "Latency Breakdown",
            Tab::InterferenceMatrix => "Interference Matrix",
            Tab::EventLog => "Event Log",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    DurationDesc,
    DurationAsc,
    TimeDesc,
    TimeAsc,
}

impl SortOrder {
    pub fn label(&self) -> &'static str {
        match self {
            Self::DurationDesc => "Duration ↓",
            Self::DurationAsc => "Duration ↑",
            Self::TimeDesc => "Time ↓",
            Self::TimeAsc => "Time ↑",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::DurationDesc => Self::TimeDesc,
            Self::TimeDesc => Self::DurationAsc,
            Self::DurationAsc => Self::TimeAsc,
            Self::TimeAsc => Self::DurationDesc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filter,
}

pub struct App {
    pub current_tab: Tab,
    pub window: EpisodeWindow,
    pub is_paused: bool,
    pub input_mode: InputMode,
    pub filter_text: String,
    pub containers_only: bool,
    pub container_filter: Option<String>,
    pub sort_order: SortOrder,
    pub selected_index: usize,
    pub show_help: bool,
    pub show_detail: bool,
    pub status_message: Option<(String, Instant)>,
    pub events_received: u64,
    pub should_quit: bool,
    pub start_time: Instant,
}

impl App {
    pub fn new(window_duration_ns: u64) -> Self {
        Self {
            current_tab: Tab::Waterfall,
            window: EpisodeWindow::new(window_duration_ns),
            is_paused: false,
            input_mode: InputMode::Normal,
            filter_text: String::new(),
            containers_only: false,
            container_filter: None,
            sort_order: SortOrder::DurationDesc,
            selected_index: 0,
            show_help: false,
            show_detail: false,
            status_message: None,
            events_received: 0,
            should_quit: false,
            start_time: Instant::now(),
        }
    }

    pub fn on_episode(&mut self, ep: Episode) {
        self.events_received += 1;
        if !self.is_paused {
            self.window.push(ep);
        }
    }

    /// Return filtered and sorted list of episodes for rendering.
    pub fn filtered_episodes(&self) -> Vec<&Episode> {
        let filter = self.filter_text.trim().to_lowercase();
        let mut list: Vec<&Episode> = self
            .window
            .episodes()
            .iter()
            .filter(|ep| {
                if self.containers_only && ep.container_id.is_none() {
                    return false;
                }
                if let Some(target_cid) = &self.container_filter {
                    let matches_cid = ep
                        .container_id
                        .as_ref()
                        .map(|cid| cid.starts_with(target_cid) || cid.contains(target_cid))
                        .unwrap_or(false);
                    if !matches_cid {
                        return false;
                    }
                }
                if filter.is_empty() {
                    return true;
                }
                if ep.comm.to_lowercase().contains(&filter)
                    || ep.tid.to_string().contains(&filter)
                    || ep.tgid.to_string().contains(&filter)
                {
                    return true;
                }
                if let Some(cid) = &ep.container_id {
                    if cid.to_lowercase().contains(&filter) {
                        return true;
                    }
                }
                if let Some(path) = &ep.cgroup_path {
                    if path.to_lowercase().contains(&filter) {
                        return true;
                    }
                }
                if let Some(pod) = &ep.pod_name {
                    if pod.to_lowercase().contains(&filter) {
                        return true;
                    }
                }
                if ep
                    .culprits
                    .iter()
                    .any(|c| c.comm.to_lowercase().contains(&filter))
                {
                    return true;
                }
                if ep
                    .segments
                    .iter()
                    .any(|s| s.cause.as_str().to_lowercase().contains(&filter))
                {
                    return true;
                }
                false
            })
            .collect();

        match self.sort_order {
            SortOrder::DurationDesc => list.sort_by_key(|a| std::cmp::Reverse(a.total_delay_ns)),
            SortOrder::DurationAsc => list.sort_by_key(|a| a.total_delay_ns),
            SortOrder::TimeDesc => list.sort_by_key(|a| std::cmp::Reverse(a.start_ns)),
            SortOrder::TimeAsc => list.sort_by_key(|a| a.start_ns),
        }

        list
    }

    pub fn toggle_containers_only(&mut self) {
        self.containers_only = !self.containers_only;
        let msg = if self.containers_only {
            "Filter: CONTAINERS ONLY [Active]"
        } else {
            "Filter: ALL CGROUPS & TASKS [Active]"
        };
        self.set_status(msg);
        self.selected_index = 0;
    }

    pub fn selected_episode(&self) -> Option<&Episode> {
        let items = self.filtered_episodes();
        if items.is_empty() {
            None
        } else {
            let idx = self.selected_index.min(items.len().saturating_sub(1));
            Some(items[idx])
        }
    }

    pub fn next_row(&mut self) {
        let count = self.filtered_episodes().len();
        if count > 0 && self.selected_index + 1 < count {
            self.selected_index += 1;
        }
    }

    pub fn prev_row(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn next_tab(&mut self) {
        let all = Tab::all();
        let idx = all.iter().position(|&t| t == self.current_tab).unwrap_or(0);
        self.current_tab = all[(idx + 1) % all.len()];
        self.selected_index = 0;
    }

    pub fn prev_tab(&mut self) {
        let all = Tab::all();
        let idx = all.iter().position(|&t| t == self.current_tab).unwrap_or(0);
        self.current_tab = all[(idx + all.len() - 1) % all.len()];
        self.selected_index = 0;
    }

    pub fn toggle_pause(&mut self) {
        self.is_paused = !self.is_paused;
        let msg = if self.is_paused {
            "Paused capture"
        } else {
            "Resumed capture"
        };
        self.set_status(msg);
    }

    pub fn toggle_sort(&mut self) {
        self.sort_order = self.sort_order.next();
        self.set_status(format!("Sort: {}", self.sort_order.label()));
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
        if self.show_help {
            self.show_detail = false;
        }
    }

    pub fn toggle_detail(&mut self) {
        if self.show_detail {
            self.show_detail = false;
        } else if self.selected_episode().is_some() {
            self.show_detail = true;
            self.show_help = false;
        }
    }

    pub fn close_overlays(&mut self) -> bool {
        if self.input_mode == InputMode::Filter {
            self.input_mode = InputMode::Normal;
            return true;
        }
        if self.show_help {
            self.show_help = false;
            return true;
        }
        if self.show_detail {
            self.show_detail = false;
            return true;
        }
        false
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some((msg.into(), Instant::now()));
    }

    pub fn get_active_status(&self) -> Option<&str> {
        if let Some((msg, time)) = &self.status_message {
            if time.elapsed().as_secs() < 4 {
                return Some(msg.as_str());
            }
        }
        None
    }

    /// Export currently buffered episodes to a Chrome Trace JSON file.
    pub fn export_snapshot(&mut self) -> Result<String> {
        let count = self.window.episodes().len();
        if count == 0 {
            self.set_status("No episodes to export");
            return Ok(String::new());
        }

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("kstall_trace_{}.json", timestamp);

        let eps: Vec<Episode> = self.window.episodes().iter().cloned().collect();
        let trace = ChromeTrace::from_episodes(&eps);
        let file = File::create(&filename)?;
        trace.write_to(BufWriter::new(file))?;

        let msg = format!("Exported {} episodes to {}", count, filename);
        self.set_status(&msg);
        Ok(filename)
    }

    pub fn rate_per_sec(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.05 {
            self.events_received as f64 / elapsed
        } else {
            0.0
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kstall_core::model::{CausalSegment, CauseKind, Culprit};

    fn make_test_episode(id: u64, tid: u32, comm: &str, delay_ns: u64) -> Episode {
        Episode {
            id,
            tid,
            tgid: tid,
            comm: comm.to_string(),
            cgroup_id: 1,
            cgroup_path: Some("/system.slice/test".to_string()),
            container_id: None,
            pod_name: None,
            namespace: None,
            start_ns: id * 1_000_000,
            end_ns: id * 1_000_000 + delay_ns,
            total_delay_ns: delay_ns,
            segments: vec![CausalSegment {
                cause: CauseKind::RunqueueWait,
                start_ns: id * 1_000_000,
                end_ns: id * 1_000_000 + delay_ns,
                duration_ns: delay_ns,
                cpu: 0,
                culprit: None,
            }],
            culprits: vec![Culprit {
                tid: 999,
                tgid: 999,
                comm: "noisy_neighbor".to_string(),
                cgroup_id: 2,
                stolen_ns: delay_ns,
                flags: 0,
            }],
        }
    }

    #[test]
    fn test_app_filtering_and_sorting() {
        let mut app = App::new(60_000_000_000);
        app.on_episode(make_test_episode(1, 100, "postgres", 10_000_000));
        app.on_episode(make_test_episode(2, 200, "redis-server", 50_000_000));
        app.on_episode(make_test_episode(3, 300, "worker", 20_000_000));

        assert_eq!(app.filtered_episodes().len(), 3);
        // Default sort DurationDesc
        assert_eq!(app.filtered_episodes()[0].comm, "redis-server");
        assert_eq!(app.filtered_episodes()[1].comm, "worker");
        assert_eq!(app.filtered_episodes()[2].comm, "postgres");

        // Filter by comm
        app.filter_text = "post".to_string();
        let filtered = app.filtered_episodes();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].comm, "postgres");

        // Filter by culprit
        app.filter_text = "noisy".to_string();
        assert_eq!(app.filtered_episodes().len(), 3);

        // Sort toggle
        app.filter_text.clear();
        app.toggle_sort(); // TimeDesc
        assert_eq!(app.sort_order, SortOrder::TimeDesc);
        assert_eq!(app.filtered_episodes()[0].id, 3);
    }

    #[test]
    fn test_app_navigation_and_selection() {
        let mut app = App::new(60_000_000_000);
        app.on_episode(make_test_episode(1, 100, "task1", 10_000_000));
        app.on_episode(make_test_episode(2, 200, "task2", 20_000_000));

        assert_eq!(app.selected_index, 0);
        assert_eq!(app.selected_episode().unwrap().comm, "task2"); // sorted by duration desc

        app.next_row();
        assert_eq!(app.selected_index, 1);
        assert_eq!(app.selected_episode().unwrap().comm, "task1");

        // Boundary check
        app.next_row();
        assert_eq!(app.selected_index, 1);

        app.prev_row();
        assert_eq!(app.selected_index, 0);

        // Drilldown
        app.toggle_detail();
        assert!(app.show_detail);
        assert!(app.close_overlays());
        assert!(!app.show_detail);
    }

    #[test]
    fn test_app_containers_only_filter() {
        let mut app = App::new(60_000_000_000);
        let mut ep_host = make_test_episode(1, 100, "host_task", 10_000_000);
        ep_host.container_id = None;

        let mut ep_container = make_test_episode(2, 200, "docker_app", 20_000_000);
        ep_container.container_id = Some("abc123def456".to_string());

        app.on_episode(ep_host);
        app.on_episode(ep_container);

        // All tasks
        assert_eq!(app.filtered_episodes().len(), 2);

        // Toggle containers only
        app.toggle_containers_only();
        assert!(app.containers_only);
        let filtered = app.filtered_episodes();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].comm, "docker_app");
        assert_eq!(filtered[0].container_id.as_deref(), Some("abc123def456"));
    }
}
