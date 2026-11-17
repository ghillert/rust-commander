#!/usr/bin/env python3
"""Exercise real AppKit quit-sheet keyboard events in a disposable app session."""

import argparse
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

from smoke_process import run_smoke


def main():
    project = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/debug/rust-commander')
    parser.add_argument('--output', type=Path, default=project / 'target/modal-keyboard-smoke')
    parser.add_argument('--case', choices=[
        'return', 'escape', 'command-period', 'checkbox', 'tab',
        'tab-space-cancel', 'tab-space-quit',
    ])
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('native AppKit keyboard smoke requires macOS')
    args.output.mkdir(parents=True, exist_ok=True)

    cases = {
        'return': [[1200, 'native-key', 'Return']],
        'escape': [[1200, 'native-key', 'Escape'], [1600, 'native-check-closed', '']],
        'command-period': [[1200, 'native-key', 'CommandPeriod'], [1600, 'native-check-closed', '']],
        'checkbox': [
            [1200, 'native-key', 'Space'],
            [1500, 'native-check-field', json.dumps(['quit-show-next-time', 'false'])],
            [1800, 'native-key', 'Escape'],
            [2100, 'native-check-closed', ''],
        ],
        'tab': [
            [1200, 'native-key', 'Tab'],
            [1500, 'native-check-focus', 'cancel'],
            [1800, 'native-key', 'Tab'],
            [2100, 'native-check-focus', 'confirm-quit'],
            [2400, 'native-key', 'ShiftTab'],
            [2700, 'native-check-focus', 'cancel'],
            [3000, 'native-key', 'ShiftTab'],
            [3300, 'native-check', ''],
            [3600, 'native-key', 'Escape'],
            [3900, 'native-check-closed', ''],
        ],
        'tab-space-cancel': [
            [1200, 'native-key', 'Tab'],
            [1500, 'native-check-focus', 'cancel'],
            [1800, 'native-key', 'Space'],
            [2200, 'native-check-closed', ''],
        ],
        'tab-space-quit': [
            [1200, 'native-key', 'Tab'],
            [1500, 'native-check-focus', 'cancel'],
            [1800, 'native-key', 'Tab'],
            [2100, 'native-check-focus', 'confirm-quit'],
            [2400, 'native-key', 'Space'],
        ],
    }
    with tempfile.TemporaryDirectory(prefix='rc-modal-keyboard-binary-') as binary_dir:
        binary = Path(binary_dir) / 'rust-commander'
        shutil.copy2(args.binary.resolve(), binary)
        for name, suffix in cases.items():
            if args.case and name != args.case:
                continue
            with tempfile.TemporaryDirectory(prefix='rc-modal-keyboard-') as directory:
                root = Path(directory)
                config, left, right = (root / part for part in ('config', 'left', 'right'))
                for path in (config, left, right):
                    path.mkdir()
                (config / 'settings.json').write_text(
                    json.dumps({'preferences': {'confirm_quit': True}}), encoding='utf-8'
                )
                sequence = [[500, 'action', 'Quit'], [900, 'native-check', ''], *suffix]
                snapshot = args.output.resolve() / f'{name}.png'
                snapshot.unlink(missing_ok=True)
                env = dict(
                    os.environ,
                    RUST_COMMANDER_CONFIG_DIR=str(config),
                    SLINT_BACKEND='winit-software',
                    RUST_COMMANDER_SCREENSHOT_TO=str(snapshot),
                    RUST_COMMANDER_SCREENSHOT_SEQUENCE=json.dumps(sequence),
                    RUST_COMMANDER_SCREENSHOT_DELAY_MS=(
                        '4300' if name == 'tab' else '3000' if name == 'tab-space-quit' else '2500'
                    ),
                )
                env.pop('RUST_COMMANDER_SMOKE_KEEP_OPEN', None)
                result = run_smoke(
                    [str(binary), str(left), str(right)],
                    env=env,
                    log_path=args.output / f'{name}.log',
                    timeout=30,
                )
                if result.returncode:
                    raise RuntimeError(f'{name}: {result.stderr}')
                quits = name in ('return', 'tab-space-quit')
                if quits and snapshot.exists():
                    raise RuntimeError(f'{name}: Quit did not finish before the screenshot timer')
                if not quits and (not snapshot.is_file() or snapshot.stat().st_size < 1000):
                    raise RuntimeError(f'{name}: missing screenshot')


if __name__ == '__main__':
    main()
