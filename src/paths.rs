use std::path::PathBuf;

pub fn default_codex_homes() -> Vec<PathBuf> {
    if let Some(path) = non_empty_env_path("CODEX_HOME") {
        return vec![path];
    }
    dirs::home_dir()
        .map(|home| vec![home.join(".codex")])
        .unwrap_or_default()
}

pub fn default_hermes_homes() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if let Some(path) = non_empty_env_path("HERMES_HOME") {
        push_unique(&mut homes, path);
        return homes;
    }

    // Hermes Desktop and the Windows installer use %LOCALAPPDATA%\hermes.
    // Hermes CLI, macOS/Linux Desktop, and legacy Windows installs use ~/.hermes.
    if cfg!(windows)
        && let Some(local_app_data) = non_empty_env_path("LOCALAPPDATA")
    {
        push_unique(&mut homes, local_app_data.join("hermes"));
    }
    if let Some(home) = dirs::home_dir() {
        push_unique(&mut homes, home.join(".hermes"));
    }
    homes
}

pub fn default_openclaw_homes() -> Vec<PathBuf> {
    if let Some(path) = non_empty_env_path("OPENCLAW_STATE_DIR") {
        return vec![path];
    }
    dirs::home_dir()
        .map(|home| vec![home.join(".openclaw"), home.join(".clawdbot")])
        .unwrap_or_default()
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_unique_deduplicates_exact_paths() {
        let mut paths = Vec::new();
        push_unique(&mut paths, PathBuf::from("/tmp/a"));
        push_unique(&mut paths, PathBuf::from("/tmp/a"));
        push_unique(&mut paths, PathBuf::from("/tmp/b"));
        assert_eq!(paths.len(), 2);
    }
}
