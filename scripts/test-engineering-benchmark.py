"""End-to-end synthetic regression for the blind-review workflow."""
import json
import runpy
import sys
import tempfile
from pathlib import Path

scripts = Path(__file__).resolve().parent


def module(stage):
    return runpy.run_path(str(scripts / (stage + "-engineering-benchmark.py")))


def verify(root):
    (root / "sample.log").write_text("VIN=5.0\n", encoding="utf-8")
    case = dict(task_id="a", category="data", capture_group="g1", split="holdout",
                sources=["sample.log"], question="Extract voltage", deliverables=["value"],
                expert_rubric={"criteria": ["5.0 V"]})
    manifest = root / "tasks.json"
    manifest.write_text(json.dumps({"tasks": [case]}), encoding="utf-8")
    frozen = root / "frozen"
    module("freeze")["freeze"](manifest, frozen)
    runner = [sys.executable, "-c", 'import json; from pathlib import Path; Path("values.csv").write_text("5.0"); print(json.dumps(dict(answer="5.0 V",artifacts=["values.csv"],total_tokens=20)))']
    for arm in ("baseline", "candidate"):
        module("run")["run"](frozen / "operator", root / arm, arm, "fixture", "holdout", 20, runner)
    # The child may fail or time out after an already billed request.
    import uuid
    ledger=root/"usage.jsonl"
    first, second = str(uuid.uuid4()), str(uuid.uuid4())
    records=[dict(request_id=first,phase="started"),dict(request_id=first,phase="response",total_tokens=37)]
    ledger.write_text("\n".join(map(json.dumps,records+[records[-1]])),encoding="utf-8")
    observe=module("run")["request_usage"]
    assert observe(ledger)["total_tokens"]==37
    with ledger.open("a",encoding="utf-8") as out:out.write("\n"+json.dumps(dict(request_id=second,phase="started")))
    partial=observe(ledger)
    assert partial["known_tokens"]==37 and partial["total_tokens"] is None and partial["requests"]==2
    emitter = 'import os,json,uuid,time,sys; from pathlib import Path; key=str(uuid.uuid4()); ledger=Path(os.environ["WIPARSE_BENCHMARK_USAGE_FILE"]); ledger.write_text(json.dumps(dict(request_id=key,phase="started"))+"\\n"+json.dumps(dict(request_id=key,phase="response",total_tokens=23))+"\\n"); '
    for name,suffix in [("failed","sys.exit(2)"),("killed","time.sleep(6)")]:
        module("run")["run"](frozen/"operator",root/name,name,"fixture","holdout",2,[sys.executable,"-c",emitter+suffix])
        result=json.loads((root/name/"results.jsonl").read_text())
        assert result["total_tokens"]==23 and result["status"]==("timeout" if name=="killed" else "runner_error")
    review = module("review")
    blind = root / "blind"
    review["prepare"](frozen, [root / "baseline", root / "candidate"], blind)
    verdicts = blind / "review/verdicts.json"
    entries = json.loads(verdicts.read_text(encoding="utf-8"))
    for entry in entries:
        entry.update(correct=True, rationale="Checked source and CSV")
    verdicts.write_text(json.dumps(entries), encoding="utf-8")
    joined = root / "scores.jsonl"
    review["merge"](blind, verdicts, joined)
    rows = [json.loads(s) for s in joined.read_text(encoding="utf-8").splitlines()]
    scores = runpy.run_path(str(scripts / "compare-engineering-agent.py"))["compare"](rows)
    assert len(scores) == 2 and all(v["overall"]["tokens_per_correct"] == 20 for v in scores.values())
    next((blind / "review").glob("*/artifact-1.csv")).write_text("changed")
    try:
        review["merge"](blind, verdicts, root / "invalid.jsonl")
    except ValueError as error:
        assert "artifact changed" in str(error)
    else:
        raise AssertionError("Changed CSV accepted")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as directory:
        verify(Path(directory))
    print("PASS synthetic freeze, run, blind review, merge, score, and changed artifact rejection")
