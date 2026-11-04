#!/usr/bin/env python3
"""Exercise native Copy/Move/Rename/Pack controls with disposable files/config."""
import argparse
import json
import os
from pathlib import Path
import shutil
import tempfile
import zipfile
from smoke_process import run_smoke


def main():
    project = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / ('target/debug/rust-commander.exe' if os.name == 'nt' else 'target/debug/rust-commander'))
    parser.add_argument('--output', type=Path, default=project / 'target/native-operation-smoke')
    parser.add_argument('--renderer', action='append', choices=['software', 'femtovg'])
    parser.add_argument('--case', action='append', choices=['copy', 'move', 'rename', 'pack', 'pack-form', 'pack-encrypted', 'pack-7z', 'pack-background', 'cancel', 'conflict', 'return-fast'])
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='rc-native-operation-') as directory:
        binary = Path(directory) / ('rust-commander.exe' if os.name == 'nt' else 'rust-commander')
        shutil.copy2(args.binary.resolve(), binary)
        for renderer in args.renderer or ['software', 'femtovg']:
            for case in args.case or ['copy', 'move', 'rename', 'pack', 'pack-form', 'pack-encrypted', 'pack-7z', 'pack-background', 'cancel', 'conflict', 'return-fast']:
                run_case(args, binary, renderer, case)


def run_case(args, binary, renderer, case):
    with tempfile.TemporaryDirectory(prefix='rc-native-form-') as directory:
        root = Path(directory)
        config, left, right = [root / name for name in ['config', 'left', 'right']]
        for path in [config, left, right]:
            path.mkdir()
        (config / 'settings.json').write_text(json.dumps({'preferences': {
            'color_scheme': 'Dark' if renderer == 'femtovg' else 'Light'}}), encoding="utf-8")
        source = left / 'café source.txt'
        source.write_text('Native operation fixture\n', encoding="utf-8")
        action = {'move': 'Move', 'rename': 'Rename'}.get(case, 'Pack' if case.startswith('pack') else 'Copy')
        destination = right / ('result.zip' if case.startswith('pack') else 'renamed.txt' if case == 'rename' else source.name)
        if case == 'rename':
            destination = left / 'renamed.txt'
        if case == 'pack-7z':
            destination = destination.with_suffix('.7z')
        if case == 'conflict':
            destination.write_text('Keep original\n', encoding="utf-8")
        value = destination.name if case == 'rename' else str(destination) if case.startswith('pack') else str(right)
        sequence = [[350, 'click', str(source)], [800, 'action', action],
                    [1250, 'native-field', json.dumps(['prompt', value])],
                    [1750, 'native-check', '']]
        if case == 'return-fast':
            sequence = sequence[:2] + [[1250, 'native-field-return', json.dumps(['prompt', value])]]
        elif case.startswith('pack'):
            sequence += [[1900, 'native-field', json.dumps(['pack-files', 'true'])]]
            if case in ['pack', 'pack-form', 'pack-encrypted']:
                sequence += [[2100, 'native-field', json.dumps(['pack-comment', 'Native comment\nGrüße'])]]
            if case == 'pack-7z':
                sequence += [[2100, 'native-field', json.dumps(['pack-format', '7z'])]]
            if case == 'pack-encrypted':
                sequence += [[2300, 'native-field', json.dumps(['pack-encrypted', 'true'])],
                             [2500, 'native-field', json.dumps(['pack-password', 'smoke-fixture-password'])]]
            if case == 'pack-background':
                sequence += [[2300, 'native-field', json.dumps(['pack-background', 'true'])]]
            if case == 'pack-form':
                sequence += [[2800, 'native-check', '']]
            else:
                sequence += [[3000, 'native-action', 'accept']]
        elif case == 'conflict':
            sequence += [[2100, 'native-field', json.dumps(['policy', 'Keep both'])],
                         [2600, 'native-action', 'accept']]
        else:
            sequence += [[2400, 'native-action', 'cancel' if case == 'cancel' else 'return' if case == 'rename' else 'accept']]
        target = args.output.resolve() / f'{renderer}-{case}.png'
        target.unlink(missing_ok=True)
        env = dict(os.environ, RUST_COMMANDER_CONFIG_DIR=str(config),
                   SLINT_BACKEND='winit-' + renderer,
                   RUST_COMMANDER_SCREENSHOT_TO=str(target),
                   RUST_COMMANDER_SCREENSHOT_SEQUENCE=json.dumps(sequence),
                   RUST_COMMANDER_SCREENSHOT_DELAY_MS='4600')
        env.pop('RUST_COMMANDER_SMOKE_KEEP_OPEN', None)
        result = run_smoke([str(binary), str(left), str(right)], env=env,
                           log_path=args.output / f'{renderer}-{case}.log', timeout=60)
        if result.returncode:
            raise RuntimeError(f'{renderer}/{case}: {result.stderr}')
        assert target.is_file() and target.stat().st_size > 1000, target
        if case in ['copy', 'move', 'rename', 'return-fast']:
            assert destination.read_text(encoding="utf-8") == 'Native operation fixture\n', list(right.iterdir())
            assert source.exists() == (case not in ['move', 'rename'])
        elif case in ['pack', 'pack-encrypted', 'pack-background']:
            with zipfile.ZipFile(destination) as archive:
                assert archive.namelist() == [source.name]
                if case != 'pack-background':
                    assert archive.comment == 'Native comment\nGrüße'.encode()
                info = archive.getinfo(source.name)
                assert bool(info.flag_bits & 1) == (case == 'pack-encrypted')
                if case != 'pack-encrypted':
                    assert archive.read(source.name) == source.read_bytes()
        elif case == 'pack-7z':
            assert destination.read_bytes().startswith(b"7z\xbc\xaf\x27\x1c")
        elif case == 'conflict':
            assert destination.read_text(encoding="utf-8") == 'Keep original\n'
            assert len(list(right.iterdir())) == 2
        else:
            assert not destination.exists()
        print(f'{renderer}/{case}: passed', flush=True)


if __name__ == '__main__':
    main()
