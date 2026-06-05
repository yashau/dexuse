use anyhow::Result;
use clap::Parser;
use dexuse::{
    aggregate::aggregate,
    cli::{Args, parse_filter},
    output::print_json,
    quota::fetch_codex_quota,
    sources::{builtin_harnesses, collect_harness_records},
};
use std::collections::{BTreeMap, BTreeSet};

fn main() -> Result<()> {
    let args = Args::parse();
    let filter = parse_filter(&args)?;
    let selected_ids = selected_source_ids(&args);
    let home_overrides = home_overrides(&args);
    let mut records =
        collect_harness_records(&builtin_harnesses(), &home_overrides, &selected_ids)?;

    records.sort_by_key(|r| r.timestamp);
    let summary = aggregate(&records, &filter, args.granularity);
    if args.json {
        print_json(&summary, fetch_codex_quota().map(Into::into))?;
    } else {
        dexuse::tui::run(records, filter, args.granularity)?;
    }
    Ok(())
}

fn selected_source_ids(args: &Args) -> BTreeSet<&'static str> {
    let mut selected = BTreeSet::new();
    if args.codex_only {
        selected.insert("codex");
    }
    if args.hermes_only {
        selected.insert("hermes");
    }
    if args.openclaw_only {
        selected.insert("openclaw");
    }
    selected
}

fn home_overrides(args: &Args) -> BTreeMap<&'static str, std::path::PathBuf> {
    let mut overrides = BTreeMap::new();
    if let Some(home) = args.codex_home.clone() {
        overrides.insert("codex", home);
    }
    if let Some(home) = args.hermes_home.clone() {
        overrides.insert("hermes", home);
    }
    if let Some(home) = args.openclaw_home.clone() {
        overrides.insert("openclaw", home);
    }
    overrides
}
