"""Real loopback-only protocol services for the integration harness."""
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

PASSWORD = "commander-test-only"
ROOT = Path("/srv/test")


def webdav():
    from cheroot.wsgi import Server
    from wsgidav.wsgidav_app import WsgiDAVApp
    app = WsgiDAVApp({
        "provider_mapping": {"/": str(ROOT / "webdav")},
        "http_authenticator": {"accept_basic": True, "accept_digest": False,
                               "default_to_digest": False},
        "simple_dc": {"user_mapping": {"*": {"commander": {"password": PASSWORD}}}},
        "verbose": 1,
    })
    Server(("0.0.0.0", 8080), app).start()


def ftp():
    from pyftpdlib.authorizers import DummyAuthorizer
    from pyftpdlib.handlers import FTPHandler
    from pyftpdlib.servers import FTPServer
    authorizer = DummyAuthorizer()
    authorizer.add_user("commander", PASSWORD, str(ROOT / "ftp"), perm="elradfmwMT")
    FTPHandler.authorizer = authorizer
    FTPHandler.passive_ports = [int(os.environ["FTP_PASSIVE_PORT"])]
    FTPHandler.masquerade_address = "127.0.0.1"
    FTPServer(("0.0.0.0", 2121), FTPHandler).serve_forever()


def main():
    if len(sys.argv) > 1:
        {"ftp": ftp, "webdav": webdav}[sys.argv[1]]()
        return
    for service in ["sftp", "ftp", "webdav", "smb"]:
        path = ROOT / service
        path.mkdir(exist_ok=True)
        subprocess.run(["chown", "commander:commander", str(path)], check=True)
    Path("/fixture/authorized_keys").write_text(os.environ["TEST_PUBLIC_KEY"] + "\n")
    Path("/fixture/authorized_keys").chmod(0o644)
    subprocess.run(["ssh-keygen", "-A"], check=True)
    Path("/fixture/sshd_config").write_text("""Port 2222
ListenAddress 0.0.0.0
HostKey /etc/ssh/ssh_host_ed25519_key
AuthorizedKeysFile /fixture/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
AllowUsers commander
AllowAgentForwarding no
AllowTcpForwarding no
X11Forwarding no
Subsystem sftp internal-sftp
""")
    subprocess.run(["smbpasswd", "-a", "-s", "commander"],
                   input=(PASSWORD + "\n") * 2, text=True, check=True)
    Path("/fixture/smb.conf").write_text("""[global]
server role = standalone server
security = user
smb ports = 1445
server min protocol = SMB2
map to guest = Never
load printers = no
printing = bsd
disable spoolss = yes
[commander]
path = /srv/test/smb
read only = no
valid users = commander
""")
    children = [
        subprocess.Popen(["/usr/sbin/sshd", "-D", "-e", "-f", "/fixture/sshd_config"]),
        subprocess.Popen(["/usr/sbin/smbd", "--foreground", "--no-process-group",
                          "--debug-stdout", "--configfile=/fixture/smb.conf"]),
        subprocess.Popen([sys.executable, "-u", __file__, "ftp"]),
        subprocess.Popen([sys.executable, "-u", __file__, "webdav"]),
    ]
    def stop(_signum, _frame):
        for child in children:
            child.terminate()
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    while all(child.poll() is None for child in children):
        time.sleep(0.2)
    raise SystemExit("A fixture service exited unexpectedly")


if __name__ == "__main__":
    main()
