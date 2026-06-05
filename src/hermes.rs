use crate::model::{Source, Usage, UsageRecord};
use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, Row};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn collect_hermes(home: &Path) -> Result<Vec<UsageRecord>> {
    let mut records = Vec::new();
    for db in hermes_state_dbs(home) {
        records.extend(read_state_db(&db).with_context(|| format!("reading {}", db.display()))?);
    }
    Ok(records)
}

fn hermes_state_dbs(home: &Path) -> Vec<PathBuf> {
    if !home.exists() {
        return vec![];
    }
    let mut dbs = Vec::new();
    let root = home.join("state.db");
    if root.exists() {
        dbs.push(root);
    }
    let profiles = home.join("profiles");
    if profiles.exists() {
        for entry in WalkDir::new(profiles).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if path.file_name().and_then(|s| s.to_str()) == Some("state.db") {
                dbs.push(path.to_path_buf());
            }
        }
    }
    dbs
}

fn read_state_db(path: &Path) -> Result<Vec<UsageRecord>> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "select id,title,model,started_at,ended_at,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,reasoning_tokens,api_call_count,billing_provider,billing_base_url,billing_mode,estimated_cost_usd from sessions",
    )?;
    let rows = stmt.query_map([], row_to_record)?;
    let mut records = Vec::new();
    for row in rows {
        if let Some(record) = row? {
            records.push(record);
        }
    }
    Ok(records)
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<Option<UsageRecord>> {
    let provider: Option<String> = row.get("billing_provider")?;
    let provider = provider.unwrap_or_else(|| "unknown".to_string());
    if !is_openai_provider(&provider) {
        return Ok(None);
    }
    let started: Option<f64> = row.get("started_at")?;
    let ended: Option<f64> = row.get("ended_at")?;
    let Some(timestamp) = ts_from_seconds(ended.or(started).unwrap_or(0.0)) else {
        return Ok(None);
    };
    let mut usage = Usage {
        input_tokens: opt_i64(row, "input_tokens") as u64,
        cached_input_tokens: opt_i64(row, "cache_read_tokens") as u64,
        cache_write_tokens: opt_i64(row, "cache_write_tokens") as u64,
        output_tokens: opt_i64(row, "output_tokens") as u64,
        reasoning_tokens: opt_i64(row, "reasoning_tokens") as u64,
        api_calls: opt_i64(row, "api_call_count") as u64,
        estimated_cost_usd: row
            .get::<_, Option<f64>>("estimated_cost_usd")?
            .unwrap_or(0.0),
        ..Usage::default()
    };
    usage.recompute_total();
    if usage.total_tokens == 0 {
        return Ok(None);
    }
    Ok(Some(UsageRecord {
        timestamp,
        source: Source::Hermes,
        provider,
        model: row
            .get::<_, Option<String>>("model")?
            .unwrap_or_else(|| "unknown".to_string()),
        session_id: row.get("id")?,
        title: row.get("title")?,
        usage,
    }))
}

fn is_openai_provider(provider: &str) -> bool {
    matches!(provider, "openai-codex" | "openai") || provider.starts_with("openai:")
}

fn opt_i64(row: &Row<'_>, name: &str) -> i64 {
    row.get::<_, Option<i64>>(name)
        .ok()
        .flatten()
        .unwrap_or(0)
        .max(0)
}

fn ts_from_seconds(seconds: f64) -> Option<DateTime<Utc>> {
    if seconds <= 0.0 {
        return None;
    }
    let secs = seconds.trunc() as i64;
    let nanos = ((seconds.fract().abs()) * 1_000_000_000.0) as u32;
    Utc.timestamp_opt(secs, nanos).single()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "SQLite/file FFI is not supported by Miri on Windows")]
    fn reads_openai_codex_and_openai_api_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("create table sessions(id text,title text,model text,started_at real,ended_at real,input_tokens integer,output_tokens integer,cache_read_tokens integer,cache_write_tokens integer,reasoning_tokens integer,api_call_count integer,billing_provider text,billing_base_url text,billing_mode text,estimated_cost_usd real);").unwrap();
        conn.execute("insert into sessions values('h1','codex','gpt-5.5',1780513413.0,null,10,3,90,0,1,2,'openai-codex','https://chatgpt.com/backend-api/codex','subscription_included',0.0)", []).unwrap();
        conn.execute("insert into sessions values('h2','api','gpt-5.4-mini',1780513414.0,null,100,20,30,5,2,1,'openai','https://api.openai.com/v1','pay_as_you_go',0.42)", []).unwrap();
        conn.execute("insert into sessions values('h3','other','claude',1780513415.0,null,1000,20,0,0,0,1,'anthropic','',null,1.0)", []).unwrap();
        drop(conn);
        let records = collect_hermes(dir.path()).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|r| r.provider == "openai-codex"));
        assert!(
            records
                .iter()
                .any(|r| r.provider == "openai" && r.usage.estimated_cost_usd == 0.42)
        );
    }
}
