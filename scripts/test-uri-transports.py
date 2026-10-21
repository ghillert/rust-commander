#!/usr/bin/env python3
"""Run real URI transport tests on any OS using disposable loopback servers.

Install scripts/uri-fixtures/requirements.txt into a virtual environment first.
Never reads or modifies user credentials, SSH files, or application settings.
"""
from datetime import datetime, timedelta, timezone
import ipaddress
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

PROJECT = Path(__file__).resolve().parent.parent


def main():
    import paramiko
    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from cryptography.x509.oid import NameOID
    logs = PROJECT / "target/uri-transport-logs"
    logs.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rust-commander-uri-") as directory:
        work = Path(directory)
        host_key = paramiko.RSAKey.generate(2048)
        host_key.write_private_key_file(str(work / "host_key"))
        client_key = paramiko.RSAKey.generate(2048)
        client_key.write_private_key_file(str(work / "client_key"))
        key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Rust Commander disposable fixture")])
        now = datetime.now(timezone.utc)
        ca_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        ca = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject)
              .public_key(ca_key.public_key()).serial_number(x509.random_serial_number())
              .not_valid_before(now - timedelta(minutes=5)).not_valid_after(now + timedelta(days=1))
              .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
              .sign(ca_key, hashes.SHA256()))
        cert = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject)
                .public_key(key.public_key()).serial_number(x509.random_serial_number())
                .not_valid_before(now - timedelta(minutes=5)).not_valid_after(now + timedelta(days=1))
                .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
                .add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address("127.0.0.1"))]), critical=False)
                .sign(ca_key, hashes.SHA256()))
        (work / "cert.pem").write_bytes(cert.public_bytes(serialization.Encoding.PEM) + ca.public_bytes(serialization.Encoding.PEM))
        (work / "cert.der").write_bytes(ca.public_bytes(serialization.Encoding.DER))
        (work / "key.pem").write_bytes(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.TraditionalOpenSSL, serialization.NoEncryption()))
        children, handles, profiles = [], [], {}
        try:
            for service in ["ftp", "explicit", "implicit", "sftp"]:
                root = work / service
                root.mkdir()
                (root / "seed.txt").write_text("fixture")
                with socket.socket() as listener:
                    listener.bind(("127.0.0.1", 0))
                    port = listener.getsockname()[1]
                log = (logs / (service + ".log")).open("w")
                handles.append(log)
                children.append(subprocess.Popen([sys.executable, "-u", str(PROJECT / "scripts/uri-fixtures/server.py"), service,
                    "--port", str(port), "--root", str(root), "--cert", str(work / "cert.pem"),
                    "--key", str(work / ("host_key" if service == "sftp" else "key.pem")), "--client-key", str(work / "client_key")], stdout=log, stderr=subprocess.STDOUT))
                deadline = time.monotonic() + 20
                while True:
                    if children[-1].poll() is not None:
                        raise RuntimeError(f"{service} fixture exited; see {logs}")
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                            break
                    except OSError:
                        if time.monotonic() > deadline:
                            raise RuntimeError(f"{service} fixture did not start")
                        time.sleep(0.05)
                name = "native_sftp" if service == "sftp" else "ftps_" + service if service != "ftp" else "ftp"
                known = work / "known_hosts"
                if service == "sftp":
                    known.write_text(f"[127.0.0.1]:{port} {host_key.get_name()} {host_key.get_base64()}\n")
                profiles[name] = dict(id="uri-fixture-" + name, name=name, service="sftp" if service == "sftp" else "ftp",
                    endpoint=f"{'ssh' if service == 'sftp' else 'ftp' if service == 'ftp' else 'ftps'}://127.0.0.1:{port}",
                    root="/", username="commander", ftp_tls={"ftp": "Plain", "explicit": "Explicit", "implicit": "Implicit", "sftp": "Automatic"}[service],
                    native_sftp=True, known_hosts_file=str(known), tls_ca_file=str(work / "cert.der"), listed=False)
            profiles["anonymous_ftp"] = dict(profiles["ftp"], id="uri-fixture-anonymous", username="")
            profiles["native_sftp_key"] = dict(profiles["native_sftp"], id="uri-fixture-key", key_file=str(work / "client_key"))
            recipes = work / "profiles.json"
            recipes.write_text(json.dumps(profiles))
            env = dict(os.environ, RUST_COMMANDER_TEST_SERVICES=str(recipes))
            cargo = [str(PROJECT / "scripts/cargo-local.sh")] if os.name != "nt" else ["cargo"]
            cargo += ["test", "--locked", "--all-features", "--test", "remote_services", "uri_", "--", "--ignored", "--test-threads=1", "--nocapture"]
            result = subprocess.run(cargo, cwd=PROJECT, env=env, capture_output=True, text=True, timeout=900)
            (logs / "tests.log").write_text(result.stdout + result.stderr)
            print(result.stdout + result.stderr, end="")
            if result.returncode:
                raise RuntimeError("URI transport integration tests failed")
            for service in ["ftp", "explicit", "implicit", "sftp"]:
                if (work / service / "must-not-be-created").exists():
                    raise RuntimeError("Connecting created a missing remote root")
        finally:
            for child in children:
                child.terminate()
            for child in children:
                child.wait(timeout=10)
            for handle in handles:
                handle.close()


if __name__ == "__main__":
    main()
