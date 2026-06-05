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

pub fn collect_openclaw(home: &Path) -> Result<Vec<UsageRecord>> {
    if !home.exists() {
        return Ok(Vec::new());
    }

    let mut records = Vec::new();
    let agents = home.join("agents");
    let search_root = if agents.exists() {
        agents.as_path()
    } else {
        home
    };
    for entry in WalkDir::new(search_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if !is_usage_counted_transcript(path) {
            continue;
        }
        parse_openclaw_file(path, &mut records)
            .with_context(|| format!("reading {}", path.display()))?;
    }
    Ok(records)
}

fn parse_openclaw_file(path: &Path, out: &mut Vec<UsageRecord>) -> Result<()> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let session_id = session_id_from_path(path);
    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(record) = parse_openclaw_value(&value, &session_id) {
            out.push(record);
        }
    }
    Ok(())
}

fn parse_openclaw_value(value: &Value, session_id: &str) -> Option<UsageRecord> {
    let message = value.get("message").filter(|v| v.is_object());
    if let Some(role) = message.and_then(|m| m.get("role")).and_then(Value::as_str)
        && role != "assistant"
    {
        return None;
    }

    let usage_raw = message
        .and_then(|m| m.get("usage"))
        .or_else(|| value.get("usage"))?;
    let usage = normalize_usage(usage_raw)?;
    if usage.total_tokens == 0 && usage.estimated_cost_usd == 0.0 {
        return None;
    }

    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .or_else(|| {
            message
                .and_then(|m| m.get("timestamp"))
                .and_then(Value::as_str)
        })
        .and_then(parse_ts)?;

    let provider = message
        .and_then(|m| m.get("provider"))
        .and_then(Value::as_str)
        .or_else(|| value.get("provider").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();
    let model = message
        .and_then(|m| m.get("model"))
        .and_then(Value::as_str)
        .or_else(|| value.get("model").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();

    Some(UsageRecord {
        timestamp,
        source: Source::OpenClaw,
        provider,
        model,
        session_id: session_id.to_string(),
        title: None,
        usage,
    })
}

fn normalize_usage(value: &Value) -> Option<Usage> {
    let mut input = first_u64(
        value,
        &[
            "input",
            "inputTokens",
            "input_tokens",
            "promptTokens",
            "prompt_tokens",
            "prompt_n",
        ],
    );
    let output = first_u64(
        value,
        &[
            "output",
            "outputTokens",
            "output_tokens",
            "completionTokens",
            "completion_tokens",
            "predicted_n",
        ],
    );
    let cache_read = first_u64(
        value,
        &[
            "cacheRead",
            "cache_read",
            "cache_read_input_tokens",
            "cached_tokens",
        ],
    )
    .or_else(|| nested_u64(value, "input_tokens_details", "cached_tokens"))
    .or_else(|| nested_u64(value, "prompt_tokens_details", "cached_tokens"));
    let cache_write = first_u64(
        value,
        &["cacheWrite", "cache_write", "cache_creation_input_tokens"],
    );
    let reasoning = first_u64(value, &["reasoningTokens", "reasoning_tokens"])
        .or_else(|| nested_u64(value, "completion_tokens_details", "reasoning_tokens"))
        .or_else(|| nested_u64(value, "output_tokens_details", "reasoning_tokens"));

    let input_raw = input.unwrap_or(0);
    let cache_read_tokens = cache_read.unwrap_or(0);
    if has_openai_style_input_cache(value) {
        input = Some(input_raw.saturating_sub(cache_read_tokens));
    }

    let explicit_total = first_u64(value, &["total", "totalTokens", "total_tokens"]);
    let mut usage = Usage {
        input_tokens: input.unwrap_or(0),
        cached_input_tokens: cache_read_tokens,
        cache_write_tokens: cache_write.unwrap_or(0),
        output_tokens: output.unwrap_or(0),
        reasoning_tokens: reasoning.unwrap_or(0),
        api_calls: 1,
        estimated_cost_usd: cost_total(value).unwrap_or(0.0),
        ..Usage::default()
    };
    if let Some(total) = explicit_total {
        usage.total_tokens = total;
    } else {
        // Match OpenClaw's CostUsageTotals basis: input + output + cache read/write.
        usage.total_tokens = usage.input_tokens
            + usage.cached_input_tokens
            + usage.cache_write_tokens
            + usage.output_tokens;
    }
    if usage.total_tokens == 0 && usage.estimated_cost_usd == 0.0 {
        None
    } else {
        Some(usage)
    }
}

fn has_openai_style_input_cache(value: &Value) -> bool {
    (value.get("input_tokens").is_some() || value.get("prompt_tokens").is_some())
        && (nested_u64(value, "input_tokens_details", "cached_tokens").is_some()
            || nested_u64(value, "prompt_tokens_details", "cached_tokens").is_some())
}

fn cost_total(value: &Value) -> Option<f64> {
    value
        .get("cost")
        .and_then(|cost| cost.get("total"))
        .and_then(Value::as_f64)
        .or_else(|| value.get("costTotal").and_then(Value::as_f64))
        .or_else(|| value.get("totalCost").and_then(Value::as_f64))
}

fn first_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(number) = value.get(*key).and_then(value_to_u64) {
            return Some(number);
        }
    }
    None
}

fn nested_u64(value: &Value, object_key: &str, number_key: &str) -> Option<u64> {
    value
        .get(object_key)
        .and_then(|nested| nested.get(number_key))
        .and_then(value_to_u64)
}

fn value_to_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| value.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
}

fn parse_ts(input: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn is_usage_counted_transcript(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    if name == "sessions.json"
        || name.ends_with(".trajectory.jsonl")
        || name.contains(".checkpoint.")
        || name.contains(".jsonl.bak.")
    {
        return false;
    }
    name.ends_with(".jsonl") || name.contains(".jsonl.reset.") || name.contains(".jsonl.deleted.")
}

fn session_id_from_path(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("openclaw-session");
    if let Some(session) = name.strip_suffix(".jsonl") {
        return session.to_string();
    }
    for marker in [".jsonl.reset.", ".jsonl.deleted."] {
        if let Some(index) = name.find(marker) {
            return name[..index].to_string();
        }
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("openclaw-session")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    #[cfg_attr(miri, ignore = "filesystem FFI is not supported by Miri on Windows")]
    fn reads_usage_from_openclaw_agent_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("agents/main/sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("sess-main.jsonl"),
            format!(
                "{}\n{}\n",
                serde_json::json!({
                    "type":"message",
                    "id":"m1",
                    "timestamp":"2026-05-19T11:06:24.927Z",
                    "message":{
                        "role":"assistant",
                        "provider":"openai-codex",
                        "model":"gpt-5.5",
                        "usage":{
                            "input":43643,
                            "output":123,
                            "cacheRead":4480,
                            "cacheWrite":7,
                            "totalTokens":48253,
                            "cost":{"total":0.031}
                        }
                    }
                }),
                serde_json::json!({
                    "type":"message",
                    "timestamp":"2026-05-19T11:07:24.927Z",
                    "message":{
                        "role":"user",
                        "usage":{"input":999,"totalTokens":999}
                    }
                })
            ),
        )
        .unwrap();

        let records = collect_openclaw(dir.path()).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.provider, "openai-codex");
        assert_eq!(record.model, "gpt-5.5");
        assert_eq!(record.session_id, "sess-main");
        assert_eq!(record.usage.input_tokens, 43643);
        assert_eq!(record.usage.cached_input_tokens, 4480);
        assert_eq!(record.usage.cache_write_tokens, 7);
        assert_eq!(record.usage.output_tokens, 123);
        assert_eq!(record.usage.total_tokens, 48253);
        assert_eq!(record.usage.api_calls, 1);
        assert!((record.usage.estimated_cost_usd - 0.031).abs() < f64::EPSILON);
    }

    #[test]
    #[cfg_attr(miri, ignore = "filesystem FFI is not supported by Miri on Windows")]
    fn includes_reset_and_deleted_archives_but_skips_trajectory_checkpoints_and_bak() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("agents/worker/sessions");
        fs::create_dir_all(&sessions).unwrap();
        let assistant = serde_json::json!({
            "type":"message",
            "timestamp":"2026-05-19T11:06:24.927Z",
            "message":{
                "role":"assistant",
                "provider":"openai",
                "model":"gpt-5.4-mini",
                "usage":{"input":10,"output":3,"totalTokens":13}
            }
        })
        .to_string();
        for name in [
            "live.jsonl",
            "reset.jsonl.reset.2026-05-19T12-00-00.000Z",
            "deleted.jsonl.deleted.2026-05-19T12-00-00.000Z",
            "ignored.trajectory.jsonl",
            "ignored.checkpoint.019e575e-3d28-77c1-95f2-80f2cbf7eebd.jsonl",
            "ignored.jsonl.bak.2026-05-19T12-00-00.000Z",
        ] {
            fs::write(sessions.join(name), format!("{assistant}\n")).unwrap();
        }

        let records = collect_openclaw(dir.path()).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records.iter().map(|r| r.usage.total_tokens).sum::<u64>(),
            39
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "filesystem FFI is not supported by Miri on Windows")]
    fn normalizes_openai_style_cached_prompt_tokens_without_double_counting() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("agents/main/sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join("openai-style.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({
                    "type":"message",
                    "timestamp":"2026-05-19T11:06:24.927Z",
                    "message":{
                        "role":"assistant",
                        "provider":"openai",
                        "model":"gpt-5.4",
                        "usage":{
                            "prompt_tokens":100,
                            "completion_tokens":10,
                            "prompt_tokens_details":{"cached_tokens":80},
                            "completion_tokens_details":{"reasoning_tokens":4},
                            "total_tokens":110
                        }
                    }
                })
            ),
        )
        .unwrap();

        let records = collect_openclaw(dir.path()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].usage.input_tokens, 20);
        assert_eq!(records[0].usage.cached_input_tokens, 80);
        assert_eq!(records[0].usage.output_tokens, 10);
        assert_eq!(records[0].usage.reasoning_tokens, 4);
        assert_eq!(records[0].usage.total_tokens, 110);
    }
}
