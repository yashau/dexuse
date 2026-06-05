use crate::model::Granularity;
use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "dexuse",
    version,
    about = "Explore OpenAI Codex and Hermes usage"
)]
pub struct Args {
    #[arg(long, help = "Emit machine-readable JSON and skip the TUI")]
    pub json: bool,

    #[arg(
        long,
        value_enum,
        default_value = "day",
        help = "Aggregate buckets by year, month, week, or day"
    )]
    pub granularity: Granularity,

    #[arg(
        long,
        help = "Start date/time, e.g. 2026-06-01 or 2026-06-01T12:00:00Z"
    )]
    pub from: Option<String>,

    #[arg(long, help = "End date/time, exclusive, e.g. 2026-06-06")]
    pub to: Option<String>,

    #[arg(long, env = "CODEX_HOME", help = "Override Codex home directory")]
    pub codex_home: Option<PathBuf>,

    #[arg(long, env = "HERMES_HOME", help = "Override Hermes home directory")]
    pub hermes_home: Option<PathBuf>,

    #[arg(
        long,
        env = "OPENCLAW_STATE_DIR",
        help = "Override OpenClaw state directory"
    )]
    pub openclaw_home: Option<PathBuf>,

    #[arg(long, help = "Include only Codex CLI records")]
    pub codex_only: bool,

    #[arg(long, help = "Include only Hermes records")]
    pub hermes_only: bool,

    #[arg(long, help = "Include only OpenClaw records")]
    pub openclaw_only: bool,
}

pub fn parse_datetime(input: &str) -> Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Ok(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap()));
    }
    anyhow::bail!("could not parse date/time: {input}")
}

pub fn parse_filter(args: &Args) -> Result<crate::model::DateFilter> {
    Ok(crate::model::DateFilter {
        from: args
            .from
            .as_deref()
            .map(parse_datetime)
            .transpose()
            .context("invalid --from")?,
        to: args
            .to
            .as_deref()
            .map(parse_datetime)
            .transpose()
            .context("invalid --to")?,
    })
}
