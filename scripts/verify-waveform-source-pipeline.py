"""Final-EXE simulation smoke; never opens or controls physical instruments."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import queue
import socket
import subprocess
import threading
import time
import urllib.request
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--cycles", type=int, default=10)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    audit = repo / "tmp" / ("source-pipeline-runtime-" + uuid.uuid4().hex)
    audit.mkdir(parents=True)
    exe = args.exe.resolve(strict=True)
    with socket.socket() as port_socket:
        port_socket.bind(("127.0.0.1", 0))
        port = port_socket.getsockname()[1]
    config = audit / "config.json"
    config.write_text(json.dumps({"ui": {"debug_mode": False}, "apps": {"test_tool": {"marketplace": {"enabled": False}}}}), encoding="utf-8")
    env = dict(os.environ, WIPARSE_PROJECT_ROOT=str(audit), WIPARSE_APP_ROOT=str(audit), WIPARSE_CONFIG=str(config), WIPARSE_API_BIND=f"127.0.0.1:{port}")
    for key in ("WCM_PROJECT_ROOT", "WCM_CONFIG"):
        env.pop(key, None)
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0
    base = f"http://127.0.0.1:{port}"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    latencies = []
    events = queue.Queue(maxsize=4096)
    completed = {}
    captured = []
    response = None

    def invoke(method, params):
        request = urllib.request.Request(base + "/v1/invoke", json.dumps({"method": method, "params": params}).encode(), {"Content-Type": "application/json"})
        start = time.monotonic()
        with opener.open(request, timeout=10) as result:
            reply = json.load(result)
        latencies.append((time.monotonic() - start) * 1000)
        assert reply.get("ok"), (method, reply)
        return reply["data"]

    def wait_job(job_id):
        deadline = time.monotonic() + 20
        while job_id not in completed:
            assert time.monotonic() < deadline, ("job timed out", job_id)
            try:
                event = events.get(timeout=0.05)
            except queue.Empty:
                # UI-thread method: this would stall if the old save/parse work
                # still ran there. It also exercises tab switches during work.
                invoke("ui.show", {"tab": "instruments"})
                continue
            if event.get("type") == "instrument.job_done":
                completed[event["data"]["job_id"]] = event["data"]
        return completed[job_id]

    with (audit / "stdout.log").open("wb") as stdout, (audit / "stderr.log").open("wb") as stderr:
        process = subprocess.Popen([str(exe)], cwd=audit, env=env, stdout=stdout, stderr=stderr, startupinfo=startup)
        try:
            deadline = time.monotonic() + 25
            while True:
                assert process.poll() is None, "GUI exited before readiness"
                try:
                    with opener.open(base + "/v1/health", timeout=1) as ready:
                        health = json.load(ready)
                    break
                except OSError:
                    assert time.monotonic() < deadline, "GUI API unavailable"
                    time.sleep(0.1)
            response = opener.open(base + "/v1/events", timeout=30)

            def read_events():
                try:
                    for line in response:
                        if line.strip():
                            events.put(json.loads(line), timeout=1)
                except (OSError, ValueError, queue.Full):
                    pass

            threading.Thread(target=read_events, daemon=True).start()
            scope = invoke("instrument.connect", {"resource": "DEMO::Oscilloscope", "kind": "oscilloscope"})["device_id"]
            power = invoke("instrument.connect", {"resource": "DEMO::DC Source", "kind": "dc_source"})["device_id"]
            deadline = time.monotonic() + 10
            while len(invoke("instrument.overview", {})["devices"]) != 2:
                assert time.monotonic() < deadline, "Demo sessions not connected"
                time.sleep(0.05)
            for cycle in range(args.cycles):
                job = invoke("instrument.waveform_source", {"device_id": scope, "dir": str(audit / "captures"), "filename": "capture.csv", "overwrite": False})
                query = invoke("instrument.command", {"device_id": power, "command": {"RawQuery": "*IDN?"}})
                invoke("ui.show", {"tab": ["serial", "calculator", "instruments", "test_report"][cycle % 4]})
                result = wait_job(job["job_id"])
                assert result["ok"], result
                assert wait_job(query["job_id"])["ok"]
                path = Path(result["detail"]["path"])
                assert path.is_file(), result
                assert path not in captured, "overwrite=False reused an existing file"
                with path.open(encoding="utf-8-sig", newline="") as source:
                    assert sum(1 for _ in csv.reader(source)) > 64
                assert result["detail"]["metrics"]["CH1"], result
                captured.append(path)
            invalid = audit / "not-a-directory"
            invalid.write_text("keep", encoding="utf-8")
            failed = invoke("instrument.waveform_source", {"device_id": scope, "dir": str(invalid)})
            assert not wait_job(failed["job_id"])["ok"], "Save failure was reported as success"
            recovery = invoke("instrument.waveform_source", {"device_id": scope, "dir": str(audit / "recovered")})
            assert wait_job(recovery["job_id"])["ok"], "Save failure did not release the next operation"
            inventory = invoke("instrument.overview", {})
            assert len(inventory["devices"]) == 2
            ordered = sorted(latencies)
            result = {"kind": "simulated_source_pipeline_final_exe", "passed": True, "cycles": args.cycles,
                      "version": health["data"]["version"], "exe_sha256": hashlib.sha256(exe.read_bytes()).hexdigest(),
                      "physical_hardware_test": False, "interactive_save_dialog_test": False,
                      "save_error_recovery": True, "second_instrument_commands": True,
                      "unique_output_files": len(captured), "ui_invoke_count": len(latencies),
                      "ui_invoke_p95_ms": ordered[min(len(ordered) - 1, int(len(ordered) * .95))],
                      "ui_invoke_max_ms": max(ordered)}
            (audit / "result.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
            print(json.dumps(result, ensure_ascii=False))
            print("Evidence:", audit)
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
            if response is not None:
                response.close()


if __name__ == "__main__":
    main()
