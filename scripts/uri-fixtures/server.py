"""Disposable FTP, implicit/explicit FTPS and SFTP servers confined to a test root."""
import argparse
import errno
import os
from pathlib import Path
import socket
import threading
import time

PASSWORD = "commander-test-only"


def ftp(args):
    from pyftpdlib.authorizers import DummyAuthorizer
    from pyftpdlib.handlers import FTPHandler, TLS_FTPHandler
    from pyftpdlib.servers import ThreadedFTPServer
    authorizer = DummyAuthorizer()
    authorizer.add_user("commander", PASSWORD, str(args.root), perm="elradfmwMT")
    authorizer.add_anonymous(str(args.root), perm="elr")
    base = FTPHandler if args.service == "ftp" else TLS_FTPHandler

    class Handler(base):
        def handle(self):
            if args.service == "implicit":
                self.secure_connection(self.ssl_context)
            else:
                super().handle()

        def handle_ssl_established(self):
            super().handle_ssl_established()
            if args.service == "implicit":
                # Send the greeting after TLS is ready on the connection's loop.
                super().handle()

    Handler.authorizer = authorizer
    Handler.banner = "Disposable Rust Commander URI fixture"
    if args.service != "ftp":
        Handler.certfile = str(args.cert)
        Handler.keyfile = str(args.key)
        Handler.tls_control_required = True
        Handler.tls_data_required = True
    # Isolate each connection's OpenSSL state. The single-loop Windows fixture
    # can fail an upload with a "system library" error as a concurrent RETR closes.
    ThreadedFTPServer(("127.0.0.1", args.port), Handler).serve_forever()


def sftp(args):
    import paramiko
    host_key = paramiko.RSAKey.from_private_key_file(str(args.key))
    client_key = paramiko.RSAKey.from_private_key_file(str(args.client_key))

    class Auth(paramiko.ServerInterface):
        def check_auth_password(self, username, password):
            return paramiko.AUTH_SUCCESSFUL if username == "commander" and password == PASSWORD else paramiko.AUTH_FAILED

        def check_auth_publickey(self, username, key):
            return paramiko.AUTH_SUCCESSFUL if username == "commander" and key == client_key else paramiko.AUTH_FAILED

        def get_allowed_auths(self, username):
            return "publickey,password"

        def check_channel_request(self, kind, channel_id):
            return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    class Files(paramiko.SFTPServerInterface):
        def local(self, path):
            result = (args.root / path.lstrip("/")).resolve()
            if not result.is_relative_to(args.root.resolve()):
                raise OSError(errno.EACCES, "Outside fixture root")
            return result

        def stat(self, path):
            try:
                return paramiko.SFTPAttributes.from_stat(self.local(path).stat())
            except OSError as error:
                return paramiko.SFTPServer.convert_errno(error.errno)

        lstat = stat

        def list_folder(self, path):
            try:
                result = []
                for child in self.local(path).iterdir():
                    attr = paramiko.SFTPAttributes.from_stat(child.stat())
                    attr.filename = child.name
                    result.append(attr)
                return result
            except OSError as error:
                return paramiko.SFTPServer.convert_errno(error.errno)

        def open(self, path, flags, attr):
            try:
                fd = os.open(self.local(path), flags | getattr(os, "O_BINARY", 0), 0o600)
                mode = "r+b" if flags & os.O_RDWR else "wb" if flags & os.O_WRONLY else "rb"
                file = os.fdopen(fd, mode)
                handle = paramiko.SFTPHandle(flags)
                handle.readfile = file
                handle.writefile = file
                return handle
            except OSError as error:
                return paramiko.SFTPServer.convert_errno(error.errno)

        def operation(self, callback, *paths):
            try:
                callback(*(self.local(path) for path in paths))
                return paramiko.SFTP_OK
            except OSError as error:
                return paramiko.SFTPServer.convert_errno(error.errno)

        def mkdir(self, path, attr):
            return self.operation(lambda p: p.mkdir(), path)

        def remove(self, path):
            return self.operation(lambda p: p.unlink(), path)

        def rmdir(self, path):
            return self.operation(lambda p: p.rmdir(), path)

        def rename(self, old, new):
            return self.operation(lambda a, b: a.rename(b), old, new)

    def connection(client):
        transport = paramiko.Transport(client)
        transport.add_server_key(host_key)
        transport.set_subsystem_handler("sftp", paramiko.SFTPServer, Files)
        try:
            transport.start_server(server=Auth())
            while transport.is_active():
                time.sleep(0.05)
        except (EOFError, OSError, paramiko.SSHException):
            pass
        finally:
            transport.close()

    with socket.socket() as listener:
        listener.bind(("127.0.0.1", args.port))
        listener.listen()
        while True:
            client, _ = listener.accept()
            threading.Thread(target=connection, args=(client,), daemon=True).start()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("service", choices=["ftp", "explicit", "implicit", "sftp"])
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--cert", type=Path)
    parser.add_argument("--key", type=Path)
    parser.add_argument("--client-key", type=Path)
    args = parser.parse_args()
    (sftp if args.service == "sftp" else ftp)(args)
