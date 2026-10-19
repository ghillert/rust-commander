# Disposable protocol servers; no host directories are mounted.
FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    openssh-server samba python3 python3-venv python3-pyftpdlib python3-cheroot \
    && python3 -m venv --system-site-packages /opt/dav \
    && /opt/dav/bin/pip install --no-cache-dir WsgiDAV==4.3.3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --shell /bin/sh commander \
    && echo 'commander:commander-test-only' | chpasswd \
    && mkdir -p /run/sshd /fixture /srv/test
COPY servers.py /fixture/servers.py
ENTRYPOINT ["/opt/dav/bin/python", "-u", "/fixture/servers.py"]
