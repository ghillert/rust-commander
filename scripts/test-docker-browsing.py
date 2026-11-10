#!/usr/bin/env python3
"""Exercise the pane through the Docker CLI with disposable local containers.

Requires the cached image produced by scripts/test-remote.py. This runner
does not pull images or change the active Docker context.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import uuid

PROJECT = Path(__file__).resolve().parent.parent
IMAGE = "rust-commander-protocol-tests:bookworm"


def main():
    env = os.environ.copy()
    context = env.get("DOCKER_CONTEXT")
    host = env.get("DOCKER_HOST") if not context else None
    if not host:
        context = context or subprocess.check_output(
            ["docker", "context", "show"], text=True, timeout=10
        ).strip()
        context_env = env.copy()
        context_env.pop("DOCKER_HOST", None)
        details = subprocess.check_output(
            ["docker", "context", "inspect", "--", context], env=context_env, timeout=10
        )
        host = json.loads(details)[0]["Endpoints"]["docker"]["Host"]
    if not host.startswith(("unix:///", "npipe:////./pipe/", "npipe://./pipe/")):
        raise SystemExit("The Docker browsing fixture requires a local engine.")

    env.pop("DOCKER_CONTEXT", None)
    env["DOCKER_HOST"] = host
    docker = ["docker", "--host", host]

    def run(*args):
        return subprocess.check_output(docker + list(args), text=True, env=env, timeout=25).strip()

    run("image", "inspect", IMAGE)
    prefix = "rc-container-fixture-" + uuid.uuid4().hex[:10]
    with tempfile.TemporaryDirectory(prefix="rc-container-") as directory:
        root = Path(directory)
        mount = root / "mounted"
        mount.mkdir()
        root.chmod(0o755)
        mount.chmod(0o755)
        (mount / "mounted.txt").write_bytes(b"mounted content\n")
        source = root / "rc-docker.txt"
        source.write_bytes(b"fixture content\n")

        ids = []
        try:
            for mode in ("live", "stopped", "minimal"):
                flags = {
                    "live": ["--user", "65534:65534"],
                    "stopped": [],
                    "minimal": ["--env", "PATH=/unavailable"],
                }[mode]
                ident = run(
                    "create", "--name", f"{prefix}-{mode}", "--network", "none",
                    "--label", "io.github.ghillert.rust-commander.docker-fixture=true",
                    "--mount", f"type=bind,source={mount},target=/mnt/rc-test,readonly",
                    *flags, "--entrypoint", "/bin/sleep", IMAGE, "300",
                )
                ids.append(ident)
                run("cp", str(source), f"{ident}:/tmp/rc-docker.txt")
                run("start", ident)
                if mode == "stopped":
                    run("stop", "--time", "1", ident)

            fixture_env = dict(env, RUST_COMMANDER_DOCKER_FIXTURES=json.dumps(ids))
            if context:
                fixture_env.update(DOCKER_CONTEXT=context, DOCKER_HOST="ssh://invalid.example")
            cargo = "cargo" if os.name == "nt" else str(PROJECT / "scripts/cargo-local.sh")
            subprocess.run(
                [cargo, "test", "--lib", "docker_browses_live_and_snapshot_containers_in_either_pane",
                 "--", "--ignored", "--nocapture"],
                env=fixture_env, cwd=PROJECT, check=True, timeout=240,
            )
        finally:
            if ids:
                run("rm", "--force", *ids)


if __name__ == "__main__":
    main()
