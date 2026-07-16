#!/usr/bin/env python3
"""Recover WiParse-Rust by replaying transcript edits in chronological order."""

from __future__ import annotations

import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(r"D:\windlink\windlink\WiParse-Rust")
TRANSCRIPT_DIR = Path(
    r"C:\Users\roy zhao\.cursor\projects\d-windlink-windlink-WiParse\agent-transcripts\337e84b0-ce09-4135-acd7-86d2dc8d9218"
)


def normalize_path(raw: str) -> Path | None:
    p = raw.replace("/", "\\")
    for marker in (r"D:\windlink\windlink\WiParse-Rust", r"D:\windlink\windlink\WiParse_Rust"):
        idx = p.find(marker)
        if idx >= 0:
            rel = p[idx + len(marker) :].lstrip("\\/")
            return ROOT / rel
    return None


@dataclass
class Stats:
    write: int = 0
    strreplace: int = 0
    patch: int = 0
    delete: int = 0
    skipped: int = 0


def apply_patch_text(path: Path, patch: str, stats: Stats) -> None:
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
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("\n".join(body_lines) + ("\n" if body_lines else ""), encoding="utf-8")
        stats.patch += 1
        return

    m = re.search(r"\*\*\* Update File: (.+)\n", patch)
    if not m:
        return
    target = normalize_path(m.group(1).strip())
    if target is None or not target.exists():
        stats.skipped += 1
        return
    text = target.read_text(encoding="utf-8")
    changed = False
    for hunk in patch.split("@@")[1:]:
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
            changed = True
    if changed:
        target.write_text(text, encoding="utf-8")
        stats.patch += 1
    else:
        stats.skipped += 1


def replay(events: list[tuple], stats: Stats) -> None:
    for name, inp in events:
        if name == "Write":
            target = normalize_path(inp.get("path", ""))
            if target is None:
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(inp.get("contents", ""), encoding="utf-8", newline="\n")
            stats.write += 1
        elif name == "StrReplace":
            target = normalize_path(inp.get("path", ""))
            if target is None or not target.exists():
                stats.skipped += 1
                continue
            text = target.read_text(encoding="utf-8")
            old = inp.get("old_string", "")
            new = inp.get("new_string", "")
            if inp.get("replace_all"):
                if old not in text:
                    stats.skipped += 1
                    continue
                text = text.replace(old, new)
            elif old in text:
                text = text.replace(old, new, 1)
            else:
                stats.skipped += 1
                continue
            target.write_text(text, encoding="utf-8", newline="\n")
            stats.strreplace += 1
        elif name == "ApplyPatch":
            raw = inp if isinstance(inp, str) else inp.get("patch") or inp.get("raw") or ""
            if not raw:
                continue
            m = re.search(r"\*\*\* (?:Add|Update) File: (.+)\n", raw)
            if not m:
                stats.skipped += 1
                continue
            target = normalize_path(m.group(1).strip())
            if target is None:
                continue
            apply_patch_text(target, raw, stats)
        elif name == "Delete":
            target = normalize_path(inp.get("path", ""))
            if target and target.exists():
                target.unlink()
                stats.delete += 1


def collect_events() -> list[tuple]:
    events: list[tuple] = []
    for fp in sorted(TRANSCRIPT_DIR.rglob("*.jsonl")):
        with fp.open("r", encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if not line.startswith("{"):
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
                    if isinstance(item, dict) and item.get("type") == "tool_use":
                        name = item.get("name")
                        inp = item.get("input") or {}
                        if name in {"Write", "StrReplace", "ApplyPatch", "Delete"}:
                            events.append((name, inp))
    return events


def main() -> int:
    stats = Stats()
    events = collect_events()
    replay(events, stats)
    rust_files = [p for p in ROOT.rglob("*.rs") if "third_party" not in p.parts]
    print(
        json.dumps(
            {
                "events": len(events),
                "stats": stats.__dict__,
                "project_rust_files": len(rust_files),
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
