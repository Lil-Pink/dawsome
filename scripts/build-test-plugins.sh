#!/usr/bin/env bash
# Builds Dawsome's own CLAP and VST3 test plugins and puts them where the tests look for them:
# <target dir>/test-plugins/dawsome_test_plugin.clap (the sine instrument and the gain effect)
# and <target dir>/test-plugins/dawsome_test_crash.clap (an entry that aborts as it loads, so every
# scan of the folder also proves a crashing file is survived), plus the same library as
# <target dir>/test-plugin-variants/dawsome_test_variants.clap, whose name makes it offer the
# big-state and failing variants instead (a folder of its own keeps them out of the scans of the
# first), and as <target dir>/test-plugin-gui/dawsome_test_gui.clap, the sine with an interface
# (the plugin windows' tests); and the VST3 bundles <target dir>/test-plugins-vst3/Dawsome Test VST3.vst3 (the sine,
# gain, fail, tick, latency and view classes) and Dawsome Test VST3 Crash.vst3 (its ModuleEntry
# aborts), each binary at Contents/<arch>-linux/<bundle name>.so (Contents/<arch>-win/<bundle
# name>.vst3 on Windows), the bundle's own name, as the format asks. scripts/check.sh runs this
# first. On Windows it runs under the Git Bash or MSYS shell.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
target_dir="$(scripts/target-dir.sh)"
cargo build -q -p dawsome-test-plugin --lib --example dawsome_test_crash
cargo build -q -p dawsome-test-plugin-vst3 --lib --example dawsome_test_vst3_crash
# Copied, not linked: a later build replaces the .so in place, and a plugin a test has loaded
# must not change under it. Why a temp file renamed into place rather than `cp -f`: cp rewrites
# the existing file, so a test binary that has it mapped (another run of the tests) reads the
# new bytes mid-run and crashes; a rename leaves the old file whole for whoever has it open.
# The temp name is hidden and not .clap, so a scan of the folder never sees it half written.
# An unchanged library is not copied at all, so the catalogues of earlier runs stay fresh.
install_plugin() {
    local from="$1" to="$2"
    if cmp -s -- "$from" "$to"; then
        return
    fi
    local temp
    temp="$(dirname -- "$to")/.$(basename -- "$to").$$.tmp"
    cp -- "$from" "$temp"
    mv -f -- "$temp" "$to"
}
# The file names cargo gives the libraries.
# Where a VST3 bundle keeps this platform's binary, and that binary's extension.
arch="$(uname -m)"
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
        library() { printf '%s.dll' "$1"; }
        vst3_folder="$arch-win" vst3_extension=vst3
        ;;
    *)
        library() { printf 'lib%s.so' "$1"; }
        vst3_folder="$arch-linux" vst3_extension=so
        ;;
esac
plugin="$target_dir/debug/$(library dawsome_test_plugin)"
out="$target_dir/test-plugins"
mkdir -p -- "$out"
install_plugin "$plugin" "$out/dawsome_test_plugin.clap"
install_plugin "$target_dir/debug/examples/$(library dawsome_test_crash)" \
    "$out/dawsome_test_crash.clap"
variants="$target_dir/test-plugin-variants"
mkdir -p -- "$variants"
install_plugin "$plugin" "$variants/dawsome_test_variants.clap"
gui="$target_dir/test-plugin-gui"
mkdir -p -- "$gui"
install_plugin "$plugin" "$gui/dawsome_test_gui.clap"
# A VST3 bundle: <name>.vst3/Contents/<folder>/<name>.<extension>.
install_bundle() {
    local from="$1" bundle="$2" name
    name="$(basename -- "$bundle" .vst3)"
    mkdir -p -- "$bundle/Contents/$vst3_folder"
    install_plugin "$from" "$bundle/Contents/$vst3_folder/$name.$vst3_extension"
}
vst3="$target_dir/test-plugins-vst3"
install_bundle "$target_dir/debug/$(library dawsome_test_plugin_vst3)" "$vst3/Dawsome Test VST3.vst3"
install_bundle "$target_dir/debug/examples/$(library dawsome_test_vst3_crash)" \
    "$vst3/Dawsome Test VST3 Crash.vst3"
echo "test plugins in $out, $variants, $gui and $vst3"
