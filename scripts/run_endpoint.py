"""Run the ignored endpoint suite against an owned loopback dynoxide process.

Build the revision pinned in CI first, then pass its binary as the argument.
This runner supplies dummy credentials; real-service tests use cargo directly.
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time
import urllib.error
import urllib.request


def endpoint_test_count(output):
    names = re.findall(r"^(.+): test$", output, re.M)
    totals = re.findall(r"^(\d+) tests?, 0 benchmarks$", output, re.M)
    if not names or len(names) != len(set(names)) or totals != [str(len(names))]:
        raise RuntimeError("endpoint inventory must list every ignored test and be nonempty")
    return len(names)


def require_endpoint_evidence(output, expected):
    summary = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", output, re.M)
    # Ordinary offline tests are filtered by --ignored. Every inventoried endpoint
    # test must run and pass, so filtering even one of those cannot satisfy this gate.
    if expected <= 0 or summary != [(str(expected), "0", "0")]:
        raise RuntimeError(f"endpoint suite did not execute and pass all {expected} required tests")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--port", type=int, default=18038)
    parser.add_argument("--log", type=Path, default=Path("dynoxide.log"))
    args = parser.parse_args()
    endpoint = f"http://127.0.0.1:{args.port}"
    env = dict(os.environ, TABULARY_DYNAMODB_ENDPOINT=endpoint,
               AWS_REGION="us-east-1", AWS_ACCESS_KEY_ID="dummy",
               AWS_SECRET_ACCESS_KEY="dummy", AWS_EC2_METADATA_DISABLED="true")
    env.pop("AWS_SESSION_TOKEN", None)
    # Do not send local readiness probes through a workstation proxy.
    probe = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with args.log.open("w", encoding="utf-8") as log:
        process = subprocess.Popen(
            [str(args.binary.resolve()), "serve", "--host", "127.0.0.1", "--port", str(args.port)],
            stdout=log, stderr=subprocess.STDOUT, env=env,
        )
        try:
            deadline = time.monotonic() + 30
            while True:
                if process.poll() is not None:
                    raise RuntimeError(f"dynoxide exited before readiness; see {args.log}")
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f"dynoxide readiness deadline exceeded; see {args.log}")
                try:
                    # Require this child's successful bind before probing. An unrelated
                    # server occupying the port must never satisfy emulator readiness.
                    started = f"Dynoxide listening on {endpoint}" in args.log.read_text(encoding="utf-8")
                    if started:
                        stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
                        request = urllib.request.Request(endpoint, data=b"{}", headers={
                            "Content-Type": "application/x-amz-json-1.0",
                            "X-Amz-Target": "DynamoDB_20120810.ListTables",
                            "X-Amz-Date": stamp,
                            # Dynoxide validates auth material but does not verify signatures.
                            "Authorization": f"AWS4-HMAC-SHA256 Credential=dummy/{stamp[:8]}/us-east-1/dynamodb/aws4_request, "
                                             f"SignedHeaders=host;x-amz-date, Signature={'0' * 64}",
                        })
                        with probe.open(request, timeout=min(1, remaining)) as response:
                            payload = json.load(response)
                            tables = payload.get("TableNames") if isinstance(payload, dict) else None
                            if response.status == 200 and isinstance(tables, list):
                                break
                except (OSError, urllib.error.URLError, ValueError):
                    pass
                time.sleep(min(0.1, max(0, deadline - time.monotonic())))
            print(f"dynoxide ready at {endpoint}", flush=True)
            command = ["cargo", "test", "-p", "tabulary", "--test", "endpoint", "--locked", "--", "--ignored"]
            options = dict(cwd=Path(__file__).resolve().parents[1], env=env, timeout=600,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            inventory = subprocess.run([*command, "--list", "--format=pretty"], **options)
            inventory.check_returncode()
            expected = endpoint_test_count(inventory.stdout)
            result = subprocess.run(
                [*command, "--nocapture"], **options,
            )
            print(result.stdout, end="", flush=True)
            result.check_returncode()
            require_endpoint_evidence(result.stdout, expected)
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


if __name__ == "__main__":
    main()
