#!/usr/bin/env python3
"""Fail honestly until every audited upstream behavior has completion evidence."""
import collections
import json
import pathlib
import sys

root = pathlib.Path(__file__).resolve().parent.parent
manifest = json.loads((root / "parity/upstream.json").read_text())
statuses = json.loads((root / "parity/status.json").read_text())
counts = collections.Counter()
errors = []
for key in manifest["actions"] + manifest["modules"]:
    entry = statuses.get(key, {"status": "missing", "evidence": []})
    counts[entry["status"]] += 1
    if entry["status"] not in {"implemented", "inapplicable"}:
        errors.append(f"{key}: {entry['status']}")
        continue
    if not entry.get("evidence") or (entry["status"] == "inapplicable" and not entry.get("note")):
        errors.append(f"{key}: lacks completion evidence or rationale")
    for evidence in entry.get("evidence", []):
        file, _, symbol = evidence.partition("::")
        path = root / file
        if not path.is_file() or (symbol and symbol not in path.read_text()):
            errors.append(f"{key}: invalid evidence {evidence}")
print("Upstream parity: " + ", ".join(f"{count} {state}" for state, count in sorted(counts.items())))
if errors:
    print("Full parity is not achieved. See parity/PARITY.md.")
    sys.exit(1)
print("Every audited item has recorded completion evidence.")
