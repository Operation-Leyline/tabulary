"""Failure paths must never turn an unexecuted endpoint suite into a pass."""

from pathlib import Path
import io
import subprocess
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import run_endpoint


class RunnerFailures(unittest.TestCase):
    def run_ready(self, output):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "dynoxide.log"
            child = MagicMock()
            child.poll.return_value = None
            def start(*args, **kwargs):
                kwargs["stdout"].write("Dynoxide listening on http://127.0.0.1:18038\n")
                kwargs["stdout"].flush()
                return child
            response = io.BytesIO(b'{"TableNames":[]}')
            response.status = 200
            with (patch("sys.argv", ["runner", "dynoxide", "--log", str(log)]),
                  patch.object(run_endpoint.subprocess, "Popen", side_effect=start),
                  patch.object(run_endpoint.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)),
                  patch.object(run_endpoint.urllib.request, "build_opener") as opener,
                  patch("builtins.print")):
                opener.return_value.open.return_value.__enter__.return_value = response
                run_endpoint.main()
                return opener.return_value.open.call_args.args[0]

    def test_runner_rejects_successful_cargo_with_zero_tests(self):
        with self.assertRaises(RuntimeError):
            self.run_ready("test result: ok. 0 passed; 0 failed; 0 ignored;\n")

    def test_readiness_uses_the_dynamodb_data_path(self):
        request = self.run_ready("test result: ok. 3 passed; 0 failed; 0 ignored;\n")
        self.assertEqual(request.get_method(), "POST")
        self.assertEqual(request.data, b"{}")
        self.assertEqual(request.get_header("X-amz-target"), "DynamoDB_20120810.ListTables")
        self.assertIn("/us-east-1/dynamodb/aws4_request", request.get_header("Authorization"))

    def test_zero_tests_is_unexecuted_not_success(self):
        with self.assertRaises(RuntimeError):
            run_endpoint.require_endpoint_evidence("test result: ok. 0 passed; 0 failed; 0 ignored;")

    def test_all_expected_endpoint_tests_must_pass_without_ignores(self):
        run_endpoint.require_endpoint_evidence("test result: ok. 3 passed; 0 failed; 0 ignored;")
        for output in ["", "test result: ok. 2 passed; 0 failed; 1 ignored;",
                       "test result: FAILED. 2 passed; 1 failed; 0 ignored;"]:
            with self.assertRaises(RuntimeError):
                run_endpoint.require_endpoint_evidence(output)

    def run_failure(self, *, polls, times, expected, marker=""):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "dynoxide.log"
            child = MagicMock()
            child.poll.side_effect = polls

            def start(*args, **kwargs):
                kwargs["stdout"].write(marker)
                kwargs["stdout"].flush()
                return child

            with (
                patch("sys.argv", ["run_endpoint.py", "dynoxide", "--log", str(log)]),
                patch.object(run_endpoint.subprocess, "Popen", side_effect=start),
                patch.object(run_endpoint.subprocess, "run") as cargo,
                patch.object(run_endpoint.time, "monotonic", side_effect=times),
                patch.object(run_endpoint.time, "sleep"),
                patch.object(run_endpoint.urllib.request, "build_opener") as opener,
            ):
                with self.assertRaises(expected):
                    run_endpoint.main()
                cargo.assert_not_called()
                child.terminate.assert_called_once()
                child.wait.assert_called_once_with(timeout=5)
                return opener.return_value

    def test_startup_exit_is_failure(self):
        self.run_failure(polls=[1], times=[0], expected=RuntimeError)

    def test_readiness_has_a_deadline(self):
        self.run_failure(polls=[None], times=[0, 31], expected=TimeoutError)

    def test_other_server_cannot_satisfy_readiness(self):
        opener = self.run_failure(polls=[None, 1], times=[0, 0, 0], expected=RuntimeError)
        opener.open.assert_not_called()


if __name__ == "__main__":
    unittest.main()
