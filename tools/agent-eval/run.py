"""Record actual WiParse sessions once, then grade the immutable receipts with Promptfoo."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
CASES = json.loads((HERE / "cases.json").read_text(encoding="utf8"))
TEST = "serial_tool::ai_chat::live_eval::promptfoo_live_serial_session"


def build_host():
    cmd = ["cargo", "test", "--locked", "--offline", "-p", "wiparse-gui", "--no-run", "--message-format=json"]
    artifacts = subprocess.check_output(cmd, cwd=ROOT)
    for line in reversed(artifacts.splitlines()):
        rec = json.loads(line)
        if rec.get("executable") and rec["target"]["name"] == "wiparse-gui" and rec["profile"]["test"]:
            return Path(rec["executable"])
    raise RuntimeError("WiParse GUI test host missing")


def record(host, out, model, case, config):
    with tempfile.TemporaryDirectory(prefix="wiparse-agent-eval-") as directory:
        env = dict(os.environ, WIPARSE_PROJECT_ROOT=directory, WIPARSE_CONFIG=str(Path(directory)/"config.json"),
                   WCM_CONFIG=str(Path(directory)/"config.json"), WIPARSE_EVAL_CONFIG=str(config),
                   WIPARSE_EVAL_PROVIDER=model, WIPARSE_EVAL_CASE=case)
        try:
            subprocess.run([host, "--exact", TEST, "--ignored", "--test-threads=1"], cwd=directory,
                           env=env, check=True, timeout=520, capture_output=True)
            trace = json.loads((Path(directory)/"trajectory.json").read_text(encoding="utf8"))
        except (subprocess.SubprocessError, OSError, ValueError) as exc:
            trace = {"availability":"harness_error", "error":type(exc).__name__}
        trace.update(provider=model, case=case)
        (out/f"{model}-{case}.json").write_text(json.dumps(trace, ensure_ascii=False, indent=2), encoding="utf8")
        return trace


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--providers", nargs="+", choices=["deepseek","qwen"], default=["deepseek","qwen"])
    parser.add_argument("--cases", nargs="+", choices=list(CASES), default=list(CASES),
                        help="Run only selected scenarios to avoid repeating paid calls")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--grade-only", action="store_true")
    mode.add_argument("--collect-only", action="store_true")
    parser.add_argument("--host", type=Path, help="Previously compiled GUI test executable")
    parser.add_argument("--config", type=Path, default=ROOT/"config.json")
    args = parser.parse_args()
    out = args.out.resolve()
    models = list(dict.fromkeys(args.providers))
    cases = list(dict.fromkeys(args.cases))
    if not args.grade_only:
        out.mkdir(parents=True, exist_ok=False)
        host = args.host.resolve() if args.host else build_host()
        for model in models:
            for case in cases:
                if model == "qwen" and not os.environ.get("DASHSCOPE_API_KEY"):
                    trace = {"provider":model,"case":case,"availability":"unavailable","error":"DASHSCOPE_API_KEY unavailable"}
                    (out/f"{model}-{case}.json").write_text(json.dumps(trace),encoding="utf8")
                else:
                    trace = record(host,out,model,case,args.config.resolve())
                print(model,case,trace["availability"],trace.get("error",""),flush=True)
    available = [m for m in models if any(json.loads((out/f"{m}-{c}.json").read_text(encoding="utf8"))["availability"]!="unavailable" for c in cases)]
    if args.collect_only: return 0
    if not available: return 1
    env = dict(os.environ, WIPARSE_EVAL_MODELS=",".join(available), WIPARSE_EVAL_CASES=",".join(cases), WIPARSE_EVAL_TRACES=str(out),
               PROMPTFOO_DISABLE_TELEMETRY="1", PROMPTFOO_CONFIG_DIR=str(out/"promptfoo-state"))
    # Grading is a local replay and requires no model credentials.
    for key in ("DASHSCOPE_API_KEY", "DEEPSEEK_API_KEY", "QWEN_API_KEY", "OPENAI_API_KEY"):
        env.pop(key, None)
    package = json.loads((HERE/"node_modules/promptfoo/package.json").read_text(encoding="utf8"))
    entry = package["bin"]
    cli = HERE/"node_modules/promptfoo"/(entry["promptfoo"] if isinstance(entry,dict) else entry)
    return subprocess.run(["node", str(cli), "eval", "-c", str(HERE/"promptfooconfig.cjs"),
                           "--no-cache","-j","1","-o",str(out/"promptfoo-results.json")],cwd=HERE,env=env).returncode


if __name__ == "__main__":
    raise SystemExit(main())
