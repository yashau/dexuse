use crate::model::{Source, Usage, UsageRecord};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
use walkdir::WalkDir;

pub fn collect_codex(home: &Path) -> Result<Vec<UsageRecord>> {
    if !home.exists() {
        return Ok(vec![]);
    }
    let mut records = Vec::new();
    for entry in WalkDir::new(home).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        if path.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        parse_codex_file(path, &mut records)
            .with_context(|| format!("reading {}", path.display()))?;
    }
    Ok(records)
}

fn parse_codex_file(path: &Path, out: &mut Vec<UsageRecord>) -> Result<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let values = reader
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .collect::<Vec<_>>();
    let default_session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("codex-session");
    parse_codex_values(&values, default_session_id, out);
    Ok(())
}

fn parse_codex_values(values: &[Value], default_session_id: &str, out: &mut Vec<UsageRecord>) {
    let mut session_id = default_session_id.to_string();
    let mut fallback_model = "unknown".to_string();
    let mut current_model = "unknown".to_string();
    let mut title = None;

    for v in values {
        if let Some(id) = v.get("id").and_then(Value::as_str) {
            session_id = id.to_string();
        }
        if let Some(m) = find_string_key(v, "model") {
            fallback_model = m.to_string();
            break;
        }
        if let Some(name) = v.get("thread_name").and_then(Value::as_str) {
            title = Some(name.to_string());
        }
        if let Some(payload) = v.get("payload")
            && let Some(id) = payload.get("id").and_then(Value::as_str)
        {
            session_id = id.to_string();
        }
    }

    for v in values {
        if let Some(name) = v.get("thread_name").and_then(Value::as_str) {
            title = Some(name.to_string());
        }
        if let Some(payload) = v.get("payload")
            && let Some(id) = payload.get("id").and_then(Value::as_str)
        {
            session_id = id.to_string();
        }
        if let Some(m) = find_string_key(v, "model") {
            current_model = m.to_string();
        }

        let payload = v.get("payload").unwrap_or(&Value::Null);
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        let Some(ts) = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_ts)
        else {
            continue;
        };
        let info = payload.get("info").unwrap_or(&Value::Null);
        let last = info.get("last_token_usage").unwrap_or(&Value::Null);
        let mut usage = Usage {
            input_tokens: u(last, "input_tokens").saturating_sub(u(last, "cached_input_tokens")),
            cached_input_tokens: u(last, "cached_input_tokens"),
            output_tokens: u(last, "output_tokens"),
            reasoning_tokens: u(last, "reasoning_output_tokens"),
            api_calls: if u(last, "total_tokens") > 0 { 1 } else { 0 },
            ..Usage::default()
        };
        usage.recompute_total();
        if usage.total_tokens == 0 {
            continue;
        }
        out.push(UsageRecord {
            timestamp: ts,
            source: Source::Codex,
            provider: "openai-codex".to_string(),
            model: if current_model == "unknown" {
                fallback_model.clone()
            } else {
                current_model.clone()
            },
            session_id: session_id.clone(),
            title: title.clone(),
            usage,
        });
    }
}

fn parse_ts(input: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn find_string_key<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key).and_then(Value::as_str) {
                return Some(found);
            }
            map.values().find_map(|child| find_string_key(child, key))
        }
        Value::Array(items) => items.iter().find_map(|child| find_string_key(child, key)),
        _ => None,
    }
}

fn u(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_last_usage_as_per_call_delta() {
        let values = vec![
            serde_json::json!({"timestamp":"2026-06-01T00:00:00Z","type":"session_meta","payload":{"id":"abc","model":"gpt-5.5"}}),
            serde_json::json!({"timestamp":"2026-06-01T00:01:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":90,"output_tokens":5,"reasoning_output_tokens":2,"total_tokens":105}}}}),
        ];
        let mut records = Vec::new();
        parse_codex_values(&values, "s", &mut records);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].usage.input_tokens, 10);
        assert_eq!(records[0].usage.cached_input_tokens, 90);
        assert_eq!(records[0].usage.output_tokens, 5);
        assert_eq!(records[0].usage.reasoning_tokens, 2);
        assert_eq!(records[0].usage.total_tokens, 107);
    }

    #[test]
    fn attributes_token_counts_to_current_model_within_one_session() {
        let values = vec![
            serde_json::json!({"timestamp":"2026-06-01T00:00:00Z","type":"session_meta","payload":{"id":"abc"}}),
            serde_json::json!({"timestamp":"2026-06-01T00:00:10Z","type":"turn_context","payload":{"model":"gpt-5.3-codex-spark"}}),
            serde_json::json!({"timestamp":"2026-06-01T00:01:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0,"total_tokens":11}}}}),
            serde_json::json!({"timestamp":"2026-06-01T00:02:10Z","type":"turn_context","payload":{"model":"gpt-5.5"}}),
            serde_json::json!({"timestamp":"2026-06-01T00:03:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":20,"cached_input_tokens":5,"output_tokens":2,"reasoning_output_tokens":1,"total_tokens":22}}}}),
        ];
        let mut records = Vec::new();
        parse_codex_values(&values, "multi", &mut records);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].session_id, records[1].session_id);
        assert_eq!(records[0].model, "gpt-5.3-codex-spark");
        assert_eq!(records[1].model, "gpt-5.5");
        assert_eq!(records[1].usage.input_tokens, 15);
    }
}
