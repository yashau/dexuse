use crate::{
    codex::collect_codex,
    hermes::collect_hermes,
    model::UsageRecord,
    openclaw::collect_openclaw,
    paths::{default_codex_homes, default_hermes_homes, default_openclaw_homes},
};
use anyhow::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub type DefaultHomesFn = fn() -> Vec<PathBuf>;
pub type CollectFn = fn(&Path) -> Result<Vec<UsageRecord>>;

#[derive(Clone, Copy)]
pub struct UsageHarness {
    pub id: &'static str,
    pub display_name: &'static str,
    default_homes: DefaultHomesFn,
    collect: CollectFn,
}

impl UsageHarness {
    pub const fn new(
        id: &'static str,
        display_name: &'static str,
        default_homes: DefaultHomesFn,
        collect: CollectFn,
    ) -> Self {
        Self {
            id,
            display_name,
            default_homes,
            collect,
        }
    }

    pub fn homes(&self, override_home: Option<&PathBuf>) -> Vec<PathBuf> {
        override_home
            .cloned()
            .map(|home| vec![home])
            .unwrap_or_else(|| (self.default_homes)())
    }

    pub fn collect_from_home(&self, home: &Path) -> Result<Vec<UsageRecord>> {
        (self.collect)(home)
    }
}

pub fn builtin_harnesses() -> Vec<UsageHarness> {
    vec![
        UsageHarness::new("codex", "Codex CLI", default_codex_homes, collect_codex),
        UsageHarness::new(
            "hermes",
            "Hermes Agent",
            default_hermes_homes,
            collect_hermes,
        ),
        UsageHarness::new(
            "openclaw",
            "OpenClaw",
            default_openclaw_homes,
            collect_openclaw,
        ),
    ]
}

pub fn collect_harness_records(
    harnesses: &[UsageHarness],
    home_overrides: &BTreeMap<&str, PathBuf>,
    selected_ids: &BTreeSet<&str>,
) -> Result<Vec<UsageRecord>> {
    let mut records = Vec::new();
    for harness in harnesses
        .iter()
        .filter(|harness| selected_ids.is_empty() || selected_ids.contains(harness.id))
    {
        for home in harness.homes(home_overrides.get(harness.id)) {
            records.extend(harness.collect_from_home(&home)?);
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Source, UsageRecord};
    use anyhow::Result;
    use chrono::{TimeZone, Utc};
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::{Path, PathBuf},
    };

    fn fake_collect(home: &Path) -> Result<Vec<UsageRecord>> {
        Ok(vec![UsageRecord {
            timestamp: Utc.with_ymd_and_hms(2026, 6, 5, 0, 0, 0).unwrap(),
            source: Source::Codex,
            provider: home.display().to_string(),
            model: "fake-model".to_string(),
            session_id: "fake-session".to_string(),
            title: None,
            usage: Default::default(),
        }])
    }

    #[test]
    fn selected_harnesses_are_collected_through_one_registry_path() {
        let harnesses = vec![
            UsageHarness::new(
                "alpha",
                "Alpha",
                || vec![PathBuf::from("/default/alpha")],
                fake_collect,
            ),
            UsageHarness::new(
                "beta",
                "Beta",
                || vec![PathBuf::from("/default/beta")],
                fake_collect,
            ),
        ];
        let mut selected = BTreeSet::new();
        selected.insert("beta");
        let overrides = BTreeMap::new();

        let records = collect_harness_records(&harnesses, &overrides, &selected).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider, "/default/beta");
    }

    #[test]
    fn explicit_home_override_replaces_a_harness_default_homes() {
        let harnesses = vec![UsageHarness::new(
            "alpha",
            "Alpha",
            || vec![PathBuf::from("/default/alpha")],
            fake_collect,
        )];
        let selected = BTreeSet::new();
        let mut overrides = BTreeMap::new();
        overrides.insert("alpha", PathBuf::from("/override/alpha"));

        let records = collect_harness_records(&harnesses, &overrides, &selected).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].provider, "/override/alpha");
    }

    #[test]
    fn builtin_harnesses_expose_existing_sources_in_one_extension_point() {
        let ids = builtin_harnesses()
            .into_iter()
            .map(|h| h.id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["codex", "hermes", "openclaw"]);
    }
}
