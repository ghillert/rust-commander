#!/usr/bin/env python3
"""Exercise actual AppKit Extract controls against disposable archive fixtures."""
import argparse
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import zipfile


def main():
    project = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/debug/rust-commander')
    parser.add_argument('--output', type=Path, default=project / 'target/native-extract-smoke')
    parser.add_argument('--renderer', action='append', choices=['software', 'femtovg'])
    parser.add_argument('--case', action='append', choices=['form', 'extract', 'cancel', 'close', 'conflicts', 'validation', 'scroll', 'reopen', 'skip', 'apply-all', 'return', 'quit', 'return-fast'])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    # Keep the tested executable stable if a development build runs concurrently.
    with tempfile.TemporaryDirectory(prefix='rc-native-extract-binary-') as directory:
        binary = Path(directory) / 'rust-commander'
        shutil.copy2(args.binary.resolve(), binary)
        run_cases(args, binary)


def run_cases(args, binary):
    for renderer in args.renderer or ['software', 'femtovg']:
        for case in args.case or ['form', 'extract', 'cancel', 'close', 'conflicts', 'validation', 'scroll', 'reopen', 'skip', 'apply-all', 'return', 'quit', 'return-fast']:
            with tempfile.TemporaryDirectory(prefix='rc-native-extract-') as directory:
                root = Path(directory)
                config, left, right = [root / name for name in ['config', 'left', 'right']]
                for path in [config, left, right]:
                    path.mkdir()
                (config / 'settings.json').write_text(json.dumps({'preferences': {
                    'color_scheme': 'Dark' if renderer == 'femtovg' else 'Light',
                }}))
                archive = left / 'source.zip'
                names = ['report.txt'] if case != 'scroll' else [f'report-{i:02d}.txt' for i in range(30)]
                destination = right / 'résultats with spaces'
                if case in ['conflicts', 'scroll', 'skip', 'apply-all']:
                    destination.mkdir()
                    for name in names:
                        (destination / name).write_text('Original contents\n')
                with zipfile.ZipFile(archive, 'w') as output:
                    for name in names:
                        output.writestr(name, 'Extracted café contents\n')
                if case == 'validation':
                    # A file used as a directory produces an inline preflight error.
                    destination.write_text('Must remain untouched\n')
                sequence = [[300, 'open', str(archive)], [900, 'action', 'Extract'],
                            [1300, 'native-field', json.dumps(['prompt', str(destination)])],
                            [1900, 'native-check', '']]
                if case == 'return-fast':
                    sequence = sequence[:2] + [[1300, 'native-field-return', json.dumps(['prompt', str(destination)])]]
                if case in ['extract', 'conflicts', 'skip', 'apply-all', 'return']:
                    if case == 'conflicts':
                        sequence += [[2100, 'native-field', json.dumps(['policy', 'Skip'])],
                                     [2300, 'native-field', json.dumps(['merge', 'true'])],
                                     [2500, 'native-field', json.dumps(['conflict-0', 'Keep both'])]]
                    if case in ['skip', 'apply-all']:
                        sequence += [[2100, 'native-field', json.dumps(['policy', 'Skip'])]]
                    if case == 'apply-all':
                        sequence += [[2300, 'native-field', json.dumps(['conflict-0', 'Keep both'])],
                                     [2500, 'native-action', 'apply-all']]
                    sequence += [[2800, 'native-action', 'return' if case == 'return' else 'accept']]
                elif case == 'quit':
                    sequence += [[2200, 'action', 'Quit']]
                elif case in ['cancel', 'close']:
                    sequence += [[2200, 'native-action', case]]
                elif case == 'validation':
                    sequence += [[2200, 'native-check', 'Not a directory']]
                elif case == 'reopen':
                    sequence += [[2100, 'native-action', 'cancel'], [2400, 'action', 'Extract'],
                                 [3000, 'native-check', '']]
                target = args.output.resolve() / f'{renderer}-{case}.png'
                target.unlink(missing_ok=True)
                env = dict(os.environ, RUST_COMMANDER_CONFIG_DIR=str(config),
                           SLINT_BACKEND='winit-' + renderer,
                           RUST_COMMANDER_SCREENSHOT_TO=str(target),
                           RUST_COMMANDER_SCREENSHOT_SEQUENCE=json.dumps(sequence),
                           RUST_COMMANDER_SCREENSHOT_DELAY_MS='4200')
                env.pop('RUST_COMMANDER_SMOKE_KEEP_OPEN', None)
                result = subprocess.run([str(binary), str(left), str(right)], env=env, capture_output=True, text=True, timeout=60)
                (args.output / f'{renderer}-{case}.log').write_text(result.stdout + result.stderr)
                if result.returncode:
                    raise RuntimeError(f'{renderer}/{case}: {result.stderr}')
                if case != 'quit' and (not target.is_file() or target.stat().st_size < 1000):
                    raise RuntimeError(f'Missing snapshot: {target}')
                if case == 'return-fast':
                    sequence = sequence[:2] + [[1300, 'native-field-return', json.dumps(['prompt', str(destination)])]]
                if case in ['extract', 'return', 'return-fast']:
                    assert (destination / 'report.txt').read_text() == 'Extracted café contents\n'
                elif case == 'conflicts':
                    assert (destination / 'report.txt').read_text() == 'Original contents\n'
                    assert len(list(destination.glob('report*'))) == 2, list(destination.iterdir())
                elif case in ['skip', 'apply-all']:
                    assert (destination / 'report.txt').read_text() == 'Original contents\n'
                    assert len(list(destination.iterdir())) == 1
                elif case in ['cancel', 'close', 'reopen', 'form', 'quit']:
                    assert not destination.exists(), 'Cancellation/form preview must not extract files'
                elif case == 'validation':
                    assert destination.read_text() == 'Must remain untouched\n'
                print(f'{renderer}/{case}: passed', flush=True)


if __name__ == '__main__':
    main()
