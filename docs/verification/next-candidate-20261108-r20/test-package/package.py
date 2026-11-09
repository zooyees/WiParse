"""Archive the immutable verified preview, updating only delivery metadata in ZIP."""
import hashlib
import json
from pathlib import Path
from datetime import datetime, timezone
import zipfile

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / "dist/WiParse-Next-20261108-Preview-R20"
OUTPUT = ROOT / "dist/WiParse-Next-20261108-R20-Test.zip"
PREFIX = "WiParse-Next-20261108-R20-Test/"

def digest(data):
    return hashlib.sha256(data).hexdigest()

if OUTPUT.exists():
    raise SystemExit("Existing test archives are preserved; choose another output name")
original = json.loads((SOURCE / "files-sha256.json").read_text(encoding="utf-8-sig"))
expected = {entry["path"]: entry for entry in original}
actual = {path.relative_to(SOURCE).as_posix() for path in SOURCE.rglob("*") if path.is_file()}
if actual != set(expected) | {"files-sha256.json"}:
    raise SystemExit("Preview file set differs from verified delivery")
verification = json.loads((SOURCE / "candidate-verification.json").read_text(encoding="utf-8-sig"))
if not verification["automatic_passed"] or not verification["sources_unchanged"]:
    raise SystemExit("Candidate verification did not pass")
rows = []
with zipfile.ZipFile(OUTPUT, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
    for relative, entry in sorted(expected.items()):
        data = (SOURCE / relative).read_bytes()
        if len(data) != entry["bytes"] or digest(data) != entry["sha256"]:
            raise RuntimeError(f"Verified preview changed: {relative}")
        if relative == "preview-manifest.json":
            value = json.loads(data)
            value.update(kind="tauri_test_package", zip_created=True,
                         archive_created_at=datetime.now(timezone.utc).isoformat(),
                         source_preview_manifest_sha256=digest(data))
            data = (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
        elif relative in {"PREVIEW.txt", "VERSION-NOTES.md"}:
            text = data.decode("utf-8-sig")
            text = text.replace("未生成完整 ZIP", "已生成本测试 ZIP").replace("未生成 ZIP", "已生成本测试 ZIP")
            data = text.encode("utf-8")
        archive.writestr(PREFIX + relative, data)
        rows.append(dict(path=relative, bytes=len(data), sha256=digest(data)))
    inventory = (json.dumps(rows, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    archive.writestr(PREFIX + "files-sha256.json", inventory)

# Reopen and verify decompressed contents, not just archive creation success.
with zipfile.ZipFile(OUTPUT) as archive:
    if set(archive.namelist()) != {PREFIX + row["path"] for row in rows} | {PREFIX + "files-sha256.json"}:
        raise RuntimeError("Archive file set mismatch")
    if archive.testzip() is not None:
        raise RuntimeError("Archive CRC verification failed")
    for row in rows:
        data = archive.read(PREFIX + row["path"])
        if len(data) != row["bytes"] or digest(data) != row["sha256"]:
            raise RuntimeError(f"Archive content mismatch: {row['path']}")
checksum = hashlib.sha256()
with OUTPUT.open("rb") as stream:
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        checksum.update(block)
archive_hash = checksum.hexdigest()
OUTPUT.with_suffix(".zip.sha256").write_text(f"{archive_hash}  {OUTPUT.name}\n", encoding="ascii")
summary = dict(archive=str(OUTPUT), bytes=OUTPUT.stat().st_size, files=len(rows) + 1,
               sha256=archive_hash, all_entries_verified=True, zip_created=True,
               source_preview_unchanged=True, industrial_approved=False)
(Path(__file__).parent / "delivery.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
print(json.dumps(summary, ensure_ascii=False))
