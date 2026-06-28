use crate::{
    model::{DateFilter, Granularity, Summary, Usage, UsageRecord, aggregate},
    output::compact_tokens,
    quota::{
        CodexQuota, CodexResetCredits, fetch_codex_quota, fetch_codex_reset_credits,
        format_quota_label, format_reset_credit_chart_label,
    },
};
use anyhow::Result;
use chrono::TimeZone;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Axis, Bar, BarChart, BarGroup, Block, BorderType, Borders, Chart, Dataset, GraphType,
        LegendPosition, Paragraph, Row, Table, Tabs, Wrap,
    },
};
use std::{
    collections::BTreeSet,
    io,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

const BG: Color = Color::Rgb(5, 7, 13);
const PANEL: Color = Color::Rgb(9, 13, 25);
const CARD: Color = Color::Rgb(13, 19, 36);
const BORDER: Color = Color::Rgb(61, 75, 112);
const MUTED: Color = Color::Rgb(127, 143, 175);
const TEXT: Color = Color::Rgb(215, 226, 255);
const CYAN: Color = Color::Rgb(0, 229, 255);
const PINK: Color = Color::Rgb(255, 92, 192);
const GREEN: Color = Color::Rgb(102, 255, 139);
const YELLOW: Color = Color::Rgb(255, 213, 74);
const PURPLE: Color = Color::Rgb(174, 129, 255);

pub fn run(
    records: Vec<UsageRecord>,
    filter: DateFilter,
    granularity: Granularity,
    codex_home: Option<PathBuf>,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = App::new(records, filter, granularity, codex_home).run(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

#[derive(Clone)]
struct DrillState {
    filter: DateFilter,
    granularity: Granularity,
    selected_bucket: usize,
}

struct App {
    records: Vec<UsageRecord>,
    filter: DateFilter,
    summary: Summary,
    tab: usize,
    granularity: Granularity,
    selected_bucket: usize,
    drill_stack: Vec<DrillState>,
    quota: Option<CodexQuota>,
    quota_rx: Option<Receiver<Option<CodexQuota>>>,
    reset_credits: Option<CodexResetCredits>,
    reset_credits_rx: Option<Receiver<Option<CodexResetCredits>>>,
}

impl App {
    fn new(
        records: Vec<UsageRecord>,
        filter: DateFilter,
        granularity: Granularity,
        codex_home: Option<PathBuf>,
    ) -> Self {
        let summary = aggregate(&records, &filter, granularity);
        let selected_bucket = selected_bucket_for_now(&summary, chrono::Utc::now());
        Self {
            records,
            filter,
            summary,
            tab: 0,
            granularity,
            selected_bucket,
            drill_stack: Vec::new(),
            quota: None,
            quota_rx: spawn_quota_probe(),
            reset_credits: None,
            reset_credits_rx: spawn_reset_credits_probe(codex_home),
        }
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            self.poll_quota_probe();
            self.poll_reset_credits_probe();
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(160))?
                && let Event::Key(key) = event::read()?
                && !self.handle_key(key)
            {
                break;
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Tab => self.tab = (self.tab + 1).min(2),
            KeyCode::BackTab => self.tab = self.tab.saturating_sub(1),
            KeyCode::Char('[') => self.tab = self.tab.saturating_sub(1),
            KeyCode::Char(']') => self.tab = (self.tab + 1).min(2),
            KeyCode::Char('1') => self.tab = 0,
            KeyCode::Char('2') => self.tab = 1,
            KeyCode::Char('3') => self.tab = 2,
            KeyCode::Left | KeyCode::Char('h') => self.move_bucket(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_bucket(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.drill_down(),
            KeyCode::Backspace | KeyCode::Char('u') => self.drill_up(),
            KeyCode::Char('y') => self.set_granularity(Granularity::Year),
            KeyCode::Char('m') => self.set_granularity(Granularity::Month),
            KeyCode::Char('w') => self.set_granularity(Granularity::Week),
            KeyCode::Char('d') => self.set_granularity(Granularity::Day),
            _ => {}
        }
        true
    }

    fn poll_quota_probe(&mut self) {
        let Some(rx) = &self.quota_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(quota) => {
                self.quota = quota;
                self.quota_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.quota_rx = None;
            }
        }
    }

    fn poll_reset_credits_probe(&mut self) {
        let Some(rx) = &self.reset_credits_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(reset_credits) => {
                self.reset_credits = reset_credits;
                self.reset_credits_rx = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.reset_credits_rx = None;
            }
        }
    }

    fn recompute(&mut self) {
        self.summary = aggregate(&self.records, &self.filter, self.granularity);
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        self.selected_bucket = if self.summary.buckets.is_empty() {
            0
        } else {
            self.selected_bucket.min(self.summary.buckets.len() - 1)
        };
    }

    fn move_bucket(&mut self, delta: isize) {
        if self.summary.buckets.is_empty() {
            self.selected_bucket = 0;
            return;
        }
        let max = self.summary.buckets.len() - 1;
        self.selected_bucket = if delta.is_negative() {
            self.selected_bucket.saturating_sub(delta.unsigned_abs())
        } else {
            (self.selected_bucket + delta as usize).min(max)
        };
    }

    fn selected_model_usage(&self) -> &std::collections::BTreeMap<String, Usage> {
        self.summary
            .buckets
            .get(self.selected_bucket)
            .map(|bucket| &bucket.by_model)
            .unwrap_or(&self.summary.by_model)
    }

    fn selected_source_usage(&self) -> &std::collections::BTreeMap<String, Usage> {
        self.summary
            .buckets
            .get(self.selected_bucket)
            .map(|bucket| &bucket.by_source)
            .unwrap_or(&self.summary.by_source)
    }

    fn selected_usage(&self) -> &Usage {
        self.summary
            .buckets
            .get(self.selected_bucket)
            .map(|bucket| &bucket.usage)
            .unwrap_or(&self.summary.total)
    }

    fn set_granularity(&mut self, granularity: Granularity) {
        if self.granularity == granularity {
            return;
        }
        self.drill_stack.clear();
        self.granularity = granularity;
        self.recompute();
        self.select_default_bucket();
    }

    fn drill_down(&mut self) {
        let Some(bucket) = self.summary.buckets.get(self.selected_bucket) else {
            return;
        };
        let next = match self.granularity {
            Granularity::Year => Granularity::Month,
            Granularity::Month => Granularity::Week,
            Granularity::Week => Granularity::Day,
            Granularity::Day => return,
        };
        self.drill_stack.push(DrillState {
            filter: self.filter.clone(),
            granularity: self.granularity,
            selected_bucket: self.selected_bucket,
        });
        self.filter = DateFilter {
            from: Some(bucket.start),
            to: Some(bucket.end),
        };
        self.granularity = next;
        self.selected_bucket = 0;
        self.recompute();
        self.select_default_bucket();
    }

    fn drill_up(&mut self) {
        let Some(prev) = self.drill_stack.pop() else {
            return;
        };
        self.filter = prev.filter;
        self.granularity = prev.granularity;
        self.selected_bucket = prev.selected_bucket;
        self.recompute();
    }

    fn draw(&self, f: &mut ratatui::Frame) {
        let frame = f.area();
        let root = Block::default()
            .style(Style::default().bg(BG).fg(TEXT))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(33, 44, 72)))
            .title(Line::from(vec![
                Span::styled(" ◆ ", Style::default().fg(PINK)),
                Span::styled(
                    "dexuse",
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" / explore", Style::default().fg(MUTED)),
            ]));
        f.render_widget(root, frame);
        let area = frame.inner(Margin {
            vertical: 1,
            horizontal: 2,
        });
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(5),
                Constraint::Min(18),
                Constraint::Length(2),
            ])
            .split(area);
        self.draw_header(f, chunks[0]);
        self.draw_summary(f, chunks[1]);
        match self.tab {
            0 => self.draw_timeline(f, chunks[2]),
            1 => self.draw_models(f, chunks[2]),
            _ => self.draw_sources(f, chunks[2]),
        }
        self.draw_footer(f, chunks[3]);
    }

    fn draw_header(&self, f: &mut ratatui::Frame, area: Rect) {
        let header = if self.quota.is_some() && area.width >= 108 {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(45), Constraint::Length(62)])
                .split(area)
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(100)])
                .split(area)
        };
        let tabs = Tabs::new(vec!["  Timeline  ", "  Models  ", "  Sources  "])
            .select(self.tab)
            .block(fancy_block(self.period_title()))
            .style(Style::default().fg(MUTED).bg(PANEL))
            .highlight_style(
                Style::default()
                    .fg(CYAN)
                    .bg(Color::Rgb(18, 34, 58))
                    .add_modifier(Modifier::BOLD),
            )
            .divider(Span::styled(" │ ", Style::default().fg(PINK)));
        f.render_widget(tabs, header[0]);

        if let Some(quota) = &self.quota
            && header.len() > 1
        {
            let label = format!("{}   ", format_quota_label(quota));
            let text = Line::from(vec![
                Span::styled("◷ ", Style::default().fg(PINK)),
                Span::styled(
                    label,
                    Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                ),
            ]);
            f.render_widget(
                Paragraph::new(text)
                    .alignment(ratatui::layout::Alignment::Right)
                    .block(fancy_block(" Codex remaining ")),
                header[1],
            );
        }
    }

    fn draw_summary(&self, f: &mut ratatui::Frame, area: Rect) {
        let cards = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
                Constraint::Percentage(20),
            ])
            .split(area);
        let usage = self.selected_usage();
        let cached = usage.cached_input_tokens + usage.cache_write_tokens;
        let stats = [
            ("TOTAL", compact_tokens(usage.total_tokens), GREEN),
            ("INPUT", compact_tokens(usage.input_tokens), CYAN),
            ("CACHED", compact_tokens(cached), PURPLE),
            ("OUTPUT", compact_tokens(usage.output_tokens), YELLOW),
            ("CALLS", usage.api_calls.to_string(), PINK),
        ];
        for (i, (label, value, color)) in stats.into_iter().enumerate() {
            let text = vec![
                Line::from(Span::styled(label, Style::default().fg(MUTED))),
                Line::from(Span::styled(
                    value,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
            ];
            f.render_widget(
                Paragraph::new(text).block(
                    fancy_block("")
                        .border_style(Style::default().fg(color))
                        .style(Style::default().bg(CARD)),
                ),
                cards[i],
            );
        }
    }

    fn draw_timeline(&self, f: &mut ratatui::Frame, area: Rect) {
        let reset_panel_height = self.reset_credit_panel_height(area);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(reset_panel_height),
                Constraint::Min(7),
            ])
            .split(area);
        let (window_start, window_end) = self.timeline_bucket_window(chunks[2]);
        self.draw_timeline_chart(f, chunks[0], window_start, window_end);
        if reset_panel_height > 0 {
            self.draw_reset_credits(f, chunks[1]);
        }
        self.draw_bucket_table(f, chunks[2], window_start, window_end);
    }

    fn reset_credit_panel_height(&self, area: Rect) -> u16 {
        let Some(reset_credits) = &self.reset_credits else {
            return 0;
        };
        if reset_credits.available_count == 0 {
            return 0;
        }
        let visible_rows = reset_credits.credits.len().clamp(1, 3) as u16;
        let desired = visible_rows + 2;
        let max = area.height.saturating_sub(15).clamp(0, 5);
        desired.min(max)
    }

    fn draw_reset_credits(&self, f: &mut ratatui::Frame, area: Rect) {
        let Some(reset_credits) = &self.reset_credits else {
            return;
        };
        let inner_rows = area.height.saturating_sub(2) as usize;
        if inner_rows == 0 {
            return;
        }
        let text = self.reset_credit_text_lines(inner_rows);
        let block_title = if reset_credits.available_count == 1 {
            " Codex reset credit "
        } else {
            " Codex reset credits "
        };
        f.render_widget(
            Paragraph::new(text)
                .block(fancy_block(block_title).border_style(Style::default().fg(PINK)))
                .wrap(Wrap { trim: true }),
            area,
        );
    }

    fn reset_credit_text_lines(&self, max_lines: usize) -> Vec<Line<'static>> {
        let Some(reset_credits) = &self.reset_credits else {
            return Vec::new();
        };
        if max_lines == 0 {
            return Vec::new();
        }
        if reset_credits.credits.is_empty() {
            return vec![Line::from(vec![
                Span::styled(
                    reset_credits.available_count.to_string(),
                    Style::default().fg(PINK).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " banked; expiry details unavailable",
                    Style::default().fg(MUTED),
                ),
            ])];
        }

        let detail_lines = max_lines.min(reset_credits.credits.len());
        let mut lines = reset_credits
            .credits
            .iter()
            .take(detail_lines)
            .enumerate()
            .map(|(i, credit)| {
                Line::from(vec![
                    Span::styled(
                        format!("Reset {} ", i + 1),
                        Style::default().fg(PINK).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(credit.label.clone(), Style::default().fg(TEXT)),
                ])
            })
            .collect::<Vec<_>>();

        let total_resets = reset_credits
            .available_count
            .max(reset_credits.credits.len());
        let mut hidden = total_resets.saturating_sub(detail_lines);
        if hidden > 0 {
            if lines.len() == max_lines {
                lines.pop();
                hidden += 1;
            }
            lines.push(Line::from(vec![
                Span::styled(format!("+{hidden} "), Style::default().fg(PINK)),
                Span::styled("more banked reset credits", Style::default().fg(MUTED)),
            ]));
        }
        lines
    }

    fn draw_timeline_chart(
        &self,
        f: &mut ratatui::Frame,
        area: Rect,
        window_start: usize,
        window_end: usize,
    ) {
        let buckets = &self.summary.buckets[window_start..window_end];
        let models = buckets
            .iter()
            .flat_map(|bucket| bucket.by_model.keys().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let raw_series = models
            .iter()
            .map(|model| {
                buckets
                    .iter()
                    .enumerate()
                    .map(|(i, bucket)| {
                        let value = bucket
                            .by_model
                            .get(model)
                            .map(|usage| usage.total_tokens)
                            .unwrap_or(0) as f64;
                        (i as f64, value)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let model_series = raw_series
            .iter()
            .map(|series| {
                let max = series.iter().map(|(_, y)| *y).fold(1.0, f64::max);
                series
                    .iter()
                    .map(|(x, y)| (*x, (*y / max) * 100.0))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut datasets = models
            .iter()
            .enumerate()
            .map(|(i, model)| {
                Dataset::default()
                    .name(model.clone())
                    .marker(symbols::Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(color(i)))
                    .data(&model_series[i])
            })
            .collect::<Vec<_>>();
        let selected_x = self.selected_bucket.saturating_sub(window_start) as f64;
        let marker = [(selected_x, 0.0), (selected_x, 100.0)];
        if !buckets.is_empty() {
            datasets.push(
                Dataset::default()
                    .marker(symbols::Marker::Dot)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(Color::White))
                    .data(&marker),
            );
        }
        let labels = sampled_chart_labels(buckets.iter().map(|bucket| bucket.key.as_str()));
        let reset_markers = self.reset_credit_marker_series(buckets, window_start);
        for (i, marker) in reset_markers.iter().enumerate() {
            datasets.push(
                Dataset::default()
                    .marker(symbols::Marker::Dot)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(reset_marker_color(i)))
                    .data(marker),
            );
        }

        let chart = Chart::new(datasets)
            .block(fancy_block(self.timeline_chart_title()))
            .legend_position(Some(LegendPosition::TopRight))
            .hidden_legend_constraints((Constraint::Min(1), Constraint::Min(1)))
            .x_axis(
                Axis::default()
                    .bounds([0.0, buckets.len().saturating_sub(1).max(1) as f64])
                    .labels(labels)
                    .style(Style::default().fg(MUTED)),
            )
            .y_axis(
                Axis::default()
                    .bounds([0.0, 100.0])
                    .labels(vec![Span::raw("0%"), Span::raw("50%"), Span::raw("100%")])
                    .style(Style::default().fg(MUTED)),
            );
        f.render_widget(chart, area);
    }

    fn reset_credit_marker_series(
        &self,
        buckets: &[crate::model::Bucket],
        window_start: usize,
    ) -> Vec<Vec<(f64, f64)>> {
        let Some(reset_credits) = &self.reset_credits else {
            return Vec::new();
        };
        reset_credits
            .credits
            .iter()
            .filter_map(|credit| {
                let expires_at = chrono::Utc.timestamp_opt(credit.expires_at, 0).single()?;
                let (offset, bucket) = buckets
                    .iter()
                    .enumerate()
                    .find(|(_, bucket)| bucket.start <= expires_at && expires_at < bucket.end)?;
                let span = (bucket.end - bucket.start).num_seconds().max(1) as f64;
                let elapsed = (expires_at - bucket.start)
                    .num_seconds()
                    .clamp(0, span as i64) as f64;
                let x = offset as f64 + (elapsed / span);
                let min_x = 0.0;
                let max_x = buckets.len().saturating_sub(1).max(1) as f64;
                let x = x.clamp(min_x, max_x);
                let bucket_index = window_start + offset;
                Some(vec![
                    (x, 0.0),
                    (
                        x,
                        if bucket_index == self.selected_bucket {
                            100.0
                        } else {
                            96.0
                        },
                    ),
                ])
            })
            .collect()
    }

    fn timeline_chart_title(&self) -> String {
        let mut title = " ◇ normalized token trend — legend: model color ".to_string();
        if let Some(reset_credits) = &self.reset_credits
            && reset_credits.available_count > 0
        {
            title.push_str("— resets ");
            title.push_str(&format_reset_credit_chart_label(reset_credits, 3));
            title.push(' ');
        }
        title
    }

    fn draw_bucket_table(
        &self,
        f: &mut ratatui::Frame,
        area: Rect,
        window_start: usize,
        window_end: usize,
    ) {
        let rows = self.summary.buckets[window_start..window_end]
            .iter()
            .enumerate()
            .map(|(offset, bucket)| {
                let i = window_start + offset;
                let top_model = bucket
                    .by_model
                    .iter()
                    .max_by_key(|(_, usage)| usage.total_tokens)
                    .map(|(model, usage)| {
                        format!("{}  {}", model, compact_tokens(usage.total_tokens))
                    })
                    .unwrap_or_else(|| "-".to_string());
                let mut row = Row::new(vec![
                    if i == self.selected_bucket {
                        format!("▶ {}", bucket.key)
                    } else {
                        format!("  {}", bucket.key)
                    },
                    compact_tokens(bucket.usage.total_tokens),
                    compact_tokens(bucket.usage.input_tokens),
                    compact_tokens(
                        bucket.usage.cached_input_tokens + bucket.usage.cache_write_tokens,
                    ),
                    compact_tokens(bucket.usage.output_tokens),
                    bucket.usage.api_calls.to_string(),
                    top_model,
                ]);
                if i == self.selected_bucket {
                    row = row.style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(CYAN)
                            .add_modifier(Modifier::BOLD),
                    );
                }
                row
            });
        let table = Table::new(
            rows,
            [
                Constraint::Length(16),
                Constraint::Length(11),
                Constraint::Length(11),
                Constraint::Length(11),
                Constraint::Length(11),
                Constraint::Length(7),
                Constraint::Min(30),
            ],
        )
        .header(table_header(vec![
            "period",
            "total",
            "input",
            "cached",
            "output",
            "calls",
            "top model",
        ]))
        .block(fancy_block(self.bucket_table_title()));
        f.render_widget(table, area);
    }

    fn select_default_bucket(&mut self) {
        self.selected_bucket = selected_bucket_for_now(&self.summary, chrono::Utc::now());
    }

    fn timeline_bucket_window(&self, table_area: Rect) -> (usize, usize) {
        let len = self.summary.buckets.len();
        if len == 0 {
            return (0, 0);
        }
        let max_rows = usize::from(table_area.height.saturating_sub(4)).max(1);
        if max_rows >= len {
            return (0, len);
        }
        let selected = self.selected_bucket.min(len - 1);
        let mut start = selected.saturating_sub(max_rows / 2);
        if start + max_rows > len {
            start = len - max_rows;
        }
        (start, start + max_rows)
    }

    fn bucket_table_title(&self) -> String {
        let mut title = " tabular data".to_string();
        let mut hints = Vec::new();
        if self.granularity != Granularity::Day {
            hints.push("enter drills down");
        }
        if !self.drill_stack.is_empty() {
            hints.push("u drills up");
        }
        if !hints.is_empty() {
            title.push_str(" — ");
            title.push_str(&hints.join(", "));
        }
        title.push(' ');
        title
    }

    fn draw_models(&self, f: &mut ratatui::Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        let model_usage = self.selected_model_usage();
        let max_tokens = model_usage
            .values()
            .map(|usage| usage.total_tokens)
            .max()
            .unwrap_or(1);
        let bars: Vec<Bar> = model_usage
            .iter()
            .enumerate()
            .map(|(i, (model, usage))| {
                let pct = (((usage.total_tokens as f64 + 1.0).log10()
                    / (max_tokens as f64 + 1.0).log10())
                    * 100.0)
                    .ceil() as u64;
                Bar::default()
                    .label(Line::from(model.clone()))
                    .value(pct.max(1))
                    .text_value(compact_tokens(usage.total_tokens).to_string())
                    .style(Style::default().fg(color(i)))
            })
            .collect();
        let chart = BarChart::default()
            .block(fancy_block(" ◇ model mix — log-scaled bars "))
            .data(BarGroup::default().bars(&bars))
            .bar_width(18)
            .bar_gap(2)
            .max(100)
            .value_style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            );
        f.render_widget(chart, chunks[0]);

        let rows = model_usage.iter().map(|(model, usage)| {
            Row::new(vec![
                model.clone(),
                compact_tokens(usage.total_tokens),
                compact_tokens(usage.input_tokens),
                compact_tokens(usage.cached_input_tokens + usage.cache_write_tokens),
                compact_tokens(usage.output_tokens),
                compact_tokens(usage.reasoning_tokens),
                usage.api_calls.to_string(),
            ])
        });
        let table = Table::new(
            rows,
            [
                Constraint::Min(24),
                Constraint::Length(11),
                Constraint::Length(11),
                Constraint::Length(11),
                Constraint::Length(11),
                Constraint::Length(11),
                Constraint::Length(7),
            ],
        )
        .header(table_header(vec![
            "model", "total", "input", "cached", "output", "reason", "calls",
        ]))
        .block(fancy_block(format!(
            " model table — {} ",
            self.selected_period_label()
        )));
        f.render_widget(table, chunks[1]);
    }

    fn draw_sources(&self, f: &mut ratatui::Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);
        let source_usage = self.selected_source_usage();
        let max_tokens = source_usage
            .values()
            .map(|usage| usage.total_tokens)
            .max()
            .unwrap_or(1);
        let bars: Vec<Bar> = source_usage
            .iter()
            .enumerate()
            .map(|(i, (source, usage))| {
                Bar::default()
                    .label(Line::from(source.clone()))
                    .value(((usage.total_tokens as f64 / max_tokens as f64) * 100.0).ceil() as u64)
                    .text_value(compact_tokens(usage.total_tokens).to_string())
                    .style(Style::default().fg(color(i)))
            })
            .collect();
        let chart = BarChart::default()
            .block(fancy_block(" ◇ source split "))
            .data(BarGroup::default().bars(&bars))
            .bar_width(18)
            .max(100)
            .value_style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            );
        f.render_widget(chart, chunks[0]);

        let rows = source_usage.iter().map(|(source, usage)| {
            Row::new(vec![
                source.clone(),
                compact_tokens(usage.total_tokens),
                compact_tokens(usage.input_tokens),
                compact_tokens(usage.cached_input_tokens + usage.cache_write_tokens),
                compact_tokens(usage.output_tokens),
                usage.api_calls.to_string(),
            ])
        });
        let table = Table::new(
            rows,
            [
                Constraint::Length(14),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Length(8),
            ],
        )
        .header(table_header(vec![
            "source", "total", "input", "cached", "output", "calls",
        ]))
        .block(fancy_block(format!(
            " source table — {} ",
            self.selected_period_label()
        )));
        f.render_widget(table, chunks[1]);
    }

    fn draw_footer(&self, f: &mut ratatui::Frame, area: Rect) {
        let text = Line::from(self.footer_hint_spans());
        f.render_widget(Paragraph::new(text).style(Style::default().bg(BG)), area);
    }

    #[cfg(test)]
    fn footer_hint_text(&self) -> String {
        self.footer_hint_spans()
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<Vec<_>>()
            .join("")
    }

    fn footer_hint_spans(&self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        self.push_footer_hint(&mut spans, "←/→", "period", CYAN);
        if self.granularity != Granularity::Day {
            self.push_footer_hint(&mut spans, "Enter", "drill", GREEN);
        }
        if !self.drill_stack.is_empty() {
            self.push_footer_hint(&mut spans, "u", "up", PINK);
        }
        self.push_footer_hint(&mut spans, "1/2/3", "tabs", YELLOW);
        self.push_footer_hint(&mut spans, "y/m/w/d", "period", PURPLE);
        spans.push(Span::styled(
            " q ",
            Style::default().fg(PINK).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("quit", Style::default().fg(MUTED)));
        spans
    }

    fn push_footer_hint(
        &self,
        spans: &mut Vec<Span<'static>>,
        key: &'static str,
        label: &'static str,
        color: Color,
    ) {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("{label}  "),
            Style::default().fg(MUTED),
        ));
    }

    fn period_title(&self) -> String {
        let scope = match (self.filter.from, self.filter.to) {
            (Some(from), Some(to)) => format!(
                "{} → {}",
                from.format("%Y-%m-%d"),
                (to - chrono::Duration::seconds(1)).format("%Y-%m-%d")
            ),
            _ => "all usage".to_string(),
        };
        format!(
            " {} • {:?} • depth {} ",
            scope,
            self.granularity,
            self.drill_stack.len()
        )
    }

    fn selected_period_label(&self) -> String {
        self.summary
            .buckets
            .get(self.selected_bucket)
            .map(|bucket| bucket.key.clone())
            .unwrap_or_else(|| "all usage".to_string())
    }
}

fn spawn_quota_probe() -> Option<Receiver<Option<CodexQuota>>> {
    if std::env::var_os("DEXUSE_DISABLE_CODEX_QUOTA").is_some() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(fetch_codex_quota());
    });
    Some(rx)
}

fn spawn_reset_credits_probe(
    codex_home: Option<PathBuf>,
) -> Option<Receiver<Option<CodexResetCredits>>> {
    if std::env::var_os("DEXUSE_DISABLE_CODEX_QUOTA").is_some()
        || std::env::var_os("DEXUSE_DISABLE_CODEX_RESET_CREDITS").is_some()
    {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(fetch_codex_reset_credits(codex_home.as_deref()));
    });
    Some(rx)
}

fn fancy_block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .title(title.into())
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .style(Style::default().bg(PANEL).fg(TEXT))
}

fn table_header(labels: Vec<&'static str>) -> Row<'static> {
    Row::new(labels)
        .style(Style::default().fg(YELLOW).add_modifier(Modifier::BOLD))
        .bottom_margin(1)
}

fn color(i: usize) -> Color {
    [CYAN, PINK, GREEN, YELLOW, PURPLE, Color::Rgb(92, 144, 255)][i % 6]
}

fn reset_marker_color(i: usize) -> Color {
    [Color::White, PINK, YELLOW, PURPLE, CYAN, GREEN][i % 6]
}

fn selected_bucket_for_now(summary: &Summary, now: chrono::DateTime<chrono::Utc>) -> usize {
    if summary.buckets.is_empty() {
        return 0;
    }
    summary
        .buckets
        .iter()
        .position(|bucket| bucket.start <= now && now < bucket.end)
        .or_else(|| {
            summary
                .buckets
                .iter()
                .rposition(|bucket| bucket.start <= now)
        })
        .unwrap_or(0)
}

fn sampled_chart_labels<'a>(keys: impl ExactSizeIterator<Item = &'a str>) -> Vec<Span<'static>> {
    const MAX_LABELS: usize = 7;

    let keys = keys.collect::<Vec<_>>();
    let len = keys.len();
    if len == 0 {
        return Vec::new();
    }
    if len <= MAX_LABELS {
        return keys
            .into_iter()
            .map(|key| Span::raw(key.to_string()))
            .collect();
    }

    let last = len - 1;
    (0..MAX_LABELS)
        .map(|i| (i * last) / (MAX_LABELS - 1))
        .scan(None, |previous, index| {
            if *previous == Some(index) {
                None
            } else {
                *previous = Some(index);
                Some(Span::raw(keys[index].to_string()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Source, Usage};
    use crate::quota::CodexResetCredit;
    use chrono::TimeZone;

    fn usage_record(day: u32, model: &str, source: Source, tokens: u64) -> UsageRecord {
        let mut usage = Usage {
            input_tokens: tokens,
            ..Usage::default()
        };
        usage.recompute_total();
        UsageRecord {
            timestamp: chrono::Utc
                .with_ymd_and_hms(2026, 6, day, 12, 0, 0)
                .unwrap(),
            source,
            provider: "openai-codex".to_string(),
            model: model.to_string(),
            session_id: format!("session-{day}-{model}"),
            title: None,
            usage,
        }
    }

    fn record(day: u32) -> UsageRecord {
        usage_record(day, "gpt-5.5", Source::Codex, 10)
    }

    fn key(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, crossterm::event::KeyModifiers::NONE, kind)
    }

    fn app_with_days(days: &[u32]) -> App {
        let records: Vec<_> = days.iter().copied().map(record).collect();
        app_with_records(records, Granularity::Day)
    }

    fn app_with_records(records: Vec<UsageRecord>, granularity: Granularity) -> App {
        let filter = DateFilter::default();
        let summary = aggregate(&records, &filter, granularity);
        App {
            records,
            filter,
            summary,
            tab: 0,
            granularity,
            selected_bucket: 0,
            drill_stack: Vec::new(),
            quota: None,
            quota_rx: None,
            reset_credits: None,
            reset_credits_rx: None,
        }
    }

    #[test]
    fn arrow_release_does_not_advance_period_twice() {
        let mut app = app_with_days(&[1, 2, 3]);

        app.handle_key(key(KeyCode::Right, KeyEventKind::Press));
        app.handle_key(key(KeyCode::Right, KeyEventKind::Release));

        assert_eq!(app.selected_bucket, 1);
    }

    #[test]
    fn default_period_selection_prefers_today_then_nearest_prior_bucket() {
        let filter = DateFilter::default();
        let summary = aggregate(
            &[record(1), record(16), record(17)],
            &filter,
            Granularity::Day,
        );
        let today = chrono::Utc.with_ymd_and_hms(2026, 6, 16, 18, 0, 0).unwrap();
        assert_eq!(selected_bucket_for_now(&summary, today), 1);

        let older_summary = aggregate(&[record(1), record(2)], &filter, Granularity::Day);
        assert_eq!(selected_bucket_for_now(&older_summary, today), 1);

        let future_summary = aggregate(&[record(20)], &filter, Granularity::Day);
        assert_eq!(selected_bucket_for_now(&future_summary, today), 0);
    }

    #[test]
    fn timeline_bucket_window_scrolls_with_selected_period_when_rows_overflow() {
        let mut app = app_with_days(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let short_table = Rect::new(0, 0, 80, 7);

        app.selected_bucket = 1;
        assert_eq!(app.timeline_bucket_window(short_table), (0, 3));

        app.selected_bucket = 5;
        assert_eq!(app.timeline_bucket_window(short_table), (4, 7));

        app.selected_bucket = 8;
        assert_eq!(app.timeline_bucket_window(short_table), (7, 10));
    }

    #[test]
    fn timeline_reset_credits_render_as_multiple_marker_lines() {
        let mut app = app_with_days(&[1, 2, 3]);
        app.reset_credits = Some(CodexResetCredits {
            available_count: 2,
            credits: vec![
                CodexResetCredit {
                    expires_at: chrono::Utc
                        .with_ymd_and_hms(2026, 6, 1, 12, 0, 0)
                        .unwrap()
                        .timestamp(),
                    label: "Jun 1 noon".to_string(),
                },
                CodexResetCredit {
                    expires_at: chrono::Utc
                        .with_ymd_and_hms(2026, 6, 2, 18, 0, 0)
                        .unwrap()
                        .timestamp(),
                    label: "Jun 2 evening".to_string(),
                },
            ],
            label: "Jun 1 noon • Jun 2 evening".to_string(),
        });

        let markers = app.reset_credit_marker_series(&app.summary.buckets, 0);

        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0], vec![(0.5, 0.0), (0.5, 100.0)]);
        assert_eq!(markers[1], vec![(1.75, 0.0), (1.75, 96.0)]);
        assert!(app.timeline_chart_title().contains("resets Jun 1 noon"));
    }

    #[test]
    fn reset_credit_panel_shows_actual_reset_text_and_overflow_count() {
        let mut app = app_with_days(&[1, 2, 3]);
        app.reset_credits = Some(CodexResetCredits {
            available_count: 4,
            credits: vec![
                CodexResetCredit {
                    expires_at: 1,
                    label: "Jul 12 9:08am".to_string(),
                },
                CodexResetCredit {
                    expires_at: 2,
                    label: "Jul 18 4:55am".to_string(),
                },
                CodexResetCredit {
                    expires_at: 3,
                    label: "Jul 27 4:08am".to_string(),
                },
            ],
            label: "Jul 12 9:08am • Jul 18 4:55am • Jul 27 4:08am +1".to_string(),
        });

        let text = app
            .reset_credit_text_lines(2)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            text,
            vec!["Reset 1 Jul 12 9:08am", "+3 more banked reset credits"]
        );
    }

    #[test]
    fn sampled_chart_labels_cover_the_same_window_start_and_end_as_the_table() {
        let keys = (1..=10)
            .map(|day| format!("2026-06-{day:02}"))
            .collect::<Vec<_>>();
        let labels = sampled_chart_labels(keys.iter().map(String::as_str))
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<Vec<_>>();

        assert_eq!(labels.first().map(String::as_str), Some("2026-06-01"));
        assert_eq!(labels.last().map(String::as_str), Some("2026-06-10"));
        assert_eq!(labels.len(), 7);
    }

    #[test]
    fn model_tab_breakdown_follows_selected_period_and_granularity_keys() {
        let mut app = app_with_records(
            vec![
                usage_record(1, "gpt-5.5", Source::Codex, 10),
                usage_record(2, "gpt-5.4", Source::Codex, 20),
                usage_record(3, "gpt-5.4-mini", Source::Codex, 30),
            ],
            Granularity::Day,
        );
        app.tab = 1;

        assert_eq!(app.selected_model_usage()["gpt-5.5"].total_tokens, 10);
        assert!(!app.selected_model_usage().contains_key("gpt-5.4"));

        app.handle_key(key(KeyCode::Right, KeyEventKind::Press));
        assert_eq!(app.selected_period_label(), "2026-06-02");
        assert_eq!(app.selected_model_usage()["gpt-5.4"].total_tokens, 20);
        assert!(!app.selected_model_usage().contains_key("gpt-5.5"));

        app.handle_key(key(KeyCode::Char('m'), KeyEventKind::Press));
        assert_eq!(app.granularity, Granularity::Month);
        assert_eq!(app.selected_period_label(), "2026-06");
        assert_eq!(app.selected_model_usage()["gpt-5.5"].total_tokens, 10);
        assert_eq!(app.selected_model_usage()["gpt-5.4"].total_tokens, 20);
        assert_eq!(app.selected_model_usage()["gpt-5.4-mini"].total_tokens, 30);
    }

    #[test]
    fn source_tab_breakdown_follows_selected_period_and_granularity_keys() {
        let mut app = app_with_records(
            vec![
                usage_record(1, "gpt-5.5", Source::Codex, 10),
                usage_record(2, "gpt-5.5", Source::Hermes, 20),
                usage_record(3, "gpt-5.5", Source::OpenClaw, 30),
            ],
            Granularity::Day,
        );
        app.tab = 2;

        assert_eq!(app.selected_source_usage()["codex"].total_tokens, 10);
        assert!(!app.selected_source_usage().contains_key("hermes"));

        app.handle_key(key(KeyCode::Right, KeyEventKind::Press));
        assert_eq!(app.selected_period_label(), "2026-06-02");
        assert_eq!(app.selected_source_usage()["hermes"].total_tokens, 20);
        assert!(!app.selected_source_usage().contains_key("codex"));

        app.handle_key(key(KeyCode::Char('m'), KeyEventKind::Press));
        assert_eq!(app.granularity, Granularity::Month);
        assert_eq!(app.selected_period_label(), "2026-06");
        assert_eq!(app.selected_source_usage()["codex"].total_tokens, 10);
        assert_eq!(app.selected_source_usage()["hermes"].total_tokens, 20);
        assert_eq!(app.selected_source_usage()["openclaw"].total_tokens, 30);
    }

    #[test]
    fn summary_cards_follow_selected_period_usage() {
        let mut app = app_with_records(
            vec![
                usage_record(1, "gpt-5.5", Source::Codex, 10),
                usage_record(2, "gpt-5.5", Source::Codex, 20),
                usage_record(3, "gpt-5.5", Source::Codex, 30),
            ],
            Granularity::Day,
        );

        assert_eq!(app.summary.total.total_tokens, 60);
        assert_eq!(app.selected_usage().total_tokens, 10);

        app.handle_key(key(KeyCode::Right, KeyEventKind::Press));
        assert_eq!(app.selected_period_label(), "2026-06-02");
        assert_eq!(app.selected_usage().total_tokens, 20);

        app.handle_key(key(KeyCode::Char('m'), KeyEventKind::Press));
        assert_eq!(app.selected_period_label(), "2026-06");
        assert_eq!(app.selected_usage().total_tokens, 60);
    }

    #[test]
    fn footer_hides_drill_and_up_keys_when_they_cannot_change_state() {
        let mut app = app_with_days(&[1, 2, 3]);
        app.granularity = Granularity::Day;
        app.recompute();

        let footer = app.footer_hint_text();

        assert!(!footer.contains("Enter"));
        assert!(!footer.contains(" u "));
        assert!(footer.contains("←/→"));
        assert!(footer.contains("y/m/w/d"));
    }

    #[test]
    fn footer_shows_drill_and_up_only_when_available_on_each_tab() {
        let mut app = app_with_records(vec![record(1), record(2)], Granularity::Week);
        app.tab = 1;
        assert!(app.footer_hint_text().contains("Enter"));
        assert!(!app.footer_hint_text().contains(" u "));

        app.drill_down();
        app.tab = 2;
        let footer = app.footer_hint_text();
        assert!(!footer.contains("Enter"));
        assert!(footer.contains(" u "));
        assert!(footer.contains("1/2/3"));
        assert!(footer.contains("y/m/w/d"));
    }

    #[test]
    fn bucket_table_title_hides_inactive_drill_keys() {
        let mut app = app_with_days(&[1, 2, 3]);
        assert!(!app.bucket_table_title().contains("enter"));
        assert!(!app.bucket_table_title().contains("u drills"));

        app = app_with_records(vec![record(1), record(2)], Granularity::Week);
        assert!(app.bucket_table_title().contains("enter drills down"));
        assert!(!app.bucket_table_title().contains("u drills up"));

        app.drill_down();
        assert!(!app.bucket_table_title().contains("enter drills down"));
        assert!(app.bucket_table_title().contains("u drills up"));
    }
}
