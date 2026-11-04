"""Keep native smoke-test diagnostics even if the app or a helper gets stuck."""
import os
from pathlib import Path
import signal
import subprocess


def run_smoke(command, *, env, log_path, timeout):
    log_path = Path(log_path)
    with log_path.open('w', encoding='utf-8') as log:
        # A helper can inherit handles. Logging directly to a file avoids waiting
        # for pipe EOF from a descendant after the main process has failed.
        with subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                              start_new_session=os.name != 'nt') as process:
            try:
                process.wait(timeout=timeout)
            except subprocess.TimeoutExpired as error:
                if os.name == 'nt':
                    # Kill the tree while its root is alive; killing only the
                    # app can leave a helper holding the copied executable open.
                    try:
                        subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                       timeout=10, check=False)
                    except (OSError, subprocess.TimeoutExpired):
                        pass  # Still terminate/reap the root if taskkill fails.
                else:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=10)
                output = log_path.read_text(encoding='utf-8', errors='replace')
                raise RuntimeError(f'Smoke test timed out after {timeout}s; log: {log_path}\n{output}') from error
    output = log_path.read_text(encoding='utf-8', errors='replace')
    return subprocess.CompletedProcess(command, process.returncode, '', output)
