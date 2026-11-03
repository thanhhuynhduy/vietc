#!/usr/bin/env python3
"""Verify test entry points without executing privileged runtime scenarios."""
import pathlib
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class TestTargetsTests(unittest.TestCase):
    def test_runtime_targets_use_the_real_integration_binary(self):
        for target in ('test-ci-smoke', 'test-runtime-targets', 'test-runtime-matrix'):
            with self.subTest(target=target):
                command = subprocess.check_output(['make', '-n', target], cwd=ROOT, text=True)
                self.assertNotIn('--test runtime ', command)
                self.assertIn('--test daemon_suite runtime::', command)


if __name__ == '__main__':
    unittest.main()
