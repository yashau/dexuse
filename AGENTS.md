# AGENTS.md

This repo is a Rust/Ratatui CLI packaged for `npx dexuse`. Keep it fast, local-first, modular, and screenshot-polished.

## Product promise

`dexuse` reads local AI usage stores and turns them into a fancy terminal dashboard plus JSON summaries. It must never upload user history or print secrets. Treat local session files, transcript text, database rows, and paths as sensitive. Usage totals are fine; raw prompts/messages are not.

## Stack

- Rust CLI and TUI: `src/`
- TUI framework: Ratatui + Crossterm
- JSON output: `src/output.rs`
- Aggregation/model types: `src/model.rs`, `src/aggregate.rs`
- npm wrapper: `scripts/dexuse.js`
- screenshot harness: `scripts/render_tui_screenshots.cjs`
- deterministic usage fixtures: `fixtures/usage/`
- package manager: `pnpm` (`packageManager` is pinned in `package.json`)
- task runner: `mise`

Use `pnpm`, not npm, when managing JS dependencies.

## Quality gates

Before saying a change is done, run the relevant subset. For user-facing, parser, fixture, or packaging changes, run all of them:

```bash
cargo fmt --check
python scripts/smoke_json.py
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
python scripts/bundle_current.py
pnpm run screenshots
pnpm pack --dry-run
```

Mise equivalents:

```bash
mise run check
mise run screenshots
mise run pack
```

Notes:

- Rebuild release before trusting `pnpm exec dexuse` or screenshot output. The JS wrapper may prefer `bin/dexuse-<platform>-<arch>`.
- `scripts/with_timeout.py` is the preferred timeout wrapper in automation.
- Miri uses nightly. Hermes SQLite tests are ignored under Miri because SQLite/file FFI is not supported by Miri on Windows.

## Versioning

Human-facing releases use calendar-build versions:

```text
YYYY.M.D.n
```

Example:

```text
2026.6.5.1
```

Use this exact dotted form for CLI output, Git tags, and GitHub releases.

Cargo and npm require SemVer and cannot store four numeric components. Package metadata uses an npm-stable patch encoding:

```text
YYYY.M.DNN
```

where `NN` is the two-digit daily release number. Examples:

```text
2026.6.5.1  -> 2026.6.501
2026.6.5.12 -> 2026.6.512
```

Do not use `YYYY.M.D-n` for npm releases unless intentionally publishing a prerelease tag; npm treats that as prerelease and requires `--tag`.

When cutting a new release on the same day, increment only `n`. Update:

- `src/version.rs` display version
- `Cargo.toml` package version, encoded as `YYYY.M.DNN`
- `package.json` package version, encoded as `YYYY.M.DNN`
- `Cargo.lock` via `cargo update -p dexuse --precise <semver-version>`
- lockfiles via the package manager if dependencies or package metadata require it

## TUI expectations

The dashboard should look dense and polished, not like a plain debug view.

Required UX standards:

- chart above table
- real navigation, not decorative shortcuts
- visible selected row/period
- dark premium theme with strong accent colors
- screenshots cropped to the actual terminal bounds
- no fake browser chrome around screenshots
- screenshots generated from deterministic fixtures, not a developer's private local history

Current screenshot outputs:

```text
screenshots/dexuse-tui-timeline.png
screenshots/dexuse-tui-drilldown.png
screenshots/dexuse-tui-models.png
screenshots/dexuse-tui-sources.png
```

## Fixture rules

Fixtures live under:

```text
fixtures/usage/
```

They must cover every built-in harness:

```text
fixtures/usage/codex/
fixtures/usage/hermes/
fixtures/usage/openclaw/
```

The smoke script must prove all fixture sources are present with nonzero totals. Current date window:

```text
2026-06-01 through 2026-06-06
```

Model names should stay current and realistic:

```text
gpt-5.5
gpt-5.4
gpt-5.4-mini
gpt-5.3-codex-spark
```

`gpt-5.5` fixture usage should remain large enough to exercise billion-scale formatting.

When changing fixtures:

1. Update `fixtures/usage/README.md`.
2. Run `python scripts/smoke_json.py`.
3. Regenerate screenshots with `pnpm run screenshots`.
4. Visually inspect at least the sources screenshot if source totals changed.

## Harness architecture

Usage ingestion is modular. Do not add collector loops directly to `main.rs`.

The extension point is:

```text
src/sources.rs
```

Every harness is represented by `UsageHarness`:

```rust
UsageHarness::new(
    "id",
    "Display Name",
    default_homes_fn,
    collect_fn,
)
```

`main.rs` should only:

1. parse CLI args,
2. build selected source IDs and home overrides,
3. call `collect_harness_records(...)`,
4. aggregate and render.

## How to add a new agent/harness

Follow this shape:

1. Add parser module:

   ```text
   src/<harness>.rs
   ```

   It should expose:

   ```rust
   pub fn collect_<harness>(home: &Path) -> anyhow::Result<Vec<UsageRecord>>
   ```

2. Add source identity in `src/model.rs`:

   ```rust
   Source::<Harness>
   ```

   Keep the serialized source name stable and lowercase in JSON.

3. Add default discovery helper in `src/paths.rs`:

   ```rust
   pub fn default_<harness>_homes() -> Vec<PathBuf>
   ```

   Support the harness's env var if it has one. Deduplicate paths.

4. Register in `src/sources.rs`:

   ```rust
   UsageHarness::new(
       "<harness>",
       "<Display Name>",
       default_<harness>_homes,
       collect_<harness>,
   )
   ```

5. Add CLI overrides in `src/cli.rs` and map them in `src/main.rs`:

   ```text
   --<harness>-home
   --<harness>-only
   ```

6. Add unit tests for parser behavior and source selection.

7. Add deterministic fixture data under `fixtures/usage/<harness>/`.

8. Update `scripts/smoke_json.py` so the new fixture source is required.

9. Update screenshots if the source/model mix changes.

## Existing harness notes

### Codex

- Module: `src/codex.rs`
- Default home: `~/.codex` or `CODEX_HOME`
- Reads current and archived JSONL session files.
- Uses `token_count` events and tracks active model changes inside a session.
- Uses per-call deltas where possible, not only final cumulative totals.

### Hermes

- Module: `src/hermes.rs`
- Homes include `%LOCALAPPDATA%/hermes` on Windows and `~/.hermes`.
- Reads `state.db` plus profile `state.db` files.
- Includes OpenAI-backed rows where `billing_provider` is `openai-codex`, `openai`, or `openai:*`.

### OpenClaw

- Module: `src/openclaw.rs`
- Default home: `~/.openclaw`, legacy fallback `~/.clawdbot`, or `OPENCLAW_STATE_DIR`.
- Reads `agents/*/sessions/*` transcript JSONL.
- Includes live `.jsonl`, `.jsonl.reset.*`, and `.jsonl.deleted.*` files.
- Skips trajectory, checkpoint, sessions metadata, and `.bak` files.
- Mirrors OpenClaw's cache normalization so cached prompt tokens are not double-counted as fresh input.

## Packaging notes

`package.json` uses explicit `files`. If README references assets, make sure those assets are included or intentionally GitHub-only.

The wrapper expects platform binaries under `bin/`:

```text
bin/dexuse-win32-x64.exe
bin/dexuse-win32-arm64.exe
bin/dexuse-darwin-x64
bin/dexuse-darwin-arm64
bin/dexuse-linux-x64
bin/dexuse-linux-arm64
```

`python scripts/bundle_current.py` copies the current platform's release binary into the right name.

## Documentation tone

README is marketing-facing. Keep it short, attractive, and human. Avoid long implementation dumps there. Put maintenance details here instead.
