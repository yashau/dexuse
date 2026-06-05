<div align="center">

# dexuse

<img src="screenshots/dexuse-tui-timeline.png" alt="dexuse timeline view" width="920" />

**Your local AI token burn, finally visible.**

`dexuse` turns messy local usage history from Codex, Hermes, and OpenClaw into a polished terminal dashboard with timelines, model splits, source breakdowns, cache reads, reasoning tokens, and JSON export.

[![Rust](https://img.shields.io/badge/Rust-fast-orange?style=for-the-badge&logo=rust)](https://www.rust-lang.org/) [![TUI](https://img.shields.io/badge/TUI-fancy_af-8b5cf6?style=for-the-badge)](#screenshots) [![npx](https://img.shields.io/badge/run_with-npx-cb3837?style=for-the-badge&logo=npm)](#install)

</div>

## Why it exists

AI tools write usage logs all over your machine. `dexuse` pulls them together so you can answer the questions that actually matter:

- Which model ate the most tokens?
- How much was cached versus fresh input?
- Was it Codex, Hermes, or OpenClaw?
- What changed by day, week, month, or year?
- Can I get the same view as JSON for scripts? Yep.

No cloud account. No upload. It reads local files only.

## Install

```bash
npx dexuse
```

Or from a checkout:

```bash
pnpm install
cargo run --release --
```

## The good stuff

```bash
npx dexuse
npx dexuse --json
npx dexuse --json --from 2026-06-01 --to 2026-06-06 --granularity day
npx dexuse --codex-only
npx dexuse --hermes-only
npx dexuse --openclaw-only
```

`dexuse` automatically looks in the usual places:

- Codex: `~/.codex`, including archived sessions
- Hermes: `~/.hermes`, `%LOCALAPPDATA%\hermes`, and profile databases
- OpenClaw: `~/.openclaw`, with legacy `~/.clawdbot` fallback

Need a custom path?

```bash
npx dexuse --codex-home ./fixtures/usage/codex
npx dexuse --hermes-home ./fixtures/usage/hermes
npx dexuse --openclaw-home ./fixtures/usage/openclaw
```

## Screenshots

<div align="center">

### Drill into time

<img src="screenshots/dexuse-tui-drilldown.png" alt="dexuse drilldown view" width="920" />

### Compare models

<img src="screenshots/dexuse-tui-models.png" alt="dexuse models view" width="920" />

### See where the tokens came from

<img src="screenshots/dexuse-tui-sources.png" alt="dexuse sources view" width="920" />

</div>

## Keyboard moves

- `1` / `2` / `3`: Timeline, Models, Sources
- `Tab`, `Shift+Tab`, `[` / `]`: switch tabs
- `←` / `→` or `h` / `l`: move the selected period
- `Enter` / `Space`: drill down through time
- `u` / `Backspace`: go back up
- `y` / `m` / `w` / `d`: year, month, week, day
- `q` / `Esc`: quit

## What it counts

`dexuse` keeps the buckets separate so big cached sessions do not look like fresh input:

- input tokens
- cached input tokens
- cache write tokens
- output tokens
- reasoning tokens
- API calls
- estimated cost when the source provides it

JSON output includes totals, time buckets, model splits, provider splits, and source splits.

## Built for more agents

Codex, Hermes, and OpenClaw are just harnesses. The ingestion layer is modular, so another local agent can be added without turning `main.rs` into spaghetti. Maintainer details live in [`AGENTS.md`](AGENTS.md).

## Ship it locally

```bash
mise install
mise run check
mise run screenshots
mise run pack
```

Direct commands work too:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
python scripts/bundle_current.py
python scripts/smoke_json.py
pnpm run screenshots
pnpm pack --dry-run
```

## License

MIT.
