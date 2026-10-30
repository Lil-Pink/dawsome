"""Tests of scripts/capped.sh against a stand-in systemd-run on the PATH, which records how it
was called and runs the command after its `--`: a run starts one memory-capped scope and marks
it with DAWSOME_CAPPED=1; a run inside one (the variable set, as for scripts/check.sh's cargo
runs under a capped shell) runs the command in place, never a scope inside a scope. Why: a
nested scope's cap counts against the outer one's, and each start costs a round trip to the
user's systemd.

Run (from the repository root; scripts/check.sh runs it):
  python3 -I -B tests/scripts/capped_test.py
"""
import os
import stat
import subprocess
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(ROOT, "scripts", "capped.sh")

# The stand-in: one line of its arguments per call, then the command after "--".
STAND_IN = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$CAPPED_TEST_LOG"
while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do shift; done
shift
exec "$@"
"""


class CappedTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="dawsome-capped-")
        bin_dir = os.path.join(self.temp.name, "bin")
        os.mkdir(bin_dir)
        stand_in = os.path.join(bin_dir, "systemd-run")
        with open(stand_in, "w", encoding="utf-8") as file:
            file.write(STAND_IN)
        os.chmod(stand_in, os.stat(stand_in).st_mode | stat.S_IXUSR)
        self.log = os.path.join(self.temp.name, "calls")
        self.env = dict(os.environ)
        self.env["PATH"] = bin_dir + os.pathsep + self.env.get("PATH", "")
        self.env["CAPPED_TEST_LOG"] = self.log
        self.env.pop("DAWSOME_CAPPED", None)

    def tearDown(self):
        self.temp.cleanup()

    def calls(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log, encoding="utf-8") as file:
            return file.read().splitlines()

    def run_capped(self):
        return subprocess.run(
            [SCRIPT, "--", "echo", "ran"],
            env=self.env,
            capture_output=True,
            text=True,
            check=True,
        )

    def test_a_run_starts_one_scope_marked_capped(self):
        done = self.run_capped()
        self.assertEqual(done.stdout, "ran\n")
        calls = self.calls()
        # The probe that systemd can make a scope, then the scope itself.
        self.assertEqual(len(calls), 2, calls)
        self.assertIn("--setenv=DAWSOME_CAPPED=1", calls[1])
        self.assertIn("-p MemoryMax=24G", calls[1])

    def test_a_run_inside_a_scope_runs_in_place(self):
        self.env["DAWSOME_CAPPED"] = "1"
        done = self.run_capped()
        self.assertEqual(done.stdout, "ran\n")
        self.assertEqual(self.calls(), [], "no scope inside a scope")


if __name__ == "__main__":
    unittest.main()
