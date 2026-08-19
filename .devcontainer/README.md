# Dev container

Builds, tests and lints **both** Rust workspaces — `distributed-intranet` (the
protocol) and `ko-ls` (the client that builds against it by path dependency) —
runs the protocol's Docker NAT harness, and builds and runs the client's Tauri
desktop app.

## Use it

Open the **`ko-ls` repo folder**, not its parent. VS Code: **Dev Containers:
Reopen in Container**, or from a terminal:

```sh
devcontainer up --workspace-folder .        # from inside the ko-ls repo
```

**Why the folder you open is not the folder you land in.** This config is tracked
in the client repo, so it is version-controlled with the work it serves. But the
client is a path dependency on its sibling, so a container that mounted only this
repo would be missing half the build. `workspaceMount` therefore binds the
*parent* of both repos and `workspaceFolder` lands at `/workspaces/ko-ls` — the
layout `design/07` §2 draws. A fresh machine needs both repos cloned side by side
before any of this works.

The bind source resolves to `<...>/ko-ls/..` rather than a normalised path — the
spec has no "parent of the workspace folder" variable, so this is the idiom. If a
host daemon ever refuses it, replacing the `source=` with the absolute parent path
is the whole fix.

```sh
cd distributed-intranet                    # the protocol
cargo test --workspace
cargo clippy --workspace --all-targets     # both halves of the CLAUDE.md gate
./harness/run-scenario.sh all              # needs the docker socket mount

cd ../ko-ls                                # the client
cargo test
cargo clippy --workspace --all-targets
./scripts/cross-check.sh                   # big-endian, not part of the default gate
```

## What's in the image

| Piece | Why |
|---|---|
| `rust:1-bookworm` | workspace is edition 2024 / rust-version 1.85 |
| `clippy`, `rustfmt` | `cargo clippy --workspace --all-targets` is part of the gate |
| `build-essential`, `pkg-config`, `libssl-dev` | native builds for the crypto/transport crates |
| Docker CLI + compose plugin | `harness/run-scenario.sh` brings up the 12-container NAT topology |
| `iproute2`, `iputils-ping`, `jq`, `curl` | tools the harness scripts reach for |
| webkit2gtk **4.1**, JavaScriptCore, libsoup 3, Ayatana appindicator, librsvg | Tauri v2 links against the system webview and its GTK stack |
| `patchelf`, `file`, `xdg-utils` | the Tauri bundler's |
| `x11-utils` | `xdpyinfo` and `xwininfo`, for checking the display path rather than assuming it |
| Node.js 24 LTS (NodeSource) | the UI toolchain; Debian 12 ships Node 18, which is past end of life |
| `locales` + generated `en_US.UTF-8` | without it GTK falls back to the C locale, which is poor footing for a client whose payload is other people's text |
| Claude Code | same as the workspace's own dev container |

Two Tauri package names are worth knowing before they cost an afternoon: v2 wants
webkit2gtk **4.1** (4.0 is the v1 line, a different pkg-config module), and Debian
12 has no `libappindicator3-dev` at all — the tray library it ships is the Ayatana
fork.

## GUI from the container

Verified rather than assumed: a Tauri v2 window maps on the X display VS Code
forwards into the container, and the WebKit child process comes up with it.

```sh
xdpyinfo | head -3                      # is there a display at all
GDK_BACKEND=x11 npm run tauri dev       # deterministic; a window on $DISPLAY
```

There is no `/mnt/wslg` in here — WSLg is on the *host*, and what reaches the
container is VS Code's own forwarding: an X server on `$DISPLAY` and a Wayland
socket in `$XDG_RUNTIME_DIR`. Both are live. GTK prefers the Wayland one when
`WAYLAND_DISPLAY` is set, which is fine but leaves nothing for `xwininfo` to
see, so `GDK_BACKEND=x11` is the setting to reach for when a window needs to be
checked from a script rather than looked at.

## Notes

- The harness containers are built and run by the **host** daemon through the
  bind-mounted `/var/run/docker.sock`; nothing runs a daemon inside this
  container. They need `NET_ADMIN` and `net.ipv4.ip_forward`, which
  `harness/docker/compose.yml` already requests. If your host has no socket at
  that path, remove the mount — everything except the NAT scenarios still works.
- Cargo's registry and **both** workspaces' `target/` are named volumes
  (`ko-ls-cargo-registry`, `ko-ls-cargo-target` for the protocol,
  `ko-ls-client-target` for the client), so the container's toolchain doesn't
  rebuild over the host's artifacts. `docker volume rm` them for a clean slate.
  The protocol's volume is named `ko-ls-cargo-target` because it was created
  before there was a client to distinguish it from; renaming it now would discard
  a multi-gigabyte cache to fix a word.
- The client's `target/` only became a volume once there was a client — before
  that it sat on the bind mount, which is the case these volumes exist to avoid.
  The first build after that change recompiles from scratch, and whatever was in
  `ko-ls/target` on the host stays there, shadowed and unreachable, until it is
  deleted from outside the container.
- This is the only dev container config in the workspace. `distributed-intranet`
  used to carry its own, scoped to that repo alone; it named itself `dclone-dev`,
  predated the harness and the clippy half of the gate, and was superseded rather
  than maintained.
