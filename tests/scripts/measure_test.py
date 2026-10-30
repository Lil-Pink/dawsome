"""Tests of scripts/measure.py collect and report against the shape each scenario's window
makes undisturbed (2 updates a second idle, 32 playing): a run more than three times above or
below it is marked in the results and the report, and collect exits 3 so scripts/measure.sh
runs it again; a run again in shape replaces it, unmarked.

The runs here are made up: a sample and the --stats lines of the app, in a temporary folder.

Run (from the repository root; scripts/check.sh runs it):
  python3 -I -B tests/scripts/measure_test.py
"""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPT = os.path.join(ROOT, "scripts", "measure.py")


def load_measure():
    """scripts/measure.py as a module, read from its path (no folder of it on sys.path)."""
    spec = importlib.util.spec_from_file_location("measure", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


measure = load_measure()

SAMPLE = {"alive": True, "threads": 20,
          "cpu_pct": {"ui": 1.0, "audio": 0.5, "other": 0.5, "total": 2.0},
          "switches_per_s": {"ui": 10.0, "audio": 100.0, "other": 10.0, "total": 120.0},
          "rss_mib": 200.0, "rss_anon_mib": 80.0, "gpu_sm_pct": 0.0, "gpu_samples": 20,
          "vram_mib": 50, "by_thread": {}}


def stats(updates_per_s, seconds=26.0):
    """The window line of the app's --stats output for `updates_per_s` over `seconds`."""
    updates = round(updates_per_s * seconds)
    return (f"window: {updates} updates ({updates_per_s:.1f}/s), {updates} frames drawn "
            f"({updates_per_s:.1f}/s) in {seconds:.1f} s\n")


class Collect(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="dawsome-measure-")
        self.addCleanup(temp.cleanup)
        self.dir = temp.name
        self.results = os.path.join(self.dir, "results.json")
        with open(os.path.join(self.dir, "sample.json"), "w", encoding="utf-8") as file:
            json.dump(SAMPLE, file)

    def collect(self, scenario, output):
        """The exit status of `measure.py collect` of one run whose app printed `output`."""
        with open(os.path.join(self.dir, "app.out"), "w", encoding="utf-8") as file:
            file.write(output)
        done = subprocess.run(
            [sys.executable, "-I", SCRIPT, "collect", self.results, scenario, f"({scenario})",
             os.path.join(self.dir, "sample.json"), os.path.join(self.dir, "app.out")],
            capture_output=True, text=True, check=False)
        return done.returncode

    def entry(self, scenario):
        with open(self.results, encoding="utf-8") as file:
            return next(entry for entry in json.load(file) if entry["scenario"] == scenario)

    # Known answers by hand: idle makes 2 a second, playing 32; off means more than 3x either
    # way (idle above 6 or below 2/3; playing above 96 or below 10.67), exactly 3x is not.
    def test_a_run_more_than_three_times_off_its_scenario_is_marked(self):
        cases = [
            ("a_empty_idle", 2.0, False),
            ("b_f1_idle", 31.9, True),
            ("b_f1_idle", 6.0, False),
            ("b_f1_idle", 6.1, True),
            ("b_f1_idle", 0.6, True),
            ("c_f1_playing", 32.0, False),
            ("c_f1_playing", 2.0, True),
            ("c_f1_playing", 11.0, False),
            ("c_f1_playing", 10.0, True),
        ]
        for scenario, rate, off in cases:
            with self.subTest(scenario=scenario, rate=rate):
                self.assertEqual(self.collect(scenario, stats(rate)), 3 if off else 0)
                entry = self.entry(scenario)
                self.assertIs(entry["off_shape"], off)
                self.assertEqual(entry["expected_updates_per_s"],
                                 32.0 if "playing" in scenario else 2.0)

    # A run whose app printed no window line is off its shape too.
    def test_a_run_without_its_window_line_is_marked(self):
        self.assertEqual(self.collect("a_empty_idle", "audio callbacks: no stream\n"), 3)
        self.assertIs(self.entry("a_empty_idle")["off_shape"], True)

    # The report marks a row off its shape with its rate and the scenario's; run again in
    # shape, the scenario's entry is replaced and its row is plain.
    def test_the_report_marks_a_run_off_its_shape_until_it_is_run_again(self):
        self.assertEqual(self.collect("b_f1_idle", stats(31.9)), 3)
        self.assertEqual(self.collect("c_f1_playing", stats(32.0)), 0)
        with open(self.results, encoding="utf-8") as file:
            rows = measure.report(json.load(file)).splitlines()
        idle = next(row for row in rows if row.startswith("| (b_f1_idle)"))
        playing = next(row for row in rows if row.startswith("| (c_f1_playing)"))
        self.assertIn("off: 31.9 updates/s, 2 expected", idle)
        self.assertNotIn("off:", playing)
        self.assertEqual(self.collect("b_f1_idle", stats(2.0)), 0)
        with open(self.results, encoding="utf-8") as file:
            results = json.load(file)
        self.assertEqual([entry["scenario"] for entry in results], ["c_f1_playing", "b_f1_idle"])
        self.assertNotIn("off:", measure.report(results))


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    unittest.main(verbosity=1)
