"""Prepare anonymous engineering benchmark reviews and merge completed verdicts."""
import argparse
import hashlib
import json
import secrets
import shutil
import tempfile
from pathlib import Path


def load(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def lines(path):
    return [json.loads(s) for s in path.read_text(encoding="utf-8-sig").splitlines() if s.strip()]


def store(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")


def child(root, name):
    rel = Path(name)
    if rel.is_absolute() or ".." in rel.parts:
        raise ValueError("Invalid relative artifact path")
    path = (root / rel).resolve(strict=True)
    if not path.is_file() or not path.is_relative_to(root.resolve(strict=True)):
        raise ValueError("Artifact is outside its case")
    return path


def digest(path):
    with path.open("rb") as stream:
        sha = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            sha.update(block)
        return sha.hexdigest()


def prepare(frozen, arms, destination):
    tasks = {t["task_id"]: t for t in load(frozen / "operator" / "tasks.json")}
    rubrics = load(frozen / "expert-rubrics.json")
    if destination.exists():
        raise ValueError("Choose a new destination")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as tmp:
        stage = Path(tmp) / "package"
        review = stage / "review"
        review.mkdir(parents=True)
        private, cases, groups = {}, [], {}
        for arm in arms:
            rows = lines(arm / "results.jsonl")
            if not rows or len({(r["arm"], r["version"]) for r in rows}) != 1:
                raise ValueError("Each input directory must contain one nonempty versioned arm")
            label = rows[0]["arm"]
            ids = {r["task_id"] for r in rows}
            if label in groups or len(ids) != len(rows) or groups and ids != next(iter(groups.values())):
                raise ValueError("Duplicate arm or unmatched tasks")
            groups[label] = ids
            for result in rows:
                task = tasks[result["task_id"]]
                rubric = rubrics[task["task_id"]]
                canonical = json.dumps(rubric, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
                if hashlib.sha256(canonical.encode()).hexdigest() != task["rubric_sha256"] or any(
                    task[k] != result[k] for k in ("category", "capture_group", "rubric_sha256")
                ):
                    raise ValueError("Run or rubric differs from frozen task")
                token = secrets.token_hex(16)
                directory = review / token
                directory.mkdir()
                case = dict(id=token, question=task["question"], criteria=rubric,
                            deliverables=task["deliverables"], answer=None, artifacts={})
                if result["status"] == "delivered":
                    answer_path = child(arm, result["answer_file"])
                    answer = load(answer_path)
                    case["answer"] = answer["answer"]
                    for number, original in enumerate(answer.get("artifacts", []), 1):
                        src = child(answer_path.parent, original)
                        name = f"artifact-{number}{src.suffix}"
                        dest = directory / name
                        shutil.copyfile(src, dest)
                        case["artifacts"][name] = digest(dest)
                store(directory / "case.json", case)
                private[token] = dict(result, case_sha256=digest(directory / "case.json"))
                cases.append(dict(id=token, correct=None, critical_failures=[], rationale=""))
        if len(groups) < 2:
            raise ValueError("Two or more arms required")
        secrets.SystemRandom().shuffle(cases)
        store(review / "verdicts.json", cases)
        store(stage / "coordinator-key.json", private)
        stage.rename(destination)


def merge(package, verdicts, output):
    private = load(package / "coordinator-key.json")
    reviews = load(verdicts)
    if len(reviews) != len(private) or {r["id"] for r in reviews} != set(private):
        raise ValueError("Every anonymous delivery needs exactly one review")
    rows = []
    for verdict in reviews:
        row = private[verdict["id"]].copy()
        directory = package / "review" / verdict["id"]
        case = child(directory, "case.json")
        if digest(case) != row.pop("case_sha256"):
            raise ValueError("Blind case changed after packaging")
        for name, sha in load(case)["artifacts"].items():
            if digest(child(directory, name)) != sha:
                raise ValueError("Reviewed artifact changed")
        correct, critical = verdict["correct"], verdict["critical_failures"]
        if type(correct) is not bool or not isinstance(critical, list) or not all(isinstance(x,str) and x.strip() for x in critical):
            raise ValueError("Invalid expert verdict")
        if not verdict["rationale"].strip() or correct and (critical or row["status"] != "delivered"):
            raise ValueError("Missing rationale or incompatible success verdict")
        rows.append(dict(row, correct=correct, critical_failures=critical, rationale=verdict["rationale"]))
    with output.open("x", encoding="utf-8") as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest="action", required=True)
    pack = subs.add_parser("prepare")
    pack.add_argument("frozen", type=Path)
    pack.add_argument("destination", type=Path)
    pack.add_argument("arms", type=Path, nargs="+")
    join = subs.add_parser("merge")
    for key in ("package", "verdicts", "output"):
        join.add_argument(key, type=Path)
    args = parser.parse_args()
    if args.action == "prepare":
        prepare(args.frozen, args.arms, args.destination)
    else:
        merge(args.package, args.verdicts, args.output)
