"""Compare matched, independently reviewed hardware-agent benchmark runs."""
import argparse
import json
import math
from collections import Counter
from pathlib import Path


def score(rows):
    correct = sum(row["correct"] for row in rows)
    critical = Counter(reason for row in rows for reason in row["critical_failures"])
    n = len(rows)
    z = 1.959963984540054
    p = correct / n
    scale = 1 + z*z/n
    middle = (p + z*z/(2*n)) / scale
    margin = z * math.sqrt(p*(1-p)/n + z*z/(4*n*n)) / scale
    times = sorted(row["elapsed_ms"] for row in rows)
    have_cost = all(row["total_tokens"] is not None for row in rows)
    return dict(tasks=n, correct=correct, rate=p, wilson_95=[middle-margin, middle+margin],
                critical_failures=dict(critical), missing_usage=sum(row["total_tokens"] is None for row in rows),
                tokens_per_correct=sum(row["total_tokens"] for row in rows)/correct if have_cost and correct else None,
                elapsed_ms_p50=times[math.ceil(.5*n)-1], elapsed_ms_p95=times[math.ceil(.95*n)-1])


def compare(rows):
    arms, captures, tasks = {}, {}, {}
    for row in rows:
        for field in ("arm", "task_id", "category", "capture_group", "rubric_sha256"):
            if not isinstance(row.get(field), str) or not row[field].strip():
                raise ValueError("Missing " + field)
        if type(row.get("correct")) is not bool or not isinstance(row.get("critical_failures"), list):
            raise ValueError("Expert verdict and critical_failures are required")
        if not all(isinstance(s,str) and s for s in row["critical_failures"]) or row["critical_failures"] and row["correct"]:
            raise ValueError("Invalid critical failure verdict")
        if type(row.get("elapsed_ms")) is not int or row["elapsed_ms"] < 0:
            raise ValueError("Invalid elapsed_ms")
        if "total_tokens" not in row or row["total_tokens"] is not None and (type(row["total_tokens"]) is not int or row["total_tokens"] < 0):
            raise ValueError("Use null for missing usage")
        task, group = row["task_id"], row["capture_group"]
        identity = group, row["category"], row["rubric_sha256"]
        if tasks.setdefault(task,identity) != identity or captures.setdefault(group,task) != task:
            raise ValueError("Task, rubric or independent capture mismatch")
        arm = arms.setdefault(row["arm"], {})
        if task in arm:
            raise ValueError("Duplicate task in arm")
        arm[task] = row
    if len(arms)<2 or any(set(items)!=set(tasks) for items in arms.values()):
        raise ValueError("Two or more arms with identical tasks required")
    return {name: dict(overall=score(list(items.values())), by_category={
        category:score([r for r in items.values() if r["category"]==category])
        for category in sorted({r["category"] for r in items.values()})}) for name,items in arms.items()}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("results",type=Path,help="JSONL with one reviewed task per arm and capture")
    args=parser.parse_args()
    try:
        rows=[json.loads(line) for line in args.results.read_text(encoding="utf-8-sig").splitlines() if line.strip()]
        print(json.dumps(compare(rows),indent=2,ensure_ascii=False))
    except (ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
