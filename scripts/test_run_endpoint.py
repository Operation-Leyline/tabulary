"""Failure paths must never turn an unexecuted endpoint suite into a pass."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import run_endpoint


class RunnerFailures(unittest.TestCase):
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
