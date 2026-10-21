#!/usr/bin/env python3
"""Import pinned icon artwork and generate the offline Rust/Slint catalog.

--fetch downloads missing assets from the revisions in ui/icons/catalog.json.
--refresh deliberately replaces checksums after changing a pinned revision.
--check validates local assets and generated files without accessing the network.
"""
import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
import re
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "ui/icons"
MANIFEST = ICONS / "catalog.json"
RUST = ROOT / "src/icons/catalog.rs"
SLINT = ICONS / "catalog.slint"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def download(source, path):
    url = f'https://raw.githubusercontent.com/{source["repository"]}/{source["revision"]}/'
    request = urllib.request.Request(url + urllib.parse.quote(path), headers={"User-Agent": "Rust-Commander-icons"})
    with urllib.request.urlopen(request, timeout=45) as response:
        return response.read()


def validate_svg(data):
    root = ET.fromstring(data)
    if root.tag.split("}")[-1] != "svg" or not root.get("viewBox"):
        raise ValueError("SVG must have a viewBox")
    for node in root.iter():
        if node.tag.split("}")[-1] in {"script", "text", "foreignObject", "image"}:
            raise ValueError("Icons cannot depend on scripts, fonts, or embedded raster images")
        for name, value in node.attrib.items():
            if name.split("}")[-1] == "href" and not value.startswith("#"):
                raise ValueError("External SVG references are not allowed")
            if re.search(r"url\(\s*['\"]?(?!#)[^\s'\"]", value):
                raise ValueError("External SVG resources are not allowed")


def transform(data, asset):
    for operation in asset.get("transforms", []):
        ET.register_namespace("", "http://www.w3.org/2000/svg")
        root = ET.fromstring(data)
        if operation == "remove-folder-motive":
            for parent in root.iter():
                for child in list(parent):
                    if child.get("id") == "motive":
                        parent.remove(child)
        elif operation == "tint-gradient":
            colors = asset.get("palette", [])
            if len(colors) != 2 or not all(re.fullmatch(r"#[0-9A-Fa-f]{6}", c) for c in colors):
                raise ValueError("tint-gradient requires two RGB palette colors")
            painted = False
            for node in root.iter():
                if node.get("fill") == "#212121":
                    node.set("fill", "url(#rc-palette)")
                    painted = True
            if not painted:
                raise ValueError("tint-gradient expected Fluent filled artwork")
            tag = "{http://www.w3.org/2000/svg}"
            defs = ET.SubElement(root, tag + "defs")
            gradient = ET.SubElement(defs, tag + "linearGradient", {
                "id": "rc-palette", "x1": "0%", "y1": "0%", "x2": "100%", "y2": "100%",
            })
            for offset, color in zip(["0", "1"], colors):
                ET.SubElement(gradient, tag + "stop", {"offset": offset, "stop-color": color})
        else:
            raise ValueError(f"Unknown SVG transformation: {operation}")
        data = ET.tostring(root, encoding="utf-8") + b"\n"
    return data


def artifacts(catalog):
    for source_id, source in catalog["sources"].items():
        if not re.fullmatch(r"[0-9a-f]{40}", source["revision"]):
            raise ValueError(f"{source_id}: revision must be an immutable commit SHA")
        yield source, source["license"], ICONS / "licenses" / f"{source_id}-MIT.txt"
    for icon_id, icon in catalog["assets"].items():
        if not re.fullmatch(r"[a-z0-9-]+", icon_id):
            raise ValueError(f"Invalid semantic icon ID: {icon_id}")
        for variant, asset in icon["variants"].items():
            yield catalog["sources"][icon["source"]], asset, ICONS / "vendor" / f"{icon_id}-{variant}.svg"


def validate_catalog(catalog):
    ids = set(catalog["assets"])
    referenced = {catalog[key] for key in ["default_action", "default_place", "file_default", "folder_default"]}
    referenced.update(catalog["actions"].values())
    referenced.update(place["icon"] for place in catalog["places"])
    names, extensions, prefixes = set(), set(), set()
    for rule in catalog["file_rules"]:
        referenced.add(rule["icon"])
        for field, seen in [("names", names), ("extensions", extensions), ("prefixes", prefixes)]:
            for value in rule.get(field, []):
                if value != value.lower() or not value.isascii() or value in seen:
                    raise ValueError(f"Duplicate/noncanonical {field} rule: {value}")
                seen.add(value)
    if referenced != ids:
        raise ValueError(f"Missing/unused catalog assets: {referenced ^ ids}")
    for icon in catalog["assets"].values():
        if icon["treatment"] not in {"monochrome", "color"} or "default" not in icon["variants"]:
            raise ValueError("Invalid rendering treatment or missing default variant")


def generated(catalog):
    quote = json.dumps
    header = "// Generated by scripts/icons.py from ui/icons/catalog.json. Do not edit.\n"
    slint = [header, "export global IconCatalog {\n", "    public pure function image(id: string, dark: bool) -> image {\n"]
    for icon_id, icon in sorted(catalog["assets"].items()):
        if "light" in icon["variants"]:
            slint.append(f'        if id == {quote(icon_id)} && !dark {{ return @image-url("vendor/{icon_id}-light.svg"); }}\n')
        slint.append(f'        if id == {quote(icon_id)} {{ return @image-url("vendor/{icon_id}-default.svg"); }}\n')
    slint.append(f'        return @image-url("vendor/{catalog["file_default"]}-default.svg");\n    }}\n')
    slint.append("    public pure function monochrome(id: string) -> bool {\n")
    for icon_id, icon in sorted(catalog["assets"].items()):
        if icon["treatment"] == "monochrome":
            slint.append(f"        if id == {quote(icon_id)} {{ return true; }}\n")
    slint.append("        return false;\n    }\n    public pure function action(id: string) -> string {\n")
    for action, icon_id in catalog["actions"].items():
        slint.append(f"        if id == {quote(action)} {{ return {quote(icon_id)}; }}\n")
    slint.append(f'        return {quote(catalog["default_action"])};\n    }}\n')
    slint.append("    public pure function place(id: string) -> string {\n")
    for place in catalog["places"]:
        condition = f'id.starts-with({quote(place["prefix"])})' if "prefix" in place else f'id == {quote(place["key"])}'
        slint.append(f'        if {condition} {{ return {quote(place["icon"])}; }}\n')
    slint.append(f'        return {quote(catalog["default_place"])};\n    }}\n}}\n')
    rust = [header, "pub(super) struct Rule {\n    pub icon: &'static str,\n    pub names: &'static [&'static str],\n    pub extensions: &'static [&'static str],\n    pub prefixes: &'static [&'static str],\n}\n\n#[rustfmt::skip]\npub(super) const RULES: &[Rule] = &[\n"]
    for rule in catalog["file_rules"]:
        arrays = ["&[" + ", ".join(quote(v) for v in rule.get(field, [])) + "]" for field in ["names", "extensions", "prefixes"]]
        rust.append(f'    Rule {{ icon: {quote(rule["icon"])}, names: {arrays[0]}, extensions: {arrays[1]}, prefixes: {arrays[2]} }},\n')
    rust.append("];\n")
    rust.append(f'pub(super) const FILE: &str = {quote(catalog["file_default"])};\n')
    rust.append(f'pub(super) const FOLDER: &str = {quote(catalog["folder_default"])};\n')
    rust.append('#[cfg(feature = "screenshot")]\n#[rustfmt::skip]\npub(super) const IDS: &[&str] = &[' + ", ".join(quote(v) for v in sorted(catalog["assets"])) + '];\n')
    return {RUST: "".join(rust), SLINT: "".join(slint)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fetch", action="store_true")
    parser.add_argument("--refresh", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.check and (args.fetch or args.refresh) or args.refresh and not args.fetch:
        parser.error("--check is offline; --refresh requires --fetch")
    catalog = json.loads(MANIFEST.read_text(encoding="utf-8"))
    validate_catalog(catalog)
    all_artifacts = list(artifacts(catalog))

    def process(item):
        source, asset, destination = item
        valid = destination.exists() and digest(destination.read_bytes()) == asset.get("output_sha256", asset.get("sha256"))
        if args.fetch and (args.refresh or not valid):
            try:
                data = download(source, asset["path"])
            except Exception as error:
                raise ValueError(f'Cannot import {source["repository"]}/{asset["path"]}: {error}') from error
            if destination.suffix == ".svg":
                validate_svg(data)
            if asset.get("sha256") and digest(data) != asset["sha256"] and not args.refresh:
                raise ValueError(f"Checksum mismatch: {destination.name}")
            asset["sha256"] = digest(data)
            data = transform(data, asset)
            asset["output_sha256"] = digest(data)
            destination.parent.mkdir(parents=True, exist_ok=True)
            temporary = destination.with_suffix(destination.suffix + ".tmp")
            temporary.write_bytes(data)
            temporary.replace(destination)
        data = destination.read_bytes()
        if digest(data) != asset.get("output_sha256", asset.get("sha256")):
            raise ValueError(f"Checksum mismatch: {destination.name}")
        if destination.suffix == ".svg":
            validate_svg(data)
        return destination

    destinations, errors = set(), []
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
        for future in concurrent.futures.as_completed([executor.submit(process, item) for item in all_artifacts]):
            try:
                destinations.add(future.result())
            except Exception as error:
                errors.append(str(error))
    if args.fetch:
        MANIFEST.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")
    if errors:
        raise ValueError("\n".join(errors))
    for file in (ICONS / "vendor").glob("*.svg"):
        if file not in destinations:
            if args.check:
                raise ValueError(f"Unused vendored icon: {file}")
            file.unlink()
    files = generated(catalog)
    notice_path = ROOT / "NOTICE"
    base = re.sub(r"\n*--- BEGIN VENDORED ICON NOTICES ---.*?--- END VENDORED ICON NOTICES ---\n*", "", notice_path.read_text(encoding="utf-8"), flags=re.S).rstrip()
    notice = [base, "\n\n--- BEGIN VENDORED ICON NOTICES ---\n"]
    for source_id, source in catalog["sources"].items():
        notice.append(f'\n{source_id}: https://github.com/{source["repository"]}\nPinned revision: {source["revision"]}\n\n')
        notice.append((ICONS / "licenses" / f"{source_id}-MIT.txt").read_text(encoding="utf-8").strip() + "\n")
    notice.append("\n--- END VENDORED ICON NOTICES ---\n")
    files[notice_path] = "".join(notice)
    for path, content in files.items():
        if args.check:
            if path.read_text(encoding="utf-8") != content:
                raise ValueError(f"Stale generated file: {path.relative_to(ROOT)}")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
    notice = (ROOT / "NOTICE").read_text(encoding="utf-8")
    for path in (ICONS / "licenses").glob("*.txt"):
        if path.read_text(encoding="utf-8").strip() not in notice:
            raise ValueError(f"Packaged NOTICE must contain {path.name}")
    print(f'Icon catalog: {len(catalog["assets"])} icons, {len(destinations)} pinned assets/notices verified')


if __name__ == "__main__":
    main()
