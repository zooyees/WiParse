import hashlib
import json
from pathlib import Path
import zipfile

root = Path(__file__).resolve().parents[4]
source = root / "dist/WiParse-Next-20261108-Preview-R20"
output = root / "dist/WiParse-Next-20261108-R20-Test.zip"
prefix = "WiParse-Next-20261108-R20-Test/"
original = {row["path"]: row for row in json.loads((source / "files-sha256.json").read_text(encoding="utf-8-sig"))}
with zipfile.ZipFile(output) as archive:
    rows = json.loads(archive.read(prefix + "files-sha256.json"))
    assert len(rows) == len(original)
    assert len(archive.namelist()) == len(set(archive.namelist()))
    assert set(archive.namelist()) == {prefix + row["path"] for row in rows} | {prefix + "files-sha256.json"}
    for row in rows:
        data = archive.read(prefix + row["path"])
        assert len(data) == row["bytes"]
        assert hashlib.sha256(data).hexdigest() == row["sha256"]
        if row["path"] not in {"PREVIEW.txt", "VERSION-NOTES.md", "preview-manifest.json"}:
            assert row["sha256"] == original[row["path"]]["sha256"]
    manifest = json.loads(archive.read(prefix + "preview-manifest.json"))
    original_manifest = (source / "preview-manifest.json").read_bytes()
    assert manifest["zip_created"] is True
    assert manifest["source_preview_manifest_sha256"] == hashlib.sha256(original_manifest).hexdigest()
    original_manifest = json.loads(original_manifest)
    assert manifest["binary_sha256"] == original_manifest["binary_sha256"]
    assert manifest["symbol_sha256"] == original_manifest["symbol_sha256"]
checksum = hashlib.sha256()
with output.open("rb") as stream:
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        checksum.update(block)
archive_hash = checksum.hexdigest()
output.with_suffix(".zip.sha256").write_text(f"{archive_hash}  {output.name}\n", encoding="ascii")
summary = dict(archive=str(output), bytes=output.stat().st_size, files=len(rows) + 1,
               sha256=archive_hash, all_entries_verified=True, zip_created=True,
               source_preview_unchanged=True, industrial_approved=False)
(Path(__file__).parent / "delivery.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
print(json.dumps(summary, ensure_ascii=False))
