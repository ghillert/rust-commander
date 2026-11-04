#!/usr/bin/env python3
"""Exercise smoke-process failure diagnostics and descendant cleanup."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from smoke_process import run_smoke


class SmokeProcessTests(unittest.TestCase):
    def test_utf8_output_is_saved_on_success_and_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'output.log'
            for status in [0, 7]:
                result = run_smoke([sys.executable, '-u', '-c',
                    'import sys; print("café 雪"); print("failure details", file=sys.stderr); sys.exit(int(sys.argv[1]))',
                    str(status)], env=dict(os.environ, PYTHONIOENCODING='utf-8'),
                    log_path=log, timeout=10)
                self.assertEqual(result.returncode, status)
                self.assertIn('café 雪', result.stderr)
                self.assertIn('failure details', result.stderr)
                self.assertEqual(log.read_text(encoding='utf-8'), result.stderr)

    def test_timeout_keeps_output_and_stops_descendants(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log, marker, locked = [root / name for name in ['output.log', 'ready', 'locked']]
            locked.write_text('preserve contents', encoding='utf-8')
            child = '''import os, pathlib, sys, time
handle = open(sys.argv[2], encoding='utf-8')
pathlib.Path(sys.argv[1]).write_text(str(os.getpid()), encoding='utf-8')
while True:
    print('child heartbeat', flush=True)
    time.sleep(.02)
'''
            parent = '''import pathlib, subprocess, sys, time
subprocess.Popen([sys.executable, '-u', '-c', sys.argv[1], sys.argv[2], sys.argv[3]])
while not pathlib.Path(sys.argv[2]).exists():
    time.sleep(.01)
print('parent ready', flush=True)
time.sleep(60)
'''
            try:
                with self.assertRaisesRegex(RuntimeError, 'parent ready'):
                    run_smoke([sys.executable, '-u', '-c', parent, child, str(marker), str(locked)],
                              env=os.environ.copy(), log_path=log, timeout=3)
                self.assertTrue(marker.exists(), 'The descendant must have started')
                size = log.stat().st_size
                time.sleep(.2)
                self.assertEqual(log.stat().st_size, size, 'The descendant must stop writing')
                # On Windows an open child handle would prevent this deletion,
                # just like the orphan helper retaining the copied executable.
                locked.unlink()
            finally:
                if os.name == 'nt' and marker.exists() and locked.exists():
                    subprocess.run(['taskkill', '/PID', marker.read_text(encoding='utf-8'), '/T', '/F'],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)


if __name__ == '__main__':
    unittest.main()
