#!/usr/bin/env python3
"""Recover WiParse-Rust sources by replaying Cursor agent transcript edits."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(r"D:\windlink\windlink\WiParse-Rust")
TRANSCRIPT_DIR = Path(
    r"C:\Users\roy zhao\.cursor\projects\d-windlink-windlink-WiParse\agent-transcripts\337e84b0-ce09-4135-acd7-86d2dc8d9218"
)


def normalize_path(raw: str) -> Path | None:
    p = raw.replace("/", "\\")
    markers = [
        r"D:\windlink\windlink\WiParse-Rust",
        r"D:\windlink\windlink\WiParse_Rust",
    ]
    for marker in markers:
        idx = p.find(marker)
        if idx >= 0:
            rel = p[idx + len(marker) :].lstrip("\\/")
            return ROOT / rel
    return None


def apply_patch_text(patch: str) -> None:
    if "*** Add File:" in patch:
        m = re.search(r"\*\*\* Add File: (.+)\n", patch)
        if not m:
            return
        target = normalize_path(m.group(1).strip())
        if target is None:
            return
        body_lines = []
        for line in patch.splitlines():
            if line.startswith("+") and not line.startswith("+++"):
                body_lines.append(line[1:])
            elif line.startswith("***"):
                continue
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("\n".join(body_lines) + ("\n" if body_lines else ""), encoding="utf-8", newline="\n")
        return

    m = re.search(r"\*\*\* Update File: (.+)\n", patch)
    if not m:
        return
    target = normalize_path(m.group(1).strip())
    if target is None or not target.exists():
        return
    text = target.read_text(encoding="utf-8")
    hunks = patch.split("@@")
    for hunk in hunks[1:]:
        lines = hunk.splitlines()
        if not lines:
            continue
        old_lines: list[str] = []
        new_lines: list[str] = []
        for line in lines[1:]:
            if not line or line.startswith("***"):
                continue
            if line.startswith("-"):
                old_lines.append(line[1:])
            elif line.startswith("+"):
                new_lines.append(line[1:])
            elif line.startswith(" "):
                old_lines.append(line[1:])
                new_lines.append(line[1:])
        if not old_lines:
            continue
        old = "\n".join(old_lines)
        new = "\n".join(new_lines)
        if old in text:
            text = text.replace(old, new, 1)
    target.write_text(text, encoding="utf-8", newline="\n")


def apply_patch(path: Path, patch: str) -> None:
    apply_patch_text(patch)


def replay_jsonl(path: Path, stats: dict) -> None:
    with path.open("r", encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line or not line.startswith("{"):
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            if obj.get("role") != "assistant":
                continue
            content = obj.get("message", {}).get("content")
            if not isinstance(content, list):
                continue
            for item in content:
                if not isinstance(item, dict) or item.get("type") != "tool_use":
                    continue
                name = item.get("name")
                inp = item.get("input") or {}
                if name == "Write":
                    target = normalize_path(inp.get("path", ""))
                    if target is None:
                        continue
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(inp.get("contents", ""), encoding="utf-8", newline="\n")
                    stats["write"] += 1
                elif name == "StrReplace":
                    target = normalize_path(inp.get("path", ""))
                    if target is None or not target.exists():
                        continue
                    text = target.read_text(encoding="utf-8")
                    old = inp.get("old_string", "")
                    new = inp.get("new_string", "")
                    if inp.get("replace_all"):
                        text = text.replace(old, new)
                    elif old in text:
                        text = text.replace(old, new, 1)
                    else:
                        continue
                    target.write_text(text, encoding="utf-8", newline="\n")
                    stats["strreplace"] += 1
                elif name == "ApplyPatch":
                    raw = inp if isinstance(inp, str) else ""
                    if isinstance(inp, dict):
                        raw = inp.get("patch") or inp.get("raw") or ""
                    if raw:
                        apply_patch_text(raw)
                        stats["patch"] += 1
                elif name == "Delete":
                    target = normalize_path(inp.get("path", ""))
                    if target and target.exists():
                        target.unlink()
                        stats["delete"] += 1


def main() -> int:
    stats = {"write": 0, "strreplace": 0, "patch": 0, "delete": 0}
    files = sorted(TRANSCRIPT_DIR.rglob("*.jsonl"))
    if not files:
        print("No transcript files found", file=sys.stderr)
        return 1
    for fp in files:
        replay_jsonl(fp, stats)
    rust_files = list(ROOT.rglob("*.rs"))
    toml_files = list(ROOT.rglob("Cargo.toml"))
    print(json.dumps({"stats": stats, "rust_files": len(rust_files), "cargo_tomls": len(toml_files)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
