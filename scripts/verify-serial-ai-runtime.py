"""Final-EXE serial regression with loopback data only; no physical devices."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import threading
import time
import urllib.request
import uuid


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    exe = args.exe.resolve(strict=True)
    audit = repo / "tmp" / ("serial-ai-runtime-" + uuid.uuid4().hex)
    audit.mkdir(parents=True)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        api_port = probe.getsockname()[1]
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    listener.settimeout(0.2)
    address = f"tcp://127.0.0.1:{listener.getsockname()[1]}"
    done = threading.Event()
    errors = []

    def feed():
        try:
            for cycle in range(3):
                while not done.is_set():
                    try:
                        connection, _ = listener.accept()
                        break
                    except socket.timeout:
                        continue
                else:
                    return
                with connection:
                    connection.settimeout(0.2)
                    lines = [f"cycle={cycle} seq={n} VIN=5.00V " + ("ERROR timeout" if n == 37 else "OK") for n in range(80)]
                    connection.sendall(("\n".join(lines) + "\n").encode())
                    while not done.is_set():
                        try:
                            if not connection.recv(1):
                                break
                        except socket.timeout:
                            continue
        except OSError as error:
            if not done.is_set():
                errors.append(str(error))

    server = threading.Thread(target=feed, daemon=True)
    server.start()
    config = audit / "config.json"
    config.write_text(json.dumps({"ai": {"enabled": False}, "log_monitor": {"save_live_to_disk": False}, "apps": {"test_tool": {"plugins_dir": str(exe.parent.parent / "test-tools/plugins"), "data_root": ".", "marketplace": {"enabled": False}}}}), encoding="utf-8")
    env = dict(os.environ, WIPARSE_PROJECT_ROOT=str(audit), WIPARSE_APP_ROOT=str(audit), WIPARSE_CONFIG=str(config), WIPARSE_API_BIND=f"127.0.0.1:{api_port}")
    for name in ["WCM_PROJECT_ROOT", "WCM_CONFIG"]:
        env.pop(name, None)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base = f"http://127.0.0.1:{api_port}"
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = 0

    def invoke(method, params):
        request = urllib.request.Request(base + "/v1/invoke", json.dumps({"method": method, "params": params}).encode(), {"Content-Type": "application/json"})
        with opener.open(request, timeout=10) as response:
            reply = json.load(response)
        assert reply.get("ok"), (method, reply)
        return reply["data"]

    results = []
    with (audit / "stdout.log").open("wb") as stdout, (audit / "stderr.log").open("wb") as stderr:
        process = subprocess.Popen([str(exe)], cwd=audit, env=env, startupinfo=startup, stdout=stdout, stderr=stderr)
        try:
            deadline = time.monotonic() + 25
            while True:
                assert process.poll() is None, "GUI exited during startup"
                try:
                    with opener.open(base + "/v1/health", timeout=1) as response:
                        health = json.load(response)
                    break
                except OSError:
                    assert time.monotonic() < deadline, "GUI API unavailable"
                    time.sleep(0.1)
            invoke("ui.show", {"tab": "serial"})
            for cycle in range(3):
                before = invoke("log.lines.get", {"tab_id": 0, "limit": 1})["total"]
                invoke("serial.monitor.start", {"port": address, "baud": 115200})
                deadline = time.monotonic() + 10
                while True:
                    data = invoke("log.lines.get", {"port": address, "from_row": before, "limit": 200})
                    if data["total"] >= before + 80:
                        break
                    assert time.monotonic() < deadline, ("serial receive stalled", data)
                    assert process.poll() is None
                    time.sleep(0.05)
                assert len(data["lines"]) == 80, data
                assert any(f"cycle={cycle} seq=37" in line and "ERROR timeout" in line for line in data["lines"])
                invoke("ui.show", {"tab": "test_tool"})
                invoke("ui.show", {"tab": "serial"})
                invoke("serial.monitor.stop", {"port": address})
                status = invoke("serial.monitor.status", {})
                assert not status["monitoring"], status
                results.append({"cycle": cycle, "received": 80, "stop_confirmed": True})
                time.sleep(0.15)
            assert not errors, errors
            result = {"ok": True, "exe": str(exe), "sha256": hashlib.sha256(exe.read_bytes()).hexdigest(), "health": health, "cycles": results, "scope": "native EXE serial API regression; AI dialog covered separately by egui/HTTP/TCP tests", "physical_hardware_tested": False}
            (audit / "verification.json").write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps(dict(result, audit=str(audit)), ensure_ascii=False, indent=2))
        finally:
            done.set()
            listener.close()
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
            server.join(timeout=2)


if __name__ == "__main__":
    main()
