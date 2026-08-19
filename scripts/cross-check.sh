#!/usr/bin/env bash
# Verify the canonical encoding on a big-endian target.
#
# design/08 §3 obligation 6: the frozen vectors are produced on a little-endian
# machine, so running them only there proves they are self-consistent, not that
# the encoding is host-independent. `Enc` writes big-endian explicitly and this
# codebase contains no native-endian conversion, no transmute and no unsafe —
# but that is an argument, and this is the test that replaces it.
#
# Not part of the default gate: it needs an emulator and a cross-linker, and
# takes about a minute. Run it whenever the encoding, its vectors, or a
# dependency in the hashing/signing path changes.
#
# Prerequisites (Debian/Ubuntu):
#   apt-get install -y qemu-user-static gcc-s390x-linux-gnu
#   rustup target add s390x-unknown-linux-gnu
set -euo pipefail

TARGET=s390x-unknown-linux-gnu

for tool in qemu-s390x-static s390x-linux-gnu-gcc; do
    command -v "$tool" >/dev/null || {
        echo "missing $tool — see the prerequisites in this script" >&2
        exit 1
    }
done

rustup target list --installed | grep -qx "$TARGET" || {
    echo "missing rust target $TARGET — rustup target add $TARGET" >&2
    exit 1
}

echo "running kols-core tests on $TARGET (big-endian, emulated)"
cargo test -p kols-core --target "$TARGET"

# kols-net is deliberately not cross-tested: it exercises the network stack
# rather than the encoding, and libp2p's dependency tree does not cross-compile
# without further work. The bytes that must be host-independent are all in
# kols-core.
echo "ok — encoding is byte-identical on a big-endian host"
