# Dev container image for the ko-ls workspace.
#
# The folder holds two Rust workspaces — `distributed-intranet` (the protocol)
# and `ko-ls` (the client that builds against it by path dependency) — both
# edition 2024, so the base image must stay recent enough to compile them;
# `rust:1-bookworm` tracks the current stable release. The client also ships a
# Tauri desktop app, which is why the GUI and Node layers below exist.

FROM rust:1-bookworm

# Build dependencies for the workspace, plus the small tools the harness
# scripts and the NAT topology expect to find on PATH.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    build-essential \
    pkg-config \
    libssl-dev \
    iproute2 \
    iputils-ping \
    jq \
    locales \
    && rm -rf /var/lib/apt/lists/*

# The image otherwise carries only the C locale, so GTK starts with
# "Locale not supported by C library" and falls back — which is a poor footing
# for a chat client whose whole payload is other people's text. `LANG` arrives
# from the host already set to en_US.UTF-8; this is what makes it resolve.
RUN sed -i 's/^# *en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/' /etc/locale.gen \
    && locale-gen
ENV LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8

# Docker CLI + compose plugin. `harness/run-scenario.sh` drives the 12-container
# NAT topology through `docker compose`; the container talks to the host daemon
# over the socket bind-mounted in devcontainer.json, so no daemon runs in here.
RUN install -m 0755 -d /etc/apt/keyrings \
    && curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc \
    && chmod a+r /etc/apt/keyrings/docker.asc \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian bookworm stable" \
       > /etc/apt/sources.list.d/docker.list \
    && apt-get update && apt-get install -y --no-install-recommends \
        docker-ce-cli \
        docker-compose-plugin \
    && rm -rf /var/lib/apt/lists/*

# Tauri v2's Linux build links against the system webview and its GTK stack
# (`design/05` §1), so these are compile-time dependencies of the client's
# desktop shell rather than conveniences. Two names are worth knowing before
# they waste an afternoon: Tauri v2 wants webkit2gtk **4.1**, and 4.0 is the v1
# line under a different, incompatible pkg-config module; and Debian 12 has no
# `libappindicator3-dev` at all — the tray library it ships is the Ayatana fork,
# which is what Tauri actually builds against here. `patchelf`, `file` and
# `xdg-utils` are the bundler's; `x11-utils` is how a display path gets checked
# rather than assumed.
RUN apt-get update && apt-get install -y --no-install-recommends \
    libwebkit2gtk-4.1-dev \
    libjavascriptcoregtk-4.1-dev \
    libsoup-3.0-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    patchelf \
    file \
    xdg-utils \
    x11-utils \
    && rm -rf /var/lib/apt/lists/*

# Node for the UI toolchain — the interface is HTML/CSS/JS in a webview
# (`design/05` §1), so the JS side needs a real one. Debian 12 ships Node 18,
# which is past end of life; NodeSource's 24.x is the current LTS and is what
# the Tauri CLI expects.
RUN install -m 0755 -d /etc/apt/keyrings \
    && curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key \
       | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg \
    && chmod a+r /etc/apt/keyrings/nodesource.gpg \
    && echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_24.x nodistro main" \
       > /etc/apt/sources.list.d/nodesource.list \
    && apt-get update && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# CLAUDE.md gates work on `cargo test --workspace` *and*
# `cargo clippy --workspace --all-targets`, so clippy has to be present.
RUN rustup component add clippy rustfmt

RUN curl -fsSL https://claude.ai/install.sh | bash
ENV PATH="/root/.cargo/bin:/root/.local/bin:${PATH}"

WORKDIR /workspaces/ko-ls
