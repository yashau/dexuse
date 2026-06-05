use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Year,
    Month,
    Week,
    Day,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
    pub api_calls: u64,
    pub estimated_cost_usd: f64,
}

impl Usage {
    pub fn recompute_total(&mut self) {
        self.total_tokens = self.input_tokens
            + self.cached_input_tokens
            + self.cache_write_tokens
            + self.output_tokens
            + self.reasoning_tokens;
    }

    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.total_tokens += other.total_tokens;
        self.api_calls += other.api_calls;
        self.estimated_cost_usd += other.estimated_cost_usd;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub timestamp: DateTime<Utc>,
    pub source: Source,
    pub provider: String,
    pub model: String,
    pub session_id: String,
    pub title: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Codex,
    Hermes,
    OpenClaw,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bucket {
    pub key: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub usage: Usage,
    pub by_model: BTreeMap<String, Usage>,
    pub by_source: BTreeMap<String, Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub generated_at: DateTime<Utc>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub granularity: Granularity,
    pub total: Usage,
    pub by_model: BTreeMap<String, Usage>,
    pub by_provider: BTreeMap<String, Usage>,
    pub by_source: BTreeMap<String, Usage>,
    pub buckets: Vec<Bucket>,
    pub records: usize,
}

#[derive(Debug, Clone, Default)]
pub struct DateFilter {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

impl DateFilter {
    pub fn contains(&self, ts: DateTime<Utc>) -> bool {
        if let Some(from) = self.from
            && ts < from
        {
            return false;
        }
        if let Some(to) = self.to
            && ts >= to
        {
            return false;
        }
        true
    }
}

pub fn aggregate(
    records: &[UsageRecord],
    filter: &DateFilter,
    granularity: Granularity,
) -> Summary {
    let mut total = Usage::default();
    let mut by_model: BTreeMap<String, Usage> = BTreeMap::new();
    let mut by_provider: BTreeMap<String, Usage> = BTreeMap::new();
    let mut by_source: BTreeMap<String, Usage> = BTreeMap::new();
    let mut buckets: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut count = 0;

    for record in records.iter().filter(|r| filter.contains(r.timestamp)) {
        count += 1;
        total.add(&record.usage);
        by_model
            .entry(record.model.clone())
            .or_default()
            .add(&record.usage);
        by_provider
            .entry(record.provider.clone())
            .or_default()
            .add(&record.usage);
        by_source
            .entry(format!("{:?}", record.source).to_lowercase())
            .or_default()
            .add(&record.usage);

        let (key, start, end) = bucket_for(record.timestamp, granularity);
        let bucket = buckets.entry(key.clone()).or_insert_with(|| Bucket {
            key,
            start,
            end,
            usage: Usage::default(),
            by_model: BTreeMap::new(),
            by_source: BTreeMap::new(),
        });
        bucket.usage.add(&record.usage);
        bucket
            .by_model
            .entry(record.model.clone())
            .or_default()
            .add(&record.usage);
        bucket
            .by_source
            .entry(format!("{:?}", record.source).to_lowercase())
            .or_default()
            .add(&record.usage);
    }

    Summary {
        generated_at: Utc::now(),
        from: filter.from,
        to: filter.to,
        granularity,
        total,
        by_model,
        by_provider,
        by_source,
        buckets: buckets.into_values().collect(),
        records: count,
    }
}

pub fn bucket_for(
    ts: DateTime<Utc>,
    granularity: Granularity,
) -> (String, DateTime<Utc>, DateTime<Utc>) {
    match granularity {
        Granularity::Year => {
            let start = Utc.with_ymd_and_hms(ts.year(), 1, 1, 0, 0, 0).unwrap();
            (
                format!("{:04}", ts.year()),
                start,
                Utc.with_ymd_and_hms(ts.year() + 1, 1, 1, 0, 0, 0).unwrap(),
            )
        }
        Granularity::Month => {
            let start = Utc
                .with_ymd_and_hms(ts.year(), ts.month(), 1, 0, 0, 0)
                .unwrap();
            let end = if ts.month() == 12 {
                Utc.with_ymd_and_hms(ts.year() + 1, 1, 1, 0, 0, 0).unwrap()
            } else {
                Utc.with_ymd_and_hms(ts.year(), ts.month() + 1, 1, 0, 0, 0)
                    .unwrap()
            };
            (format!("{:04}-{:02}", ts.year(), ts.month()), start, end)
        }
        Granularity::Week => {
            let weekday = ts.weekday().num_days_from_monday() as i64;
            let date = ts.date_naive() - Duration::days(weekday);
            let start = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
            let iso = ts.iso_week();
            (
                format!("{:04}-W{:02}", iso.year(), iso.week()),
                start,
                start + Duration::days(7),
            )
        }
        Granularity::Day => {
            let start = Utc
                .with_ymd_and_hms(ts.year(), ts.month(), ts.day(), 0, 0, 0)
                .unwrap();
            (
                format!("{:04}-{:02}-{:02}", ts.year(), ts.month(), ts.day()),
                start,
                start + Duration::days(1),
            )
        }
    }
}
