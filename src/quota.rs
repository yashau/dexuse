use chrono::{Local, TimeZone};
use serde::Serialize;
use serde_json::Value;
use std::{
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodexQuota {
    pub five_hour_remaining_percent: u8,
    pub five_hour_resets_at: i64,
    pub seven_day_remaining_percent: u8,
    pub seven_day_resets_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodexQuotaSnapshot {
    #[serde(flatten)]
    pub quota: CodexQuota,
    pub label: String,
}

impl From<CodexQuota> for CodexQuotaSnapshot {
    fn from(quota: CodexQuota) -> Self {
        let label = format_quota_label(&quota);
        Self { quota, label }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexAuthStatus {
    Authenticated,
    RequiresChatGptAuth,
}

pub fn fetch_codex_quota() -> Option<CodexQuota> {
    if std::env::var_os("DEXUSE_DISABLE_CODEX_QUOTA").is_some() {
        return None;
    }
    let output = query_codex_app_server();
    let messages = output
        .as_deref()
        .unwrap_or("")
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    if let Some(quota) = choose_quota_with_backup(&messages, None) {
        return Some(quota);
    }
    let hermes_backup =
        query_hermes_account_usage().and_then(|text| serde_json::from_str::<Value>(&text).ok());
    if let Some(quota) = choose_quota_with_backup(&messages, hermes_backup.as_ref()) {
        return Some(quota);
    }
    query_openclaw_quota()
}

pub fn parse_quota_messages(messages: &[Value]) -> Option<CodexQuota> {
    choose_quota_with_backup(messages, None)
}

pub fn choose_quota_with_backup(
    app_server_messages: &[Value],
    hermes_usage_payload: Option<&Value>,
) -> Option<CodexQuota> {
    let quota = app_server_messages.iter().find_map(parse_quota_response);
    if quota.is_some() {
        return quota;
    }
    hermes_usage_payload.and_then(parse_hermes_usage_payload)
}

pub fn parse_wham_usage_payload(payload: &Value) -> Option<CodexQuota> {
    let rate_limit = payload.get("rate_limit")?;
    let primary = rate_limit.get("primary_window")?;
    let secondary = rate_limit.get("secondary_window")?;
    if window_seconds(primary) != Some(18_000) || window_seconds(secondary) != Some(604_800) {
        return None;
    }
    Some(CodexQuota {
        five_hour_remaining_percent: remaining_percent_snake(primary)?,
        five_hour_resets_at: reset_at_snake(primary)?,
        seven_day_remaining_percent: remaining_percent_snake(secondary)?,
        seven_day_resets_at: reset_at_snake(secondary)?,
    })
}

pub fn parse_hermes_usage_payload(payload: &Value) -> Option<CodexQuota> {
    parse_wham_usage_payload(payload)
}

fn query_codex_app_server() -> Option<String> {
    query_codex_app_server_with_home(None)
}

fn query_codex_app_server_with_home(codex_home: Option<&std::path::Path>) -> Option<String> {
    if std::env::var_os("DEXUSE_DISABLE_CODEX_APP_SERVER_QUOTA").is_some() {
        return None;
    }
    let mut command = Command::new(resolve_codex_binary());
    command
        .args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(codex_home) = codex_home {
        command.env("CODEX_HOME", codex_home);
    }
    let mut child = command.spawn().ok()?;

    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    {
        let stdin = child.stdin.as_mut()?;
        let initialize = serde_json::json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "dexgram",
                    "title": null,
                    "version": "0.0.1"
                },
                "capabilities": {
                    "experimentalApi": true
                }
            },
            "trace": null
        });
        let initialized = serde_json::json!({
            "method": "initialized"
        });
        let read_account = serde_json::json!({
            "id": 2,
            "method": "account/read",
            "params": {},
            "trace": null
        });
        let read_limits = serde_json::json!({
            "id": 3,
            "method": "account/rateLimits/read",
            "params": null,
            "trace": null
        });
        writeln!(stdin, "{initialize}").ok()?;
        writeln!(stdin, "{initialized}").ok()?;
        writeln!(stdin, "{read_account}").ok()?;
        writeln!(stdin, "{read_limits}").ok()?;
    }
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut output = String::new();
    loop {
        while let Ok(line) = rx.try_recv() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&line);
            if serde_json::from_str::<Value>(&line)
                .ok()
                .is_some_and(|value| {
                    parse_quota_response(&value).is_some()
                        || parse_codex_auth_status(&value)
                            == Some(CodexAuthStatus::RequiresChatGptAuth)
                        || value.get("id").and_then(Value::as_i64) == Some(3)
                })
            {
                let _ = child.kill();
                let _ = child.wait();
                return Some(output);
            }
        }

        match child.try_wait() {
            Ok(Some(_)) => return Some(output),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Some(output);
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(_) => return None,
        }
    }
}

fn run_child_with_timeout(mut child: std::process::Child, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().ok()?;
                return String::from_utf8(output.stdout).ok();
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let output = child.wait_with_output().ok()?;
                return String::from_utf8(output.stdout).ok();
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(_) => return None,
        }
    }
}

fn query_hermes_account_usage() -> Option<String> {
    if std::env::var_os("DEXUSE_DISABLE_HERMES_QUOTA_BACKUP").is_some() {
        return None;
    }
    let hermes_agent_dir = hermes_agent_dir()?;
    let python = hermes_python(&hermes_agent_dir);
    let script = r#"
import json
import sys
from agent.account_usage import fetch_account_usage
snapshot = fetch_account_usage('openai-codex')
if not snapshot:
    raise SystemExit(0)
rate_limit = {}
for window in getattr(snapshot, 'windows', ()):
    label = str(getattr(window, 'label', '') or '').lower()
    if 'session' in label:
        key, seconds = 'primary_window', 18000
    elif 'week' in label or 'weekly' in label:
        key, seconds = 'secondary_window', 604800
    else:
        continue
    reset_at = getattr(window, 'reset_at', None)
    rate_limit[key] = {
        'used_percent': getattr(window, 'used_percent', None),
        'reset_at': int(reset_at.timestamp()) if reset_at else None,
        'limit_window_seconds': seconds,
    }
print(json.dumps({'rate_limit': rate_limit}, separators=(',', ':')))
"#;
    let child = Command::new(python)
        .arg("-c")
        .arg(script)
        .current_dir(hermes_agent_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    run_child_with_timeout(child, Duration::from_secs(8)).and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn query_openclaw_quota() -> Option<CodexQuota> {
    if std::env::var_os("DEXUSE_DISABLE_OPENCLAW_QUOTA_BACKUP").is_some() {
        return None;
    }
    if std::env::var_os("DEXUSE_DISABLE_CODEX_APP_SERVER_QUOTA").is_none() {
        for codex_home in openclaw_codex_homes() {
            let output = query_codex_app_server_with_home(Some(&codex_home));
            let messages = output
                .as_deref()
                .unwrap_or("")
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .collect::<Vec<_>>();
            if let Some(quota) = parse_quota_messages(&messages) {
                return Some(quota);
            }
        }
    }
    let wham =
        query_openclaw_wham_usage().and_then(|text| serde_json::from_str::<Value>(&text).ok());
    wham.as_ref().and_then(parse_wham_usage_payload)
}

fn query_openclaw_wham_usage() -> Option<String> {
    for path in openclaw_auth_profile_paths() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(store) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some((access, account_id)) = select_openclaw_oauth_profile(&store) else {
            continue;
        };
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(4))
            .build();
        let mut request = agent
            .get("https://chatgpt.com/backend-api/wham/usage")
            .set("Authorization", &format!("Bearer {access}"))
            .set("Accept", "application/json")
            .set("originator", "openclaw")
            .set("User-Agent", concat!("dexuse/", env!("CARGO_PKG_VERSION")));
        if let Some(account_id) = account_id.as_deref() {
            request = request.set("ChatGPT-Account-Id", account_id);
        }
        let Ok(response) = request.call() else {
            continue;
        };
        if let Ok(body) = response.into_string()
            && !body.trim().is_empty()
        {
            return Some(body);
        }
    }
    None
}

fn select_openclaw_oauth_profile(store: &Value) -> Option<(String, Option<String>)> {
    let profiles = store.get("profiles")?.as_object()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    profiles.values().find_map(|profile| {
        let provider = profile.get("provider")?.as_str()?.to_ascii_lowercase();
        if provider != "openai-codex" && provider != "openai" {
            return None;
        }
        if profile.get("type").and_then(Value::as_str) != Some("oauth") {
            return None;
        }
        let access = profile.get("access")?.as_str()?.trim();
        if access.is_empty() {
            return None;
        }
        if let Some(expires) = profile.get("expires").and_then(Value::as_i64)
            && expires <= now_ms + 60_000
        {
            return None;
        }
        let account_id = profile
            .get("accountId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Some((access.to_string(), account_id))
    })
}

fn openclaw_auth_profile_paths() -> Vec<PathBuf> {
    openclaw_agent_dirs()
        .into_iter()
        .map(|agent_dir| agent_dir.join("auth-profiles.json"))
        .collect()
}

fn openclaw_codex_homes() -> Vec<PathBuf> {
    openclaw_agent_dirs()
        .into_iter()
        .map(|agent_dir| agent_dir.join("codex-home"))
        .filter(|codex_home| codex_home.exists())
        .collect()
}

fn openclaw_agent_dirs() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(state_dir) = std::env::var_os("OPENCLAW_STATE_DIR") {
        push_unique_path(
            &mut paths,
            PathBuf::from(state_dir)
                .join("agents")
                .join("main")
                .join("agent"),
        );
        return paths;
    }
    if let Some(home) = dirs::home_dir() {
        for dirname in [".openclaw", ".clawdbot"] {
            push_unique_path(
                &mut paths,
                home.join(dirname).join("agents").join("main").join("agent"),
            );
        }
    }
    paths
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn hermes_agent_dir() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let dir = PathBuf::from(local_app_data)
        .join("hermes")
        .join("hermes-agent");
    dir.exists().then_some(dir)
}

fn hermes_python(hermes_agent_dir: &std::path::Path) -> OsString {
    let windows_python = hermes_agent_dir
        .join("venv")
        .join("Scripts")
        .join("python.exe");
    if windows_python.exists() {
        return windows_python.into_os_string();
    }
    let posix_python = hermes_agent_dir.join("venv").join("bin").join("python");
    if posix_python.exists() {
        return posix_python.into_os_string();
    }
    OsString::from("python")
}

fn resolve_codex_binary() -> OsString {
    if let Some(path) = latest_local_codex_install() {
        return path.into_os_string();
    }
    OsString::from("codex")
}

fn latest_local_codex_install() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let bin_dir = PathBuf::from(local_app_data)
        .join("OpenAI")
        .join("Codex")
        .join("bin");
    let mut candidates = vec![bin_dir.join("codex.exe")];
    if let Ok(entries) = fs::read_dir(&bin_dir) {
        candidates.extend(entries.flatten().filter_map(|entry| {
            let path = entry.path();
            path.is_dir().then(|| path.join("codex.exe"))
        }));
    }
    candidates
        .into_iter()
        .filter_map(|path| {
            let modified = fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

pub fn parse_codex_auth_status(response: &Value) -> Option<CodexAuthStatus> {
    if response
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .is_some_and(|message| {
            message
                .to_lowercase()
                .contains("chatgpt authentication required")
        })
    {
        return Some(CodexAuthStatus::RequiresChatGptAuth);
    }

    let result = response.get("result")?;
    let account_type = result
        .get("account")
        .and_then(|account| account.get("type"))
        .and_then(Value::as_str);
    if account_type == Some("apiKey")
        && result
            .get("requiresOpenaiAuth")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Some(CodexAuthStatus::RequiresChatGptAuth);
    }
    if result.get("account").is_some() || result.get("authMethod").is_some() {
        return Some(CodexAuthStatus::Authenticated);
    }
    None
}

pub fn parse_quota_response(response: &Value) -> Option<CodexQuota> {
    if response.get("error").is_some() {
        return None;
    }
    let payload = response.get("result").or_else(|| {
        (response.get("method").and_then(Value::as_str) == Some("account/rateLimits/updated"))
            .then(|| response.get("params"))
            .flatten()
    })?;
    parse_quota_payload(payload)
}

fn parse_quota_payload(payload: &Value) -> Option<CodexQuota> {
    let snapshot = payload
        .get("rateLimitsByLimitId")
        .and_then(|limits| limits.get("codex"))
        .or_else(|| payload.get("rateLimits"))?;
    let primary = snapshot.get("primary")?;
    let secondary = snapshot.get("secondary")?;
    if primary.get("windowDurationMins").and_then(Value::as_i64) != Some(300)
        || secondary.get("windowDurationMins").and_then(Value::as_i64) != Some(10_080)
    {
        return None;
    }

    Some(CodexQuota {
        five_hour_remaining_percent: remaining_percent(primary)?,
        five_hour_resets_at: resets_at(primary)?,
        seven_day_remaining_percent: remaining_percent(secondary)?,
        seven_day_resets_at: resets_at(secondary)?,
    })
}

pub fn format_quota_label(quota: &CodexQuota) -> String {
    format!(
        "5h: {}% ↻ {} • 7d: {}% ↻ {}",
        quota.five_hour_remaining_percent,
        format_reset_time(quota.five_hour_resets_at, false),
        quota.seven_day_remaining_percent,
        format_reset_time(quota.seven_day_resets_at, true)
    )
}
fn remaining_percent(window: &Value) -> Option<u8> {
    let used = window.get("usedPercent")?.as_f64()?;
    Some((100.0 - used.clamp(0.0, 100.0)).round() as u8)
}

fn resets_at(window: &Value) -> Option<i64> {
    let reset = window.get("resetsAt")?.as_f64()?;
    Some(if reset > 1_000_000_000_000.0 {
        (reset / 1000.0) as i64
    } else {
        reset as i64
    })
}

fn remaining_percent_snake(window: &Value) -> Option<u8> {
    let used = window.get("used_percent")?.as_f64()?;
    Some((100.0 - used.clamp(0.0, 100.0)).round() as u8)
}

fn reset_at_snake(window: &Value) -> Option<i64> {
    let reset = window.get("reset_at")?;
    if let Some(number) = reset.as_f64() {
        return Some(if number > 1_000_000_000_000.0 {
            (number / 1000.0) as i64
        } else {
            number as i64
        });
    }
    let text = reset.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(number) = text.parse::<f64>() {
        return Some(if number > 1_000_000_000_000.0 {
            (number / 1000.0) as i64
        } else {
            number as i64
        });
    }
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.timestamp())
}

fn window_seconds(window: &Value) -> Option<i64> {
    window
        .get("limit_window_seconds")
        .and_then(Value::as_i64)
        .or_else(|| window.get("window_seconds").and_then(Value::as_i64))
}

fn format_reset_time(unix_seconds: i64, include_day: bool) -> String {
    let Some(dt) = Local.timestamp_opt(unix_seconds, 0).single() else {
        return "?".to_string();
    };
    let hour = dt.format("%I").to_string();
    let hour = hour.trim_start_matches('0');
    let am_pm = dt.format("%P");
    let time = format!("{}:{}{am_pm}", hour, dt.format("%M"));
    if include_day {
        format!("{} {time}", dt.format("%a"))
    } else {
        time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_primary_and_secondary_remaining_quota_from_rate_limit_response() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "rateLimits": {
                    "limitId": "codex",
                    "primary": { "usedPercent": 26, "windowDurationMins": 300, "resetsAt": 1780513413 },
                    "secondary": { "usedPercent": 81, "windowDurationMins": 10080, "resetsAt": 1781118213 },
                    "credits": null,
                    "individualLimit": null,
                    "planType": "plus",
                    "rateLimitReachedType": null
                },
                "rateLimitsByLimitId": {}
            }
        });

        assert_eq!(
            parse_quota_response(&response),
            Some(CodexQuota {
                five_hour_remaining_percent: 74,
                five_hour_resets_at: 1780513413,
                seven_day_remaining_percent: 19,
                seven_day_resets_at: 1781118213,
            })
        );
    }

    #[test]
    fn parses_unsolicited_rate_limit_update_event_like_dexgram_logs() {
        let response = serde_json::json!({
            "method": "account/rateLimits/updated",
            "params": {
                "rateLimitsByLimitId": {
                    "codex": {
                        "limitId": "codex",
                        "primary": { "usedPercent": 26.4, "windowDurationMins": 300, "resetsAt": 1780513413000.0 },
                        "secondary": { "usedPercent": 81.0, "windowDurationMins": 10080, "resetsAt": 1781118213000.0 }
                    }
                }
            }
        });

        assert_eq!(
            parse_quota_response(&response),
            Some(CodexQuota {
                five_hour_remaining_percent: 74,
                five_hour_resets_at: 1780513413,
                seven_day_remaining_percent: 19,
                seven_day_resets_at: 1781118213,
            })
        );
    }

    #[test]
    fn detects_cli_runtime_that_still_needs_chatgpt_auth() {
        let response = serde_json::json!({
            "id": 2,
            "result": {
                "account": { "type": "apiKey" },
                "requiresOpenaiAuth": true
            }
        });

        assert_eq!(
            parse_codex_auth_status(&response),
            Some(CodexAuthStatus::RequiresChatGptAuth)
        );
    }

    #[test]
    fn treats_auth_required_rate_limit_errors_as_requires_chatgpt_auth() {
        let response = serde_json::json!({
            "id": 3,
            "error": {
                "code": -32600,
                "message": "chatgpt authentication required to read rate limits"
            }
        });

        assert_eq!(
            parse_codex_auth_status(&response),
            Some(CodexAuthStatus::RequiresChatGptAuth)
        );
    }

    #[test]
    fn prefers_rate_limit_payload_when_chatgpt_account_read_still_sets_requires_openai_auth() {
        let account = serde_json::json!({
            "id": 2,
            "result": {
                "account": { "type": "chatgpt", "planType": "prolite" },
                "requiresOpenaiAuth": true
            }
        });
        let quota = serde_json::json!({
            "id": 3,
            "result": {
                "rateLimits": {
                    "limitId": "codex",
                    "primary": { "usedPercent": 20, "windowDurationMins": 300, "resetsAt": 1780653518 },
                    "secondary": { "usedPercent": 21, "windowDurationMins": 10080, "resetsAt": 1781138432 }
                }
            }
        });

        assert_eq!(
            parse_quota_messages(&[account, quota]).map(|q| q.five_hour_remaining_percent),
            Some(80)
        );
    }

    #[test]
    fn parses_hermes_wham_usage_payload_as_backup_quota_source() {
        let payload = serde_json::json!({
            "plan_type": "prolite",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 22,
                    "reset_at": 1780653518,
                    "limit_window_seconds": 18000
                },
                "secondary_window": {
                    "used_percent": 21,
                    "reset_at": 1781138432,
                    "limit_window_seconds": 604800
                }
            }
        });

        assert_eq!(
            parse_hermes_usage_payload(&payload),
            Some(CodexQuota {
                five_hour_remaining_percent: 78,
                five_hour_resets_at: 1780653518,
                seven_day_remaining_percent: 79,
                seven_day_resets_at: 1781138432,
            })
        );
    }

    #[test]
    fn parses_openclaw_wham_usage_payload_as_backup_quota_source() {
        let payload = serde_json::json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 12.4,
                    "reset_at": "1780653518",
                    "limit_window_seconds": 18000
                },
                "secondary_window": {
                    "used_percent": 55,
                    "reset_at": 1781138432000.0,
                    "limit_window_seconds": 604800
                }
            }
        });

        assert_eq!(
            parse_wham_usage_payload(&payload),
            Some(CodexQuota {
                five_hour_remaining_percent: 88,
                five_hour_resets_at: 1780653518,
                seven_day_remaining_percent: 45,
                seven_day_resets_at: 1781138432,
            })
        );
    }

    #[test]
    fn selects_openclaw_oauth_profile_for_wham_probe() {
        let store = serde_json::json!({
            "version": 1,
            "profiles": {
                "api": { "type": "api_key", "provider": "openai", "key": "ignored" },
                "codex": {
                    "type": "oauth",
                    "provider": "openai-codex",
                    "access": " access-token ",
                    "accountId": " account-id ",
                    "expires": 4102444800000_i64
                }
            }
        });

        assert_eq!(
            select_openclaw_oauth_profile(&store),
            Some(("access-token".to_string(), Some("account-id".to_string())))
        );
    }

    #[test]
    fn skips_expired_openclaw_oauth_profiles() {
        let store = serde_json::json!({
            "profiles": {
                "expired": {
                    "type": "oauth",
                    "provider": "openai-codex",
                    "access": "stale-token",
                    "expires": 1
                }
            }
        });

        assert_eq!(select_openclaw_oauth_profile(&store), None);
    }

    #[test]
    fn falls_back_to_hermes_usage_payload_when_app_server_has_no_quota() {
        let app_server_messages = vec![serde_json::json!({
            "id": 3,
            "error": {
                "code": -32600,
                "message": "chatgpt authentication required to read rate limits"
            }
        })];
        let hermes_payload = serde_json::json!({
            "rate_limit": {
                "primary_window": { "used_percent": 10, "reset_at": 1780653518, "limit_window_seconds": 18000 },
                "secondary_window": { "used_percent": 30, "reset_at": 1781138432, "limit_window_seconds": 604800 }
            }
        });

        assert_eq!(
            choose_quota_with_backup(&app_server_messages, Some(&hermes_payload)).map(|quota| (
                quota.five_hour_remaining_percent,
                quota.seven_day_remaining_percent
            )),
            Some((90, 70))
        );
    }

    #[test]
    fn skips_api_mode_errors_and_incomplete_responses() {
        let api_mode = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "error": {
                "code": -32600,
                "message": "chatgpt authentication required to read rate limits"
            }
        });
        assert_eq!(parse_quota_response(&api_mode), None);

        let missing_secondary = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "rateLimits": {
                    "primary": { "usedPercent": 26, "windowDurationMins": 300 }
                }
            }
        });
        assert_eq!(parse_quota_response(&missing_secondary), None);
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "local timezone FFI is not supported by Miri on Windows"
    )]
    fn formats_quota_for_compact_top_tab_row() {
        let quota = CodexQuota {
            five_hour_remaining_percent: 74,
            five_hour_resets_at: 1780513413,
            seven_day_remaining_percent: 19,
            seven_day_resets_at: 1781118213,
        };
        let label = format_quota_label(&quota);
        assert!(label.starts_with("5h: 74% ↻ "));
        assert!(label.contains(" • 7d: 19% ↻ "));
        assert!(!label.contains(" am") && !label.contains(" pm"));
        assert!(label.contains("am") || label.contains("pm"));
    }
}
