"""Freeze a local engineering benchmark and separate operator tasks from expert answers."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tempfile

CATEGORIES = {"serial", "wave", "data", "hardware", "report"}


def digest(path):
    with path.open("rb") as stream:
        sha = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            sha.update(block)
        return sha.hexdigest()


def freeze(manifest, destination):
    root = manifest.resolve().parent
    spec = json.loads(manifest.read_text(encoding="utf-8-sig"))
    cases, groups, identifiers = [], {}, set()
    answers = {}
    destination = destination.resolve()
    if destination.exists():
        raise ValueError("The destination must be new")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as staging:
        stage = Path(staging) / "frozen"
        files = stage / "operator" / "sources"
        files.mkdir(parents=True)
        for case in spec["tasks"]:
            task, group, split = (case[k] for k in ("task_id", "capture_group", "split"))
            if not all(isinstance(v,str) and v.strip() for v in (task,group,case["question"])):
                raise ValueError("Missing task identity, capture group or request")
            if task in identifiers or split not in {"development","holdout"}:
                raise ValueError(f"Duplicate task or unknown split: {task}")
            identifiers.add(task)
            if groups.setdefault(group,split) != split:
                raise ValueError(f"Capture group crosses splits: {group}")
            if case["category"] not in CATEGORIES or not case["sources"] or not case["deliverables"]:
                raise ValueError(f"Missing benchmark inputs: {task}")
            gold = case["expert_rubric"]
            if not isinstance(gold,dict) or not gold.get("criteria"):
                raise ValueError(f"Missing frozen scoring criteria: {task}")
            refs = []
            for rel in case["sources"]:
                source = (root / rel).resolve(strict=True)
                if Path(rel).is_absolute() or not source.is_relative_to(root) or not source.is_file():
                    raise ValueError(f"Source outside the benchmark directory: {rel}")
                sha = digest(source)
                # Keep formats usable by the instrument importers.
                name = sha + source.suffix.lower()
                target = files / name
                if not target.exists():
                    shutil.copyfile(source,target)
                    if digest(target) != sha:
                        raise ValueError("The source changed while the benchmark was being frozen")
                refs.append(dict(file="sources/"+name,sha256=sha))
            rubric_hash = hashlib.sha256(json.dumps(gold,ensure_ascii=False,sort_keys=True,separators=(",",":")).encode()).hexdigest()
            cases.append(dict(task_id=task,capture_group=group,category=case["category"],split=split,
                              question=case["question"],deliverables=case["deliverables"],sources=refs,rubric_sha256=rubric_hash))
            answers[task] = gold
        if not cases:
            raise ValueError("Empty corpus")
        # The same input bytes cannot enter development and holdout under new group names.
        membership = {}
        for case in cases:
            for source in case["sources"]:
                if membership.setdefault(source["sha256"],case["split"])!=case["split"]:
                    raise ValueError("Identical input appears in both splits")
        (stage/"operator"/"tasks.json").write_text(json.dumps(cases,ensure_ascii=False,indent=2),encoding="utf-8")
        (stage/"expert-rubrics.json").write_text(json.dumps(answers,ensure_ascii=False,indent=2),encoding="utf-8")
        stage.rename(destination)


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest",type=Path)
    parser.add_argument("destination",type=Path)
    args=parser.parse_args()
    freeze(args.manifest,args.destination)
