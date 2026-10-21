#!/usr/bin/env python3
"""Record a reproducible upstream feature inventory; never infer completion."""
import argparse
import collections
import json
import pathlib
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("upstream", type=pathlib.Path, nargs="?")
parser.add_argument(
    "--render-only",
    action="store_true",
    help="Regenerate PARITY.md from checked-in data without changing the upstream baseline",
)
args = parser.parse_args()
if args.render_only and args.upstream is not None:
    parser.error("--render-only does not accept an upstream checkout")
if not args.render_only and args.upstream is None:
    parser.error("provide an upstream checkout, or use --render-only")

root = pathlib.Path(__file__).resolve().parent.parent
out = root / "parity"
out.mkdir(exist_ok=True)
if args.render_only:
    manifest = json.loads((out / "upstream.json").read_text())
else:
    source = args.upstream
    revision = subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "HEAD"], text=True
    ).strip()
    actions = re.findall(
        r'^\s+(\w+)\("[^"]+"\)',
        (source / "mucommander-os-api/src/main/java/com/mucommander/desktop/ActionType.java").read_text(),
        re.M,
    )
    modules = re.findall(
        r"^include '(mucommander-(?:protocol|format|viewer)-[^']+)'",
        (source / "settings.gradle").read_text(),
        re.M,
    )
    manifest = {
        "repository": "https://github.com/mucommander/mucommander",
        "revision": revision,
        "actions": actions,
        "modules": modules,
    }
    (out / "upstream.json").write_text(json.dumps(manifest, indent=2) + "\n")

status_path = out / "status.json"
status = json.loads(status_path.read_text()) if status_path.exists() else {}
keys = manifest["actions"] + manifest["modules"]
for key in keys:
    status.setdefault(key, {"status": "missing", "evidence": []})
status_path.write_text(json.dumps(status, indent=2, sort_keys=True, ensure_ascii=False) + "\n")
counts = collections.Counter(status[key]["status"] for key in keys)
summary = ", ".join(f"{counts[state]} {state}" for state in ["implemented", "partial", "missing", "inapplicable"])
lines = [
    "# Upstream parity",
    "",
    f"Baseline: `{manifest['revision']}`",
    "",
    "Generated from [upstream.json](upstream.json), [status.json](status.json) and",
    "the reviewed workflow summary in [OVERVIEW.md](OVERVIEW.md). Edit those source",
    "files and run `python3 scripts/audit-upstream.py --render-only` from the repository",
    "root to regenerate this page without changing the pinned baseline.",
    "",
    "Completion requires implemented behavior and validation evidence. A menu entry,",
    "adapter dependency, or untested backend does not establish parity. Runtime-only",
    "Java actions require an explicit rationale if they are inapplicable to Rust.",
    "",
    "| Status | Meaning |",
    "| --- | --- |",
    "| implemented | Recorded behavior and validation evidence cover the audited item. |",
    "| partial | Behavior exists, but implementation details, upstream compatibility or validation remain open; the note identifies which. |",
    "| missing | The audited behavior has no implementation; evidence may explain a related feature or why it differs. |",
    "| inapplicable | The item does not apply to this Rust application, with an explicit rationale. |",
    "",
    f"Inventory: **{len(keys)} entries — {summary}**.",
    "These are action/module counts, not a percentage of product functionality.",
    "Cross-cutting workflows such as drag and drop are reviewed separately below.",
    "The completion gate checks the enumerated inventory; the workflow review",
    "remains necessary when assessing product-wide parity.",
    "",
    (out / "OVERVIEW.md").read_text().rstrip(),
    "",
    "## Detailed action and module inventory",
    "",
    "| Upstream action or module | Status | Evidence |",
    "| --- | --- | --- |",
]
for key in keys:
    value = status[key]
    evidence = ", ".join(value["evidence"]) or "—"
    if value.get("note"):
        evidence += " — " + value["note"]
    evidence = evidence.replace("|", "\\|").replace("\n", " ")
    lines.append(f"| {key} | {value['status']} | {evidence} |")
(out / "PARITY.md").write_text("\n".join(lines) + "\n")
print(f"Rendered {len(manifest['actions'])} actions and {len(manifest['modules'])} modules at {manifest['revision']}.")
print("Upstream parity: " + summary)
