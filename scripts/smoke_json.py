import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE_ROOT = ROOT / "fixtures" / "usage"
FIXTURE_CODEX_HOME = FIXTURE_ROOT / "codex"
FIXTURE_HERMES_HOME = FIXTURE_ROOT / "hermes"
FIXTURE_OPENCLAW_HOME = FIXTURE_ROOT / "openclaw"
out = ROOT / ".mise-smoke.json"

with out.open("w", encoding="utf-8", newline="\n") as handle:
    subprocess.run(
        [
            "node",
            "scripts/dexuse.js",
            "--json",
            "--codex-home",
            str(FIXTURE_CODEX_HOME),
            "--hermes-home",
            str(FIXTURE_HERMES_HOME),
            "--openclaw-home",
            str(FIXTURE_OPENCLAW_HOME),
            "--from",
            "2026-06-01",
            "--to",
            "2026-06-06",
            "--granularity",
            "day",
        ],
        check=True,
        cwd=ROOT,
        stdout=handle,
    )

try:
    data = json.loads(out.read_text(encoding="utf-8"))
    sources = {k: v["total_tokens"] for k, v in data["by_source"].items()}
    missing_sources = {"codex", "hermes", "openclaw"} - set(sources)
    assert not missing_sources, f"fixture smoke missing sources: {sorted(missing_sources)}"
    assert all(total > 0 for total in sources.values()), sources
    print("fixtures", FIXTURE_ROOT)
    print("records", data["records"])
    print("total", data["total"]["total_tokens"])
    print("models", {k: v["total_tokens"] for k, v in data["by_model"].items()})
    print("providers", {k: v["total_tokens"] for k, v in data["by_provider"].items()})
    print("sources", sources)
finally:
    out.unlink(missing_ok=True)
