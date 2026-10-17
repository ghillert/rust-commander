#!/usr/bin/env python3
"""Record a reproducible upstream feature inventory; never infer completion."""
import argparse
import json
import pathlib
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("upstream", type=pathlib.Path)
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parent.parent
source = args.upstream
revision = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
actions = re.findall(r'^\s+(\w+)\("[^"]+"\)', (source / "mucommander-os-api/src/main/java/com/mucommander/desktop/ActionType.java").read_text(), re.M)
modules = re.findall(r"^include '(mucommander-(?:protocol|format|viewer)-[^']+)'", (source / "settings.gradle").read_text(), re.M)
out = root / "parity"
out.mkdir(exist_ok=True)
manifest = {"repository": "https://github.com/mucommander/mucommander", "revision": revision, "actions": actions, "modules": modules}
(out / "upstream.json").write_text(json.dumps(manifest, indent=2) + "\n")
status_path = out / "status.json"
status = json.loads(status_path.read_text()) if status_path.exists() else {}
for key in actions + modules:
    status.setdefault(key, {"status": "missing", "evidence": []})
status_path.write_text(json.dumps(status, indent=2, sort_keys=True) + "\n")
lines = ["# Upstream parity", "", f"Baseline: `{revision}`", "", "Completion requires implemented behavior and validation evidence. A menu entry,", "adapter dependency, or untested backend does not establish parity. Runtime-only", "Java actions require an explicit rationale if they are inapplicable to Rust.", "", "| Upstream action or module | Status | Evidence |", "| --- | --- | --- |"]
for key in actions + modules:
    value = status[key]
    evidence = ', '.join(value['evidence']) or '—'
    if value.get('note'): evidence += ' — ' + value['note']
    lines.append(f"| {key} | {value['status']} | {evidence} |")
(out / "PARITY.md").write_text("\n".join(lines) + "\n")
print(f"Recorded {len(actions)} actions and {len(modules)} modules at {revision}.")
