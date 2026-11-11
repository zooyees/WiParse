"""Run one benchmark arm against immutable, operator-visible cases."""
import argparse
import hashlib
import json
import os
import subprocess
import time
import uuid
from pathlib import Path


def run(operator_dir, destination, arm, version, split, timeout, command):
    operator_dir = operator_dir.resolve(strict=True)
    tasks=json.loads((operator_dir/"tasks.json").read_text(encoding="utf-8-sig"))
    destination.mkdir(parents=True,exist_ok=False)
    for task in tasks:
        if task["split"]!=split:
            continue
        case_dir=destination/uuid.uuid4().hex
        case_dir.mkdir()
        verified=[]
        for source in task["sources"]:
            path=(operator_dir/source["file"]).resolve(strict=True)
            if not path.is_relative_to(operator_dir):
                raise ValueError("Input escapes benchmark root")
            with path.open("rb") as stream:
                sha = hashlib.sha256()
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    sha.update(block)
                digest = sha.hexdigest()
            if digest!=source["sha256"]:
                raise ValueError("Frozen capture changed: "+source["file"])
            verified.append(dict(source,path=str(path)))
        request=dict(task,sources=verified)
        request.pop("rubric_sha256")
        started=time.monotonic()
        status="failed"
        with (case_dir/"answer.json").open("xb") as out,(case_dir/"runner.log").open("xb") as err:
            try:
                invocation=subprocess.run(command,input=json.dumps(request,ensure_ascii=False).encode("utf-8"),
                    cwd=case_dir,stdout=out,stderr=err,timeout=timeout,check=False,shell=False)
                status="delivered" if invocation.returncode==0 else "runner_error"
            except subprocess.TimeoutExpired:
                status="timeout"
            except OSError as error:
                err.write(str(error).encode("utf-8",errors="replace"))
        elapsed=round((time.monotonic()-started)*1000)
        result=dict(arm=arm,version=version,task_id=task["task_id"],category=task["category"],
            capture_group=task["capture_group"],rubric_sha256=task["rubric_sha256"],status=status,
            elapsed_ms=elapsed,answer_file=case_dir.name+"/answer.json",total_tokens=None)
        if status=="delivered":
            try:
                answer_path=case_dir/"answer.json"
                if answer_path.stat().st_size>16*1024*1024: raise ValueError("Answer too large")
                answer=json.loads(answer_path.read_text(encoding="utf-8"))
                if not isinstance(answer.get("answer"),str) or not answer["answer"].strip():
                    raise ValueError("Missing answer")
                cost=answer.get("total_tokens")
                if type(cost) is int and cost>=0:result["total_tokens"]=cost
            except (ValueError,AttributeError,UnicodeError):
                result["status"]="invalid_delivery"
        with (destination/"results.jsonl").open("a",encoding="utf-8") as summary:
            summary.write(json.dumps(result,ensure_ascii=False)+"\n")
            summary.flush()
            os.fsync(summary.fileno())


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operator_dir",type=Path)
    parser.add_argument("destination",type=Path)
    parser.add_argument("--arm",required=True)
    parser.add_argument("--version",required=True)
    parser.add_argument("--split",choices=["development","holdout"],required=True)
    parser.add_argument("--timeout",type=int,default=600)
    parser.add_argument("--runner",nargs=argparse.REMAINDER,required=True,
                        help="Absolute executable and arguments. Reads one JSON task on stdin, writes a JSON answer")
    args=parser.parse_args()
    if not args.runner or args.timeout<=0:parser.error("Provide a runner and positive timeout")
    run(args.operator_dir,args.destination,args.arm,args.version,args.split,args.timeout,args.runner)
