"""Summarize WiParse request and task measurements from local application logs."""
import argparse
import json
import math
import re
from collections import Counter, defaultdict
from pathlib import Path


def summarize(lines, reviews=None):
    requests = defaultdict(list)
    outcomes = {}
    periodic = []
    for line in lines:
        line = re.sub(r"\x1b\[[0-9;]*m", "", line)
        if "wiparse_ai_usage" in line and "metrics={" in line:
            try:
                item, _ = json.JSONDecoder().raw_decode(line.split("metrics=", 1)[1])
            except ValueError:
                continue
            if item.get("task_id"):
                requests[item["task_id"]].append(item)
            else:
                periodic.append(item)
        elif "wiparse_ai_outcome" in line:
            fields = dict(re.findall(r'(\w+)=("?[-\w.]+"?)', line))
            usage = {}
            if "usage={" in line:
                try:
                    usage, _ = json.JSONDecoder().raw_decode(line.split("usage=", 1)[1])
                except ValueError:
                    pass
            if "task_id" in fields and "outcome" in fields:
                outcomes[fields["task_id"].strip('"')] = fields["outcome"].strip('"'), fields, usage

    token_totals = []
    completion_ms = []
    first_evidence_ms = []
    for task, (outcome, fields, usage) in outcomes.items():
        if outcome != "completed":
            continue
        rows = requests.get(task, [])
        totals = [(row.get("usage") or {}).get("total_tokens") for row in rows]
        if (totals and usage.get("responses") == len(rows) and usage.get("failed_attempts") == 0
                and all(row.get("success") and isinstance(n, int) for row, n in zip(rows, totals))):
            token_totals.append(sum(totals))
        for key, values in (("elapsed_ms", completion_ms), ("first_evidence_ms", first_evidence_ms)):
            value = fields.get(key, "")
            if value.isdigit():
                values.append(int(value))

    def distribution(values):
        if not values:
            return {"tasks": 0, "p50": None, "p95": None}
        values = sorted(values)
        return {"tasks": len(values), **{
            key: values[math.ceil(len(values) * frac) - 1]
            for key, frac in (("p50", .5), ("p95", .95))}}

    # Costs include failed attempts, including tasks without a terminal event.
    # A missing provider usage value is unknown; counting it as zero would
    # reward transports which omit errors from their usage responses.
    all_requests = [item for rows in requests.values() for item in rows]
    def known_tokens(item):
        usage = item.get("usage") or {}
        value = usage.get("total_tokens")
        return value if type(value) is int and value >= 0 else None

    costs = [known_tokens(item) for item in all_requests]
    reviewed = set(reviews or {})
    correct = sum(value is True for value in (reviews or {}).values())
    observed = set(requests) | set(outcomes)
    complete_cohort = bool(reviewed) and reviewed == observed
    observed_tokens = sum(n for n in costs if n is not None)
    missing = sum(n is None for n in costs)
    accounting_complete = all(
        (usage.get("responses", 0) + usage.get("failed_attempts", 0)) == len(requests.get(task, []))
        for task, (_, _, usage) in outcomes.items()
    ) and set(requests) == set(outcomes)
    return {
        "quality_cost": {
            "observed_requests": len(costs), "requests_missing_usage": missing,
            "observed_tokens_including_failures": observed_tokens,
            "expert_reviewed_tasks": len(reviewed), "expert_correct_tasks": correct,
            "matched_review_cohort": complete_cohort,
            "tokens_per_correct_delivery": observed_tokens / correct
            if correct and complete_cohort and accounting_complete and not missing and costs else None,
            "excluded_unattributed_requests": len(periodic),
        },
        "observed_terminal_tasks": dict(Counter(v[0] for v in outcomes.values())),
        "request_tasks_without_terminal_event": len(requests.keys() - outcomes.keys()),
        "requests_without_task_id": len(periodic),
        "tokens_per_completed_task_with_known_usage": distribution(token_totals),
        "completed_task_latency_ms": distribution(completion_ms),
        "completed_task_first_evidence_ms": distribution(first_evidence_ms),
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+", type=Path, help="Non-overlapping application logs, oldest first")
    parser.add_argument("--reviews", type=Path, help="Expert verdicts as a JSON object mapping task IDs to true/false")
    args = parser.parse_args()
    reviews = json.loads(args.reviews.read_text(encoding="utf-8-sig")) if args.reviews else None
    if reviews is not None and (not isinstance(reviews, dict) or any(type(v) is not bool for v in reviews.values())):
        parser.error("--reviews requires a JSON object with boolean values")

    def read_logs():
        for path in args.logs:
            with path.open(encoding="utf-8", errors="replace") as stream:
                yield from stream

    print(json.dumps(summarize(read_logs(), reviews), indent=2, ensure_ascii=False))
