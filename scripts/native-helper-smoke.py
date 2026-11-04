#!/usr/bin/env python3
"""Check isolated native form controls and owner/pipe shutdown (Windows or X11)."""
import argparse
import ctypes as c
import ctypes.util
from ctypes import wintypes
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time


class Owner:
    def __init__(self):
        if os.name == 'nt':
            self.lib = c.WinDLL('user32', use_last_error=True)
            self.lib.CreateWindowExW.restype = c.c_void_p
            self.lib.CreateWindowExW.argtypes = [c.c_ulong, c.c_wchar_p, c.c_wchar_p,
                c.c_ulong, c.c_int, c.c_int, c.c_int, c.c_int,
                c.c_void_p, c.c_void_p, c.c_void_p, c.c_void_p]
            self.window = self.lib.CreateWindowExW(0, 'STATIC', 'Native control fixture',
                0x10cf0000, 20, 20, 900, 700, None, None, None, None)
            self.lib.DestroyWindow.argtypes = [c.c_void_p]
            self.lib.PeekMessageW.argtypes = [c.POINTER(wintypes.MSG), c.c_void_p,
                                             c.c_uint, c.c_uint, c.c_uint]
            self.lib.TranslateMessage.argtypes = [c.POINTER(wintypes.MSG)]
            self.lib.DispatchMessageW.argtypes = [c.POINTER(wintypes.MSG)]
            self.lib.DispatchMessageW.restype = c.c_ssize_t
            self.parent = {'Windows': self.window}
        else:
            self.lib = c.CDLL(ctypes.util.find_library('X11'))
            self.lib.XOpenDisplay.restype = c.c_void_p
            self.display = self.lib.XOpenDisplay(None)
            if not self.display:
                raise RuntimeError('Requires an X11 display (use xvfb-run)')
            self.lib.XDefaultRootWindow.argtypes = [c.c_void_p]
            self.lib.XDefaultRootWindow.restype = c.c_ulong
            self.lib.XCreateSimpleWindow.argtypes = [c.c_void_p, c.c_ulong, c.c_int,
                c.c_int, c.c_uint, c.c_uint, c.c_uint, c.c_ulong, c.c_ulong]
            self.lib.XCreateSimpleWindow.restype = c.c_ulong
            self.window = self.lib.XCreateSimpleWindow(self.display,
                self.lib.XDefaultRootWindow(self.display), 20, 20, 900, 700, 0, 0, 0)
            for name in ['XMapWindow', 'XDestroyWindow']:
                getattr(self.lib, name).argtypes = [c.c_void_p, c.c_ulong]
            self.lib.XFlush.argtypes = [c.c_void_p]
            self.lib.XCloseDisplay.argtypes = [c.c_void_p]
            self.lib.XMapWindow(self.display, self.window)
            self.lib.XFlush(self.display)
            self.parent = {'X11': self.window}
        if not self.window:
            raise RuntimeError('Create fixture owner')

    def pump(self):
        if os.name == 'nt':
            # Owned Win32 windows can synchronously message this thread while
            # the helper creates controls, changes focus or opens a chooser.
            message = wintypes.MSG()
            while self.lib.PeekMessageW(c.byref(message), None, 0, 0, 1):
                self.lib.TranslateMessage(c.byref(message))
                self.lib.DispatchMessageW(c.byref(message))

    def close(self):
        if os.name == 'nt':
            self.lib.DestroyWindow(self.window)
        else:
            self.lib.XDestroyWindow(self.display, self.window)
            self.lib.XCloseDisplay(self.display)


def field(id, title, value, kind=0, choices=None, page=0, picker=None):
    return dict(id=id, title=title, value=value, kind=kind, choices=choices or [],
                page=page, picker=picker)


def main():
    project = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=project / 'target/debug' /
                        ('rust-commander.exe' if os.name == 'nt' else 'rust-commander'))
    parser.add_argument('--logs', type=Path, default=project / 'target/native-helper-smoke')
    args = parser.parse_args()
    args.logs.mkdir(parents=True, exist_ok=True)
    owner = Owner()
    try:
        with tempfile.TemporaryDirectory(prefix='rc-native-helper-') as directory:
            root = Path(directory)
            # EOF can arrive while GTK is still processing window/chooser setup.
            for index, case in enumerate(['controls'] + ['eof', 'chooser-eof'] * 10):
                run_case(args.binary.resolve(), root, owner, case,
                         args.logs / f'{index:02}-{case}.log')
            assert not (root / 'config').exists(), 'The helper must never load app configuration'
    finally:
        owner.close()


def run_case(binary, root, owner, case, log_path):
    events = queue.Queue()
    env = dict(os.environ, RUST_COMMANDER_CONFIG_DIR=str(root / 'config'))
    # The parent surface in this fixture is X11 even on machines offering Wayland.
    if os.name != 'nt':
        env['GDK_BACKEND'] = 'x11'
    with log_path.open('w', encoding="utf-8") as log:
        process = subprocess.Popen([str(binary), '--native-forms-helper'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, env=env, encoding="utf-8")
        def read():
            for line in process.stdout:
                try:
                    events.put(json.loads(line))
                except json.JSONDecodeError:
                    events.put(RuntimeError(f'{case}: invalid helper output: {line[:200]!r}'))
                    return
        reader = threading.Thread(target=read, daemon=True)
        reader.start()
        def next_event(timeout):
            deadline = time.monotonic() + timeout
            while True:
                owner.pump()
                try:
                    value = events.get(timeout=min(.05, max(0, deadline - time.monotonic())))
                    break
                except queue.Empty:
                    if time.monotonic() >= deadline:
                        raise
            if isinstance(value, Exception):
                raise value
            return value
        def wait_exit(timeout):
            deadline = time.monotonic() + timeout
            while process.poll() is None:
                owner.pump()
                if time.monotonic() >= deadline:
                    raise subprocess.TimeoutExpired(process.args, timeout)
                time.sleep(.02)
            return process.returncode
        def send(value):
            process.stdin.write(json.dumps(value) + '\n')
            process.stdin.flush()
        def wait(expected, content=None):
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                try:
                    value = next_event(.2)
                except queue.Empty:
                    if process.poll() is not None:
                        raise RuntimeError(f'{case}: native helper exited {process.returncode}')
                    continue
                if value == expected or isinstance(value, dict) and expected in value and (content is None or value[expected] == content):
                    return value
            raise RuntimeError(f'{case}: missing {expected}')
        form = dict(kind='preferences' if case == 'controls' else 'prompt',
            title='Native form fixture', description='', error='', error_page=None,
            busy=False, default_action=True, rows=[],
            buttons=[['cancel', 'Cancel', True], ['apply', 'Apply', True], ['accept', 'OK', True]],
            fields=[field('prompt', 'Destination', str(root), picker='Directory'),
                    field('enabled', 'Enabled', 'true', 2),
                    field('choice', 'Format', 'Zip', 3, ['Zip', '7z']),
                    field('secret', 'Password', 'fixture', 1),
                    field('font', 'Font size', '14', page=2)])
        try:
            send({'Update': dict(form=form, parent=owner.parent, scheme='System', language='English')})
            wait('Ready')
            if case == 'controls':
                send({'Page': 2}); assert wait('Checked')['Checked']
                send({'Smoke': ['native-field', json.dumps(['font', '18'])]})
                assert wait('Field', ['font', '18'])['Field'] == ['font', '18']
                assert wait('Checked')['Checked']
                send({'Page': 0}); assert wait('Checked')['Checked']
                unicode_path = str(root / 'café 雪 Н')
                send({'Smoke': ['native-field', json.dumps(['prompt', unicode_path])]})
                assert wait('Field', ['prompt', unicode_path])['Field'] == ['prompt', unicode_path]
                assert wait('Checked')['Checked']
                send({'Smoke': ['native-action', 'apply']})
                # GTK invokes synchronously; Win32 posts the real BM_CLICK.
                seen = set()
                while len(seen) < 2:
                    value = next_event(12)
                    if value == {'Checked': True}: seen.add('checked')
                    if value == {'Action': 'apply'}: seen.add('action')
                send('Close')
            elif case == 'chooser-eof':
                send({'Smoke': ['native-action', 'browse:prompt']})
                # Editing is emitted immediately before opening the system chooser.
                wait('Editing')
                time.sleep(.5)
                process.stdin.close()
            else:
                process.stdin.close()
            code = wait_exit(8)
            assert code == 0, f'{case}: helper exited {code}; stderr: {log_path}'
            reader.join(timeout=2)
            assert not reader.is_alive(), f'{case}: helper output pipe did not close'
            while not events.empty():
                next_event(0)
            print(f'{case}: native controls and cleanup passed', flush=True)
        finally:
            if process.poll() is None:
                process.kill(); process.wait()
            reader.join(timeout=2)
            process.stdout.close()
            if not process.stdin.closed:
                process.stdin.close()


if __name__ == '__main__':
    main()
