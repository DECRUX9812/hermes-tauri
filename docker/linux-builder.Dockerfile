# Linux build environment for the Tauri shell. The workstation has the
# WebKitGTK runtime but no -dev headers and no root, so Linux builds run here.
# Same Ubuntu release as the workstation, so binaries run on the host as-is.
FROM ubuntu:26.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential pkg-config curl wget file ca-certificates git xz-utils \
      rustc cargo \
      libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev \
      libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev \
      dpkg-dev fakeroot patchelf desktop-file-utils squashfs-tools \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install tauri-cli --version '^2' --locked --root /usr/local \
    && rm -rf /root/.cargo/registry /root/.cargo/git

# linuxdeploy (AppImage) cannot mount FUSE inside a container.
ENV APPIMAGE_EXTRACT_AND_RUN=1
