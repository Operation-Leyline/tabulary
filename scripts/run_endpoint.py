"""Run the ignored endpoint suite against an owned loopback dynoxide process.

Build the revision pinned in CI first, then pass its binary as the argument.
This runner supplies dummy credentials; real-service tests use cargo directly.
"""

import argparse
import os
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request


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
                        with probe.open(endpoint, timeout=min(1, remaining)) as response:
                            if response.status == 200:
                                break
                except (OSError, urllib.error.URLError):
                    pass
                time.sleep(min(0.1, max(0, deadline - time.monotonic())))
            print(f"dynoxide ready at {endpoint}", flush=True)
            subprocess.run(
                ["cargo", "test", "-p", "tabulary", "--test", "endpoint", "--locked",
                 "--", "--ignored", "--nocapture"],
                cwd=Path(__file__).resolve().parents[1], env=env, check=True, timeout=600,
            )
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


if __name__ == "__main__":
    main()
