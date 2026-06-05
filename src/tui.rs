use crate::{
    model::{DateFilter, Granularity, Summary, UsageRecord, aggregate},
    output::compact_tokens,
};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
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
        Paragraph, Row, Table, Tabs,
    },
};
use std::{io, time::Duration};

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

pub fn run(records: Vec<UsageRecord>, filter: DateFilter, granularity: Granularity) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = App::new(records, filter, granularity).run(&mut terminal);
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
}

impl App {
    fn new(records: Vec<UsageRecord>, filter: DateFilter, granularity: Granularity) -> Self {
        let summary = aggregate(&records, &filter, granularity);
        Self {
            records,
            filter,
            summary,
            tab: 0,
            granularity,
            selected_bucket: 0,
            drill_stack: Vec::new(),
        }
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(160))?
                && let Event::Key(key) = event::read()?
            {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
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
            }
        }
        Ok(())
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

    fn set_granularity(&mut self, granularity: Granularity) {
        if self.granularity == granularity {
            return;
        }
        self.drill_stack.clear();
        self.granularity = granularity;
        self.selected_bucket = 0;
        self.recompute();
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
        f.render_widget(tabs, area);
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
        let cached = self.summary.total.cached_input_tokens + self.summary.total.cache_write_tokens;
        let stats = [
            (
                "TOTAL",
                compact_tokens(self.summary.total.total_tokens),
                GREEN,
            ),
            (
                "INPUT",
                compact_tokens(self.summary.total.input_tokens),
                CYAN,
            ),
            ("CACHED", compact_tokens(cached), PURPLE),
            (
                "OUTPUT",
                compact_tokens(self.summary.total.output_tokens),
                YELLOW,
            ),
            ("CALLS", self.summary.total.api_calls.to_string(), PINK),
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
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        self.draw_timeline_chart(f, chunks[0]);
        self.draw_bucket_table(f, chunks[1]);
    }

    fn draw_timeline_chart(&self, f: &mut ratatui::Frame, area: Rect) {
        let models = self.summary.by_model.keys().cloned().collect::<Vec<_>>();
        let raw_series = models
            .iter()
            .map(|model| {
                self.summary
                    .buckets
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
        let marker = [
            (self.selected_bucket as f64, 0.0),
            (self.selected_bucket as f64, 100.0),
        ];
        datasets.push(
            Dataset::default()
                .name("selection")
                .marker(symbols::Marker::Dot)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::White))
                .data(&marker),
        );
        let labels = self
            .summary
            .buckets
            .iter()
            .map(|b| Span::raw(b.key.clone()))
            .collect::<Vec<_>>();
        let chart = Chart::new(datasets)
            .block(fancy_block(" ◇ normalized token trend "))
            .x_axis(
                Axis::default()
                    .bounds([
                        0.0,
                        self.summary.buckets.len().saturating_sub(1).max(1) as f64,
                    ])
                    .labels(labels.into_iter().take(7).collect::<Vec<_>>())
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

    fn draw_bucket_table(&self, f: &mut ratatui::Frame, area: Rect) {
        let rows = self.summary.buckets.iter().enumerate().map(|(i, bucket)| {
            let top_model = bucket
                .by_model
                .iter()
                .max_by_key(|(_, usage)| usage.total_tokens)
                .map(|(model, usage)| format!("{}  {}", model, compact_tokens(usage.total_tokens)))
                .unwrap_or_else(|| "-".to_string());
            let mut row = Row::new(vec![
                if i == self.selected_bucket {
                    format!("▶ {}", bucket.key)
                } else {
                    format!("  {}", bucket.key)
                },
                compact_tokens(bucket.usage.total_tokens),
                compact_tokens(bucket.usage.input_tokens),
                compact_tokens(bucket.usage.cached_input_tokens + bucket.usage.cache_write_tokens),
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
        .block(fancy_block(
            " tabular data — enter drills down, u drills up ",
        ));
        f.render_widget(table, area);
    }

    fn draw_models(&self, f: &mut ratatui::Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        let max_tokens = self
            .summary
            .by_model
            .values()
            .map(|usage| usage.total_tokens)
            .max()
            .unwrap_or(1);
        let bars: Vec<Bar> = self
            .summary
            .by_model
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

        let rows = self.summary.by_model.iter().map(|(model, usage)| {
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
        .block(fancy_block(" model table "));
        f.render_widget(table, chunks[1]);
    }

    fn draw_sources(&self, f: &mut ratatui::Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);
        let max_tokens = self
            .summary
            .by_source
            .values()
            .map(|usage| usage.total_tokens)
            .max()
            .unwrap_or(1);
        let bars: Vec<Bar> = self
            .summary
            .by_source
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

        let rows = self.summary.by_source.iter().map(|(source, usage)| {
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
        .block(fancy_block(" source table "));
        f.render_widget(table, chunks[1]);
    }

    fn draw_footer(&self, f: &mut ratatui::Frame, area: Rect) {
        let text = Line::from(vec![
            Span::styled(
                " ←/→ ",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled("period  ", Style::default().fg(MUTED)),
            Span::styled(
                "Enter",
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" drill  ", Style::default().fg(MUTED)),
            Span::styled("u", Style::default().fg(PINK).add_modifier(Modifier::BOLD)),
            Span::styled(" up  ", Style::default().fg(MUTED)),
            Span::styled(
                "1/2/3",
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" tabs  ", Style::default().fg(MUTED)),
            Span::styled(
                "y/m/w/d",
                Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" period  q quit", Style::default().fg(MUTED)),
        ]);
        f.render_widget(Paragraph::new(text).style(Style::default().bg(BG)), area);
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
