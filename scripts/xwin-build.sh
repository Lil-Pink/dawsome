#!/usr/bin/env bash
# Cross-builds the Windows programs (x86_64-pc-windows-msvc) from Linux with cargo-xwin.
# Usage: scripts/xwin-build.sh [cargo-xwin arguments]
#   default: build --release --config "$(scripts/remap-paths.sh)" --target
#            x86_64-pc-windows-msvc -p dawsome -p dawsome-app -p dawsome-test-plugin
#            (the release programs, naming none of this machine's folders)
#   e.g.:    scripts/xwin-build.sh clippy --target x86_64-pc-windows-msvc --workspace \
#              --all-targets -- -D warnings
# Needs: `rustup target add x86_64-pc-windows-msvc`, `cargo install cargo-xwin`, and clang,
# clang-cl, lld-link, llvm-lib, llvm-rc (the app's icon and version details), libclang, make
# and a shell for LAME's configure on the PATH.
# The Windows SDK and C runtime cargo-xwin downloads (about 1 GB) go to $XWIN_CACHE_DIR, by
# default <target dir>/xwin, so nothing lands in the user's cache folder. The programs land in
# <target dir>/x86_64-pc-windows-msvc/release/.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
target_dir="$(scripts/target-dir.sh)"
export XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-$target_dir/xwin}"
case "$XWIN_CACHE_DIR" in
    *[[:space:]]*)
        echo "xwin-build: XWIN_CACHE_DIR must not contain spaces (the preprocessor line below)" >&2
        exit 1
        ;;
esac
# LAME's build script picks its build by the machine it runs on, not by the target, so on Linux
# it runs LAME's configure. Told the host is a Windows one, configure checks with the target's
# compiler and runs nothing it builds; its preprocessor check needs the target's headers, which
# cargo-xwin gives the compiler only.
sdk="$XWIN_CACHE_DIR/xwin"
export MP3LAME_SYS_OVERRIDE_HOST=x86_64-w64-mingw32
export CPP="clang-cl --target=x86_64-pc-windows-msvc -E /imsvc $sdk/crt/include \
/imsvc $sdk/sdk/include/ucrt /imsvc $sdk/sdk/include/um /imsvc $sdk/sdk/include/shared"
if [ "$#" -eq 0 ]; then
    set -- build --release --config "$(scripts/remap-paths.sh)" --target x86_64-pc-windows-msvc \
        -p dawsome -p dawsome-app -p dawsome-test-plugin
fi
cargo xwin "$@"
