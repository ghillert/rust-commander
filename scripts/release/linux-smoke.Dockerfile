FROM ubuntu:24.04 AS base
# A runtime-only image: no compiler, development headers, Cargo, or build cache.
COPY dist/release/ /app/dist/release/
COPY scripts/release/ /app/scripts/release/
RUN apt-get update && apt-get install -y --no-install-recommends \
      python3 xvfb xauth dbus-x11 libgl1-mesa-dri libegl-mesa0 libglx-mesa0 fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
ENV LIBGL_ALWAYS_SOFTWARE=1
FROM base AS appimage
CMD ["xvfb-run", "-a", "dbus-run-session", "--", "python3", "scripts/release/smoke.py", "--target", "linux-x64", "--linux-package", "appimage"]

FROM base AS deb
RUN apt-get update && apt-get install -y --no-install-recommends /app/dist/release/*.deb \
    && rm -rf /var/lib/apt/lists/*
CMD ["xvfb-run", "-a", "dbus-run-session", "--", "python3", "scripts/release/smoke.py", "--target", "linux-x64", "--linux-package", "deb"]
