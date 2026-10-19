#!/usr/bin/env python3
"""Build and remove four isolated real protocol servers; run ignored Rust tests.

Only public test credentials are used. Ports bind to 127.0.0.1. No user SSH,
keychain, configuration or remote account is read or modified.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import signal
import threading
import socket
import subprocess
import tempfile
import time
import uuid

PROJECT = Path(__file__).resolve().parent.parent


def run(args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def port(container, internal):
    value = subprocess.check_output(["docker", "port", container, f"{internal}/tcp"], text=True).strip()
    host, number = value.rsplit(":", 1)
    if host != "127.0.0.1":
        raise RuntimeError("Fixture port is not confined to loopback")
    return int(number)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--logs", type=Path, default=PROJECT / "target/remote-service-logs")
    parser.add_argument("--service", choices=["sftp", "ftp", "webdav", "smb"])
    options = parser.parse_args()
    options.logs.mkdir(parents=True, exist_ok=True)
    name = "rust-commander-test-" + uuid.uuid4().hex[:12]
    image = "rust-commander-protocol-tests:bookworm"
    run(["docker", "build", "--tag", image, str(PROJECT / "scripts/remote-fixtures")])
    with tempfile.TemporaryDirectory(prefix="rust-commander-protocol-") as directory:
        work = Path(directory)
        key = work / "identity"
        run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(key)])
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            passive = listener.getsockname()[1]
        args = ["docker", "run", "--detach", "--rm", "--name", name,
                "--label", "io.github.ghillert.rust-commander.fixture=true",
                "--env", "TEST_PUBLIC_KEY=" + key.with_suffix(".pub").read_text().strip(),
                "--env", f"FTP_PASSIVE_PORT={passive}"]
        for internal in [2222, 2121, 8080, 1445]:
            args += ["--publish", f"127.0.0.1::{internal}"]
        args += ["--publish", f"127.0.0.1:{passive}:{passive}", image]
        try:
            run(args, stdout=subprocess.DEVNULL)
            ports = {service: port(name, internal) for service, internal in
                     [("sftp", 2222), ("ftp", 2121), ("webdav", 8080), ("smb", 1445)]}
            deadline = time.monotonic() + 60
            for service, number in ports.items():
                while True:
                    try:
                        with socket.create_connection(("127.0.0.1", number), timeout=0.5):
                            break
                    except OSError:
                        if time.monotonic() > deadline:
                            raise RuntimeError(f"{service} did not become ready")
                        time.sleep(0.2)
            # Obtain the key from the container we just created, rather than
            # trusting an unauthenticated network keyscan or changing ~/.ssh.
            host_key = subprocess.check_output(
                ["docker", "exec", name, "cat", "/etc/ssh/ssh_host_ed25519_key.pub"], text=True)
            known = work / "known_hosts"
            known.write_text(f"[127.0.0.1]:{ports['sftp']} " + " ".join(host_key.split()[:2]) + "\n")
            config = work / "ssh_config"
            config.write_text(f'Host *\n  UserKnownHostsFile "{known}"\n'
                              '  GlobalKnownHostsFile /dev/null\n  StrictHostKeyChecking yes\n'
                              '  IdentitiesOnly yes\n  BatchMode yes\n')
            wrapper = work / "ssh"
            wrapper.write_text("#!/bin/sh\nexec /usr/bin/ssh -F " + shlex.quote(str(config)) + ' "$@"\n')
            wrapper.chmod(0o700)
            profiles = {}
            for service, number in ports.items():
                scheme = "http" if service == "webdav" else "ssh" if service == "sftp" else service
                profiles[service] = {
                    "id": name + "-" + service, "name": "Disposable " + service,
                    "service": service, "endpoint": f"{scheme}://127.0.0.1:{number}",
                    "username": "commander", "bucket": "commander" if service == "smb" else "",
                    "root": "/srv/test/sftp" if service == "sftp" else "/", "key_file": str(key),
                }
            recipes = work / "profiles.json"
            recipes.write_text(json.dumps(profiles))
            recipes.chmod(0o600)
            env = os.environ.copy()
            env["PATH"] = str(work) + os.pathsep + env.get("PATH", "")
            env["RUST_COMMANDER_TEST_SERVICES"] = str(recipes)
            cargo = [str(PROJECT / "scripts/cargo-local.sh"), "test", "--locked", "--all-features", "--test",
                     "remote_services"]
            if options.service:
                cargo.append(options.service + "_protocol_workflows")
            cargo += ["--", "--ignored", "--test-threads=1", "--nocapture"]
            print("Testing real loopback services: " + ", ".join(ports), flush=True)
            # Tee the test result without placing private SSH key material in logs.
            with (options.logs / "tests.log").open("w") as log:
                process = subprocess.Popen(cargo, cwd=PROJECT, env=env, stdout=subprocess.PIPE,
                                           stderr=subprocess.STDOUT, text=True, start_new_session=True)
                timed_out = threading.Event()
                def terminate():
                    timed_out.set()
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGTERM)
                watchdog = threading.Timer(900, terminate)
                watchdog.start()
                try:
                    for line in process.stdout:
                        print(line, end="", flush=True)
                        log.write(line)
                        log.flush()
                    if process.wait() != 0:
                        raise RuntimeError("Remote-service integration tests timed out" if timed_out.is_set()
                                           else "Remote-service integration tests failed")
                finally:
                    watchdog.cancel()
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGTERM)
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()
        finally:
            with (options.logs / "servers.log").open("w") as log:
                subprocess.run(["docker", "logs", name], stdout=log, stderr=subprocess.STDOUT)
            subprocess.run(["docker", "rm", "--force", name], stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL)


if __name__ == "__main__":
    def interrupted(_signum, _frame):
        raise SystemExit("Remote-service tests interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    main()
