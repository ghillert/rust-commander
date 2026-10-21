#!/usr/bin/env python3
"""Offline catalog/provenance tests; no application or network access required."""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("icon_import", ROOT / "scripts/icons.py")
icons = importlib.util.module_from_spec(spec)
spec.loader.exec_module(icons)


class IconCatalogTests(unittest.TestCase):
    def setUp(self):
        self.catalog = json.loads(icons.MANIFEST.read_text(encoding="utf-8"))

    def test_committed_catalog_checks_offline(self):
        with mock.patch.object(sys, "argv", ["icons.py", "--check"]), mock.patch.object(
            icons, "download", side_effect=AssertionError("--check must never access network")
        ):
            icons.main()

    def test_missing_and_unused_assets_are_rejected(self):
        broken = copy.deepcopy(self.catalog)
        broken["actions"]["Broken"] = "missing-asset"
        with self.assertRaises(ValueError):
            icons.validate_catalog(broken)
        broken = copy.deepcopy(self.catalog)
        broken["assets"]["unused"] = broken["assets"]["file"]
        with self.assertRaises(ValueError):
            icons.validate_catalog(broken)

    def test_duplicate_filename_rules_are_rejected(self):
        broken = copy.deepcopy(self.catalog)
        broken["file_rules"].append({"icon": "file", "extensions": ["pdf"]})
        with self.assertRaises(ValueError):
            icons.validate_catalog(broken)

    def test_svg_is_self_contained_and_gradients_are_allowed(self):
        icons.validate_svg(b'<svg viewBox="0 0 20 20"><defs><linearGradient id="a"/></defs><path fill="url(#a)"/></svg>')
        for body in [b'<script/>', b'<text>Font dependency</text>', b'<image href="https://example.org/a.png"/>', b'<use href="file:///tmp/icon.svg"/>']:
            with self.assertRaises(ValueError):
                icons.validate_svg(b'<svg viewBox="0 0 20 20">' + body + b'</svg>')

    def test_recorded_generic_folder_transformation_is_reproducible(self):
        source = b'<svg viewBox="0 0 16 16"><path id="folder"/><path id="motive"/></svg>'
        asset = {"transforms": ["remove-folder-motive"]}
        output = icons.transform(source, asset)
        self.assertEqual(output, icons.transform(source, asset))
        self.assertNotIn(b'motive', output)
        self.assertIn(b'folder', output)

    def test_interrupted_fetch_reuses_verified_assets_and_repairs_corruption(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            local = root / "ui/icons"
            shutil.copytree(icons.ICONS, local)
            shutil.copyfile(ROOT / "NOTICE", root / "NOTICE")
            # One corrupt asset succeeds while another download is interrupted.
            (local / "vendor/file-default.svg").write_bytes(b"corrupt")
            (local / "vendor/pdf-default.svg").unlink()
            calls = []
            original_icons = icons.ICONS

            def fetch(source, path):
                calls.append(path)
                asset = "pdf" if path == self.catalog["assets"]["pdf"]["variants"]["default"]["path"] else "file"
                if asset == "pdf" and calls.count(path) == 1:
                    raise OSError("interrupted download")
                return (original_icons / f"vendor/{asset}-default.svg").read_bytes()

            with mock.patch.multiple(
                icons, ROOT=root, ICONS=local, MANIFEST=local / "catalog.json",
                RUST=root / "src/icons/catalog.rs", SLINT=local / "catalog.slint",
            ), mock.patch.object(sys, "argv", ["icons.py", "--fetch"]), mock.patch.object(icons, "download", side_effect=fetch):
                with self.assertRaisesRegex(ValueError, "interrupted download"):
                    icons.main()
                self.assertNotEqual((local / "vendor/file-default.svg").read_bytes(), b"corrupt")
                icons.main()
                self.assertEqual(len(calls), 3, "retry must fetch only the interrupted asset")
                with mock.patch.object(icons, "download", side_effect=AssertionError("verified assets must be reused")):
                    icons.main()
                    with mock.patch.object(sys, "argv", ["icons.py", "--check"]):
                        icons.main()

    def test_application_svg_references_use_the_shared_catalog(self):
        for path in [ROOT / "ui/chrome.slint", ROOT / "ui/commander.slint"]:
            self.assertNotIn("@image-url(", path.read_text(), str(path))


if __name__ == "__main__":
    unittest.main()
