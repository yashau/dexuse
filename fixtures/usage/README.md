# Usage fixtures

Deterministic synthetic usage fixtures used by smoke tests and screenshot generation.

The fixture tree now covers all built-in ingestion harnesses so UI captures exercise cross-source aggregation instead of only Codex data:

- `codex/` — Codex CLI JSONL session files under `sessions/`.
- `hermes/` — Hermes `state.db` SQLite database with OpenAI/OpenAI-Codex rows plus one ignored non-OpenAI provider row.
- `openclaw/` — OpenClaw transcript JSONL files under `agents/main/sessions/`, including excluded trajectory/checkpoint files that should not affect totals.

The data is synthetic but shaped like real records and includes varied input, cached input, cache-write, output, reasoning, provider, model, and source values. GPT-5.5 events remain intentionally scaled high so charts exercise large-number formatting while Hermes and OpenClaw have meaningful totals that show up in source/model breakdowns.

Models represented:

- `gpt-5.5`
- `gpt-5.4`
- `gpt-5.4-mini`
- `gpt-5.3-codex-spark`

Default date window for fixture screenshots: `2026-06-01` through `2026-06-06`.
