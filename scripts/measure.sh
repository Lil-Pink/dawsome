#!/usr/bin/env bash
# Measures the app's steady-state cost: builds --release, runs each scenario in a real window
# for a warm-up and then a sampling window, and writes <out-dir>/results.json and results.md.
# Per scenario: CPU % of one core (process total, UI thread, audio callback thread, the rest),
# peak RSS, the process's GPU SM % and VRAM (nvidia-smi pmon), context switches per second, and
# the worst audio callback (the app's --stats). Method: scripts/measure.py; numbers and host
# in docs/performance.md.
#
# Usage: scripts/measure.sh [out-dir] [scenario..]
#   (out-dir default: <target dir>/measure; scenarios default: every scenario)
#   MEASURE_SECONDS  sampling window (default 20)
#   MEASURE_WARMUP   seconds between launch and sampling (default 4)
#   MEASURE_SINK     PipeWire node the app plays into (PIPEWIRE_NODE), e.g. an unmonitored
#                    null sink; default: the default output
#   MEASURE_BEFORE   an earlier results.json: results.md then shows each change
#
# A run whose window updated far off what its scenario makes (2 a second idle, 32 playing) is
# run once more, and marked in the results when it is off again (scripts/measure.py collect).
#
# Each run is scripted (--quit-after), so it writes no settings, recent files or autosaves and
# opens no MIDI port (--midi-ports none as well); the per-user folders point into the out dir,
# and its window's application id is dawsome-test (DAWSOME_APP_ID), so a window rule of the
# desktop can place it apart and keep it from taking focus.
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
target_dir="$(scripts/target-dir.sh)"
out_dir="${1:-$target_dir/measure}"
shift || true
seconds="${MEASURE_SECONDS:-20}"
warmup="${MEASURE_WARMUP:-4}"
mkdir -p -- "$out_dir"
out_dir="$(realpath -- "$out_dir")"
export XDG_CONFIG_HOME="$out_dir/xdg/config" XDG_CACHE_HOME="$out_dir/xdg/cache" \
    XDG_DATA_HOME="$out_dir/xdg/data" XDG_STATE_HOME="$out_dir/xdg/state"
mkdir -p -- "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"
export DAWSOME_APP_ID=dawsome-test

cargo build -q --release -p dawsome-app -p dawsome --bins
cargo build -q --release -p dawsome-test-plugin --lib
release="$target_dir/release"

# The release test plugin in a folder of its own, scanned into a catalogue there.
plugins="$out_dir/plugins"
mkdir -p -- "$plugins"
cp -- "$release/libdawsome_test_plugin.so" "$plugins/.dawsome_test_plugin.clap.tmp"
mv -f -- "$plugins/.dawsome_test_plugin.clap.tmp" "$plugins/dawsome_test_plugin.clap"
catalogue="$out_dir/catalogue.json"
"$release/dawsome" scan --plugin-path "$plugins" --cache "$catalogue" >/dev/null
fixtures="$out_dir/fixtures"
python3 -I scripts/measure.py fixtures "$fixtures"

sink_env=()
if [ -n "${MEASURE_SINK:-}" ]; then
    sink_env=("PIPEWIRE_NODE=$MEASURE_SINK")
fi
results="$out_dir/results.json"
rm -f -- "$results"
quit_ms=$(( (warmup + seconds + 2) * 1000 ))

# One launch before the measured ones, so the graphics drivers' shader caches under the out
# dir are filled: the launch that fills a cold cache holds some 45 MiB more than any after it.
env "${sink_env[@]}" "$release/dawsome-app" --load tests/fixtures/f1_four_on_floor.json \
    --midi-ports none --quit-after 3000 >/dev/null 2>&1

# run <scenario> <label> <app flags..>
# A run whose window updated more than 3 times above or below what its scenario makes
# (measure.py collect exits 3: input from outside the script, or the window covered) is run
# once more; when that one is off too, it stays in the results marked.
run() {
    local scenario="$1" label="$2"
    shift 2
    if [ "${#wanted[@]}" -gt 0 ] && ! printf '%s\n' "${wanted[@]}" | grep -qx -- "$scenario"; then
        return
    fi
    local attempt
    for attempt in 1 2; do
        echo "measure: $scenario ($label)" >&2
        env "${sink_env[@]}" "$release/dawsome-app" "$@" --midi-ports none --stats \
            --quit-after "$quit_ms" >"$out_dir/$scenario.out" 2>"$out_dir/$scenario.err" &
        local pid=$!
        python3 -I scripts/measure.py sample "$pid" "$warmup" "$seconds" >"$out_dir/$scenario.json"
        local status=0
        wait "$pid" || status=$?
        if [ "$status" -ne 0 ]; then
            echo "measure: $scenario: the app exited with $status (see $out_dir/$scenario.err)" >&2
            exit 1
        fi
        status=0
        python3 -I scripts/measure.py collect "$results" "$scenario" "$label" \
            "$out_dir/$scenario.json" "$out_dir/$scenario.out" || status=$?
        case "$status" in
            0) return ;;
            3) ;;
            *) exit "$status" ;;
        esac
        echo "measure: $scenario: its window's updates a second are off what it makes" >&2
    done
    echo "measure: $scenario: off again, kept and marked in the results" >&2
}

wanted=("$@")
f=tests/fixtures
run a_empty_idle "(a) empty document, idle"
run b_f1_idle "(b) F1, idle" --load "$f/f1_four_on_floor.json"
run c_f1_playing "(c) F1, playing" --load "$f/f1_four_on_floor.json" --play
run d_p5_delay_playing "(d) p5_delay_sync, mixer open, playing" \
    --load "$f/p5_delay_sync.json" --panels rack,mixer --play
run e_p7_clap_playing "(e) p7_sine (CLAP), playing" --load "$f/p7_sine.json" \
    --plugin-path "$plugins" --cache "$catalogue" --play
run f_p8_song_playing "(f) p8 audio clip + automation, song, playing" \
    --load "$f/p8_ui.json" --song --panels playlist --cache-dir "$out_dir/stretch" --play
run g_500_tracks_idle "(g) 500 tracks, 50 clips, playlist, idle" \
    --load "$fixtures/g_500_tracks.json" --panels playlist
run h_panels_idle "(h) mixer + playlist + piano roll, 1700×952, idle" \
    --load "$fixtures/h_session.json" --panels playlist,piano-roll,mixer --window-size 1700x952
run i_p7_clap_idle "(i) p7_sine (CLAP), idle" --load "$f/p7_sine.json" \
    --plugin-path "$plugins" --cache "$catalogue"
run j_panels_playing "(j) as (h), song, playing" --load "$fixtures/h_session.json" --song \
    --panels playlist,piano-roll,mixer --window-size 1700x952 --play

if [ -n "${MEASURE_BEFORE:-}" ]; then
    python3 -I scripts/measure.py report "$results" "$MEASURE_BEFORE" >"$out_dir/results.md"
else
    python3 -I scripts/measure.py report "$results" >"$out_dir/results.md"
fi
cat -- "$out_dir/results.md"
