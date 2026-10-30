#!/usr/bin/env bash
# Headless UI check. Needs a graphical session (Wayland or X11).
# 1. F1 with the rack: probes the centers of the 16 step buttons in row 1 (positions from
#    crates/app/src/ui/layout.rs): steps 1, 5, 9 and 13 must be lit, the other 12 unlit.
# 1b. The new document (no --load) with the rack: the Sampler row lights no step, and the rack's
#    header shows "−", "+", "+ Sampler…" and the bar count holding a number (positions from
#    crates/app/src/ui/layout.rs).
# 1c. The toolbar's panel buttons (positions from crates/app/src/ui/toolbar.rs, colors of
#    its palette): on F1's shot (the rack alone) the rack's icon is lit and the other five
#    are not; with the Library, the rack and the playlist open (a first start's panels) those
#    three are lit.
# 1g. The toolbar on F1's shot (1000 px) and on the first start's panels at 1920 px (positions
#    from crates/app/src/ui/toolbar.rs): inside each group's left edge and across each line
#    between two groups, what is drawn runs from y 10 to y 62, the groups' one height. Toolbar
#    scale: at 1920 px every control has grown to the toolbar's most scale (1.6), and the
#    positions the probes read are the grown ones (the same layout function lays them out).
# 1h. The toolbar's panel icons grow with it (toolbar scale): 12 px across on F1's shot
#    (1000 px), 19 px at 1920 px and on F1's toolbar at 2560 px drawn offscreen with the
#    software renderer (app::tests::toolbar_shots). Why offscreen: the app fits its window to
#    its monitor, and this desk's side screens, where test windows open, are 1920 px wide; the
#    same 2560 px shot is probed as in 1g too.
# 1d. The starter document (tests/fixtures/a6_starter.json, what a new window and File → New
#    start with where the kit can be written) with the kit copied into the data folder and the
#    rack alone: four rows, each name drawn, no step lit, the add button's row after them.
# 1e. The empty document's shot (1b): right of the silent Sampler row's steps, its dim hint
#    "Drag a sound here, or double-click one in the Library".
# 5b. "+ Instrument…" opened at start (--open-adder) over tests/fixtures/p7_catalogue.json and
#    the four test sets of tests/fixtures/instruments copied into the data folder: its rows as
#    text (the sets, the starter kit's twelve drums, the plugins and the failed file, each
#    under its section's title, then "Sample file…"), and in the shot the sets' title on the
#    list's ground over two set rows in the two row tones.
# 2b. p3_roll with the piano roll alone and the hint bar on (--hints on; a scripted run hides
#    it otherwise): the bar fills the window's last 22 px in the panels' ground, the roll's
#    velocity lane ending where it starts.
# 2c. p3_roll with the rack, the playlist, the piano roll and the mixer at 1000x560, too short
#    for every panel's least height: the roll keeps its grid of five rows and its lane where
#    crates/app/src/ui/panels.rs stacks it, the other three squeezed.
# 1f. The instrument window opened at start (--open-instrument channel:1), a window of its own,
#    shot alone (--screenshot-window instrument): on the test set "Round robin" (a drum: one
#    pad) and the starter document's Kick 909 (a kit drum: one pad), and on the test set
#    "Layers" (a keyboard from key 36 with its roots dotted). Each twice, with the renderer the
#    toolkit picks and with the software renderer forced.
# 1i. The MIDI OUT group of an input channel's instrument window (p19_echo's "Synth", its MIDI
#    out set, --midi-out-ports standing in for the system's), shot beside a main window of
#    1000 px and of 1920 px, the instrument window seeded 560x560 and 960x560 (its own size,
#    tall enough for the group): the group's accent rule under the input's rows, the Port
#    picker filling its row to the padding, the Channel picker after the labels' column, Echo
#    and Close at the right edge above the foot line (positions from
#    crates/app/src/ui/instrument_window.rs).
# 2. p3_roll with the piano roll alone: probes notes, ghosts, rows, the bar line, the dimmed
#    rows past the end, the velocity stems and the C5 label (positions from
#    crates/app/src/ui/piano_layout.rs, colors as gen_p3_expected.py prints them); and the
#    scroll bar under the grid, its thumb the view's share at the start.
# 2d. The same with --scroll end: the grid past the pattern's end, a bar line where the scroll
#    puts it, the thumb at the bar's end (crates/app/src/ui/scroll.rs).
# 2e. p3_roll with the rack alone and a Library sound held over channel 2's header
#    (--drag-over channel:2): the header's accent outline, none on channel 1's (positions from
#    crates/app/src/ui/layout.rs).
# 2f. p5_gain (five rows) with the rack and the playlist at 1000x560 and the last row's menu
#    open (--channel-menu channel:5): every row of the menu whole, the lower ones over the
#    playlist (positions from crates/app/src/ui/menu.rs and ui/panels.rs).
# 2g. p15_long_song (a song of 19 bars, a two-bar pattern) with --scroll end: the playlist
#    alone scrolled 4 bars past the song's end, the bar at its foot with the thumb at its end; the
#    rack alone with its steps scrolled to their end under the fixed name and knobs, the bar
#    under the rows (positions from crates/app/src/ui/playlist_layout.rs, ui/layout.rs and
#    ui/scroll.rs).
# 2h. p5_gain with the rack alone at 1000x400, the hint bar on and the last row's menu open: the
#    menu stays above the hint bar and under the rack's header (too tall to flip up there, it
#    sits as low as it fits), every row whole, no menu label over the bar; and p8_ui
#    with the playlist alone at 1000x260 and the curve menu open on its automation's handle
#    (--curve-menu automation:1): the menu flips up at the tracks' foot, every row whole
#    (positions from crates/app/src/ui/menu.rs, ui/layout.rs and ui/playlist_layout.rs).
# 2i. p5_gain with the mixer alone at 1000x400, the hint bar on and the add menu of insert 12's
#    last slot open (--slot-menu insert:12,slot:9): the menu flips up above the hint bar and
#    moves left inside the window, every row whole; and p10_library with the Library alone at
#    1000x784, the hint bar on and the menu of Port A's row, its last whole row, open
#    (--library-menu "device:Port A"): the menu flips up at the panel's foot, its row whole
#    (positions from crates/app/src/ui/menu.rs, ui/mixer_layout.rs, ui/browser.rs and the
#    Library's rows). Twice, with the renderer the toolkit picks and with the software renderer
#    forced.
# 3. p4_two_clips, p4_loop and p4_loop_off with the playlist alone: probes clips, lanes, bar
#    lines, bar numbers and the loop strip (positions from
#    crates/app/src/ui/playlist_layout.rs, colors as gen_p4_expected.py prints them).
# 4. p5_gain and p5_delay_sync with the mixer alone: probes fader caps, strip bodies, the
#    selection outline, route buttons, meters and the slot names (positions from
#    crates/app/src/ui/mixer_layout.rs, colors of the mixer's palette); and that the name of
#    strip 12, which runs under the slot panel at 1000 px, stops at the panel's edge. Each run
#    says which renderer drew it.
# 4b. The same mixer shots and probes with the software renderer forced (ICED_BACKEND=tiny-skia),
#    the one the toolkit falls back to without a usable GPU: it draws a clipped canvas frame's
#    text whole, so only names the mixer cuts itself stop at the slot panel.
# 5. The plugin picker over tests/fixtures/p7_catalogue.json's instruments: its rows as text
#    (the order gen_p7_expected.py prints), three alternating row bands and the dimmer
#    failed file (positions from crates/app/src/ui/picker.rs). A scripted run reads the
#    catalogue only: it neither scans nor writes it. Then over tests/fixtures/p27_catalogue.json
#    (both formats): its rows as gen_p28_expected.py dumps them, and the format column naming
#    VST3 on the VST3 rows and CLAP on the plugin installed in both.
# 6. p8_ui at 1000x722 (the starter kit's twelve rows and the plugins' four hint lines above
#    the tree: 22 px over the 700 it was, for the line where VST3 plugins go) with the Library (its
#    Samples tree on tests/fixtures/browser) and the playlist, its lanes kept at 32 px by a
#    settings file (a height chosen, as Alt+wheel keeps one): the Library's ground, search well
#    and tree rows, the saw clip's strip, waveform, body and center line, and the automation
#    clip's strip, body and line (positions from
#    crates/app/src/ui/browser.rs, the Library's rows and playlist_layout.rs, colors of the
#    Library's and the clips' palette).
# 6b. p8_notice with no audio device: two missing samples with long names and the audio being
#    off make notices far longer than the toolbar has room for; they stay on one line, cut
#    with an ellipsis (the probe looks for the notices' color above and below the line).
# 6c. p8_notice with no audio device at 1000, 960, 930, 900 and 880 px: Retry and File are
#    whole where the toolbar places them (positions from crates/app/src/ui/toolbar.rs), and a
#    notice, whole or cut with an ellipsis, shows left of Retry.
# 6d. The same p8_notice shots: Edit is whole between Retry and File (positions from
#    crates/app/src/ui/toolbar.rs); and F1 with the Edit menu shown at start (--show edit-menu):
#    the menu's box under the toolbar, its right edge under Edit's.
# 7. p7_sine with no audio device (PipeWire and ALSA pointed at nothing): the stream cannot
#    open, its player is parked rather than dropped, and the scripted --open-plugin still finds
#    the test sine and opens its parameter window. The run lasts past the first retry of the
#    device (at 5 s), which fails again and parks the player again, and still exits 0. The
#    catalogue is scanned into the output dir.
# 8. The Settings window opened at start (--panels settings), shot alone (--screenshot-window
#    settings; positions from crates/app/src/ui/settings_layout.rs, colors of the page's
#    palette): with --midi-ports none, the "MIDI input" title is bright, the line saying no
#    port was found is dim and no checkbox is ticked; with "Port A,Port B" and a settings file
#    playing both, their checkboxes are ticked. No MIDI port is ever opened.
# 8b. The Settings window at its least size, 420 x 240 (a settings file keeping that place,
#    read and not written), with the two ports playing: its tabs, the first group's boxes, the
#    foot line and a scroller down its right edge (the page is longer than the window).
# 9. p9_stretch_ui with the playlist alone: right of the file's name, track 1's stretched clip
#    names its stretch in the title strip and track 2's unstretched clip holds nothing. The
#    stretch is kept in a cache dir under the output dir.
# 10. The Library alone (positions from crates/app/src/ui/browser.rs and the Library's rows,
#    crates/app/src/ui/library.rs; colors of its palette): p10_library at 1000x844, so all its
#    rows show, with tests/fixtures/p7_catalogue.json and two MIDI ports given, a settings file
#    playing Port A: the section headers at their rows with their counts, Port A's lit tick and
#    "→ kick", Port B's unlit tick, Port C dim as unplugged; then at 1000x800 a new document
#    with nothing installed and no port: the empty states, dim, and the Rescan button. Each
#    shot twice, with the renderer the toolkit picks and with the software renderer forced. No
#    MIDI port is ever opened, and the settings file is read, not written.
# 11. The Library alone at 1000x844 (every section header shows) with the four test sets of
#    tests/fixtures/instruments copied into the data folder's instruments/ (positions from
#    crates/app/src/ui/browser.rs and the Library's rows): the Instruments header counts the
#    kit and the sets, "Instrument sets" is written dim, and each set's row holds its glyph and
#    its name. Twice, with the renderer the toolkit picks and with the software renderer forced.
# 12. The same Library with the set "Pad" previewed (--library-preview, as a click on its row):
#    Pad's row ends in the "playing" mark in the accent and the other sets' rows have none
#    (positions from crates/app/src/ui/browser.rs and the Library's rows). Twice, with the
#    renderer the toolkit picks and with the software renderer forced.
# 13. The Settings window on its Keys page at its first size, 720x560 (--settings-page keys,
#    --screenshot-window settings; positions from crates/app/src/ui/keys_page.rs): the piano
#    row chosen, the drawn keys' faces (a white
#    and a black note, a control, a key that plays nothing, a key keeping its shortcut). Twice,
#    with the renderer the toolkit picks and with the software renderer forced.
# 14. F1 with the rack and the first-run note shown (--callouts on; a scripted run hides it
#    otherwise): the ring around the toolbar's keyboard switch, the arrow and the bubble's
#    border and ground (positions from crates/app/src/ui/callout.rs and ui/toolbar.rs). Twice,
#    as 13.
# 15. k6_forty_tracks with the Library, the rack and the playlist in a window as large as
#    1920x1080 (fitted to its monitor), the hint bar on: the playlist takes the height the rack
#    leaves, its lanes running down to its scroll bar on the hint bar (crates/app/src/ui/panels.rs,
#    ui/playlist_layout.rs).
# 16. The Help window opened at start (--show help), shot alone (--screenshot-window help) at
#    its first size: the contents on the panels' ground with the first entry lit, the page on
#    the well's ground, the foot line, and the first section's picture drawn on the page, read
#    from assets/help as a run from the build folder finds it (positions from
#    crates/app/src/ui/help_window.rs).
# 17. VST3 plugin windows (crates/host-vst3/tests/windows.rs, known answers gen_p29_expected.py):
#    the test module's View class opens its editor embedded in a window of ours (W3), resizes it
#    from inside its own call (W4), is held by its constraint when the window is resized (W5), is
#    served by its frame's run loop (W6), hears our focus as XEmbed's messages (W7), closes on every
#    way out (W8) and twenty times over leaves nothing (W9); then the real plugins' editors
#    (crates/host-vst3/tests/real_windows.rs: Surge XT's VST3, Dexed, JS80P, DPF's 3-Band EQ, LSP's
#    Limiter Stereo, each skipped with a line when absent) open embedded, map inside ours within
#    2 s, three times over, and the host goes with one open; and the app (W10) opens the View's
#    editor from --open-plugin (p29_vst3_view.json: "opened Channel(ChannelId(1)): Embedded") and
#    quits with it open, the sine's project its parameter window ("... Generic"). Each test alone,
#    one window at a time, with a temp HOME, and no window of the class dawsome-test may be left
#    open after it.
# 17b. CLAP plugin windows (crates/host-clap/tests/windows.rs, known answers gen_p29_expected.py's
#    "clap" key): the GUI file's test interface opens embedded in a window of ours and closes by
#    its button (C1), its own request_resize resizes our window, a fixed one's hints following
#    (C2), a user's resize as the window manager tells it is held by its adjust_size and our
#    window follows (C3), and a resize asked of the window manager from another client (what
#    `xdotool windowsize` asks) ends where the interface holds it (C4); then Surge XT's CLAP, when
#    fetched (crates/host-clap/tests/real_windows.rs: the fetched copy, a temp HOME), dragged
#    wider and taller that way, our window ending each time at the size of Surge's own window
#    inside it. Each alone, as 17.
# 18. The parameter window, shot alone (--screenshot-window params) at its first
#    size: p27_vst3_fx's VST3 test gain, which has no editor, opened by --open-plugin effect:1;
#    its list written out (--dump-params) is its title, "<insert> slot <n> — <plugin>" from the
#    fixture, over gen_p28's gain rows, and the shot heads the list with the place and draws the
#    rows: Gain filled to its middle, Steps to its end, Steps heard dim and empty (positions from
#    crates/app/src/ui/param_window.rs). Then, when Surge XT's VST3 is fetched, its 775 rows from
#    --open-params channel:1 with --scroll end, a temp HOME: the list's last rows are built at its
#    foot, the scroller's thumb there.
# 19. A file dropped on the app's window from another X11 client (crates/app/tests/desktop_drop.rs:
#    the app on the X display, XWayland on a Wayland desktop, the test the XDND source, the pointer
#    never moved): the pointer the app asks of the X server when the drop comes is the one the X
#    server gives the test for that window, and no window of the class dawsome-test is left open.
# 20. The sample editor opened at start (--open-sample-editor channel:1), shot alone
#    (--screenshot-window sample-editor) at its first size: p30_fades_stereo's sampler (gen_p30's
#    stereo pair faded in and out) with frames 2..6 selected (--sample-editor-select 2-6): two
#    lanes under the ruler with the wells' gap between them, the frames' dots where gen_p30 puts
#    their values, the selection's shade and its accent edges, and the foot line (positions from
#    crates/app/src/ui/sample_editor.rs and ui/sample_view.rs). Twice, with the renderer the
#    toolkit picks and with the software renderer forced.
# Every run gets per-user folders of its own under the output dir, so nothing reads or writes
# the user's settings, caches or data, and its window the application id dawsome-test
# (DAWSOME_APP_ID), so a window rule of the desktop can tell it from the app's.
# Usage: scripts/ui-check.sh [output-dir]   (default: <target dir>/ui-check)
set -euo pipefail
cd -- "$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")/.."
target_dir="$(scripts/target-dir.sh)"
out_dir="${1:-$target_dir/ui-check}"
mkdir -p -- "$out_dir"
export XDG_CONFIG_HOME="$out_dir/xdg/config" XDG_CACHE_HOME="$out_dir/xdg/cache" \
    XDG_DATA_HOME="$out_dir/xdg/data" XDG_STATE_HOME="$out_dir/xdg/state"
export DAWSOME_APP_ID=dawsome-test
rm -rf -- "$XDG_CONFIG_HOME"
mkdir -p -- "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME"

shot="$out_dir/f1.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --load tests/fixtures/f1_four_on_floor.json \
    --screenshot "$shot" --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::f1_screenshot_lights_steps_1_5_9_13 --nocapture
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::ease::a1_the_open_panels_button_is_lit_and_the_others_are_not --nocapture
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::t1_every_toolbar_group_fills_its_cell --nocapture
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::t3_the_panel_icons_grow_with_the_bar --nocapture

shot="$out_dir/t1_toolbar_1920.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --panels browser,rack,playlist --window-size 1920x600 \
    --screenshot "$shot" --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::t1_every_toolbar_group_fills_its_cell --nocapture
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::t3_the_panel_icons_grow_with_the_bar --nocapture

offscreen="$out_dir/toolbar-offscreen"
rm -rf -- "$offscreen"
mkdir -p -- "$offscreen"
DAWSOME_UI_OUT="$offscreen" DAWSOME_UI_WIDTHS=2560 cargo test -q -p dawsome-app -- --ignored \
    --exact app::tests::toolbar_shots --nocapture
shot="$offscreen/toolbar_2560.png"
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::t1_every_toolbar_group_fills_its_cell --nocapture
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::t3_the_panel_icons_grow_with_the_bar --nocapture

shot="$out_dir/a1_first_start_panels.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --panels browser,rack,playlist --window-size 1000x560 \
    --screenshot "$shot" --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::ease::a1_the_first_start_panels_buttons_are_lit --nocapture

shot="$out_dir/new.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --screenshot "$shot" --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::new_document_screenshot_shows_the_rack_header_and_an_unlit_sampler_row \
    --nocapture
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::ease::a2_an_empty_sampler_row_shows_its_hint_right_of_its_steps --nocapture

starter_data="$out_dir/xdg/data-starter"
rm -rf -- "$starter_data"
mkdir -p -- "$starter_data/dawsome/starter-kit"
cp assets/sounds/*.wav "$starter_data/dawsome/starter-kit/"
shot="$out_dir/a6_starter.png"
rm -f -- "$shot"
XDG_DATA_HOME="$starter_data" cargo run -q -p dawsome-app -- \
    --load tests/fixtures/a6_starter.json --window-size 1000x560 \
    --screenshot "$shot" --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::ease::a6_the_starter_document_shows_four_kit_rows_with_no_step_lit --nocapture

for backend in picked tiny-skia; do
    forced=()
    suffix=
    if [ "$backend" = tiny-skia ]; then
        forced=(ICED_BACKEND=tiny-skia)
        suffix=_software
    fi
    for case in "p11_rr a4_a_drum_set_shows_pads" \
        "a6_starter a4_a_kit_sound_shows_a_pad" \
        "p11_layers a4_a_pitched_set_shows_a_keyboard_with_its_roots"; do
        fixture="${case%% *}"
        probe="${case##* }"
        shot="$out_dir/a4_$fixture$suffix.png"
        rm -f -- "$shot"
        renderer="$(env "${forced[@]}" XDG_DATA_HOME="$starter_data" cargo run -q -p dawsome-app -- \
            --load "tests/fixtures/$fixture.json" --window-size 1000x700 \
            --open-instrument channel:1 --screenshot "$shot" --screenshot-window instrument \
            --screenshot-after 1200 --quit-after 4500 --stats | sed -n 's/^renderer: //p')"
        echo "ui-check: the instrument window on $fixture drew with ${renderer:-no renderer}"
        if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
            echo "ui-check: the forced software run of $fixture drew with '$renderer'" >&2
            exit 1
        fi
        DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
            "probe::ease::$probe" --nocapture
    done
done

for case in "1000 560" "1920 960"; do
    main="${case%% *}"
    instrument="${case##* }"
    shot="$out_dir/m1_midi_out_$main.png"
    rm -f -- "$shot"
    midi_out_config="$out_dir/xdg/config-midi-out-$instrument"
    rm -rf -- "$midi_out_config"
    mkdir -p -- "$midi_out_config/dawsome"
    printf '{"instrument_window":{"width":%s.0,"height":560.0}}\n' "$instrument" \
        > "$midi_out_config/dawsome/settings.json"
    XDG_CONFIG_HOME="$midi_out_config" cargo run -q -p dawsome-app -- \
        --load tests/fixtures/p19_echo.json --window-size "${main}x700" \
        --midi-out-ports "Synth:Synth MIDI 1 24:0,Midi Through:Midi Through Port-0 14:0" \
        --open-instrument channel:1 --screenshot "$shot" --screenshot-window instrument \
        --screenshot-after 1200 --quit-after 4500
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::midi_out::m1_the_midi_out_group_lines_up_under_the_input --nocapture
done

roll="$out_dir/p3.png"
rm -f -- "$roll"
cargo run -q -p dawsome-app -- --load tests/fixtures/p3_roll.json --panels piano-roll \
    --window-size 1000x560 --screenshot "$roll" --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$roll" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p3_screenshot_shows_the_piano_roll --nocapture
DAWSOME_UI_SHOT="$roll" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::p2_the_rolls_bar_shows_the_view_at_its_start --nocapture

roll="$out_dir/p2_roll_end.png"
rm -f -- "$roll"
cargo run -q -p dawsome-app -- --load tests/fixtures/p3_roll.json --panels piano-roll \
    --window-size 1000x560 --scroll end --screenshot "$roll" --screenshot-after 800 \
    --quit-after 4000
DAWSOME_UI_SHOT="$roll" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::p2_the_roll_scrolled_to_its_end --nocapture

shot="$out_dir/p4_drag_over.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --load tests/fixtures/p3_roll.json --panels rack \
    --window-size 1000x560 --drag-over channel:2 --screenshot "$shot" --screenshot-after 800 \
    --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::p4_a_held_sound_outlines_the_header_it_would_replace --nocapture

shot="$out_dir/p5_channel_menu.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --load tests/fixtures/p5_gain.json --panels rack,playlist \
    --window-size 1000x560 --channel-menu channel:5 --screenshot "$shot" \
    --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::p5_the_last_rack_rows_menu_shows_whole_over_the_playlist --nocapture

shot="$out_dir/p5_menu_over_hint_bar.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --load tests/fixtures/p5_gain.json --panels rack \
    --window-size 1000x400 --hints on --channel-menu channel:5 --screenshot "$shot" \
    --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::p5_a_menu_stays_above_the_hint_bar --nocapture

shot="$out_dir/p5_curve_menu.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --load tests/fixtures/p8_ui.json --panels playlist \
    --window-size 1000x260 --hints on --curve-menu automation:1 --screenshot "$shot" \
    --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::p5_the_curve_menu_shows_whole_at_the_tracks_foot --nocapture

for backend in picked tiny-skia; do
    forced=()
    suffix=
    if [ "$backend" = tiny-skia ]; then
        forced=(ICED_BACKEND=tiny-skia)
        suffix=_software
    fi
    shot="$out_dir/p5_slot_menu$suffix.png"
    rm -f -- "$shot"
    renderer="$(env "${forced[@]}" cargo run -q -p dawsome-app -- \
        --load tests/fixtures/p5_gain.json --panels mixer --window-size 1000x400 --hints on \
        --slot-menu insert:12,slot:9 --screenshot "$shot" --screenshot-after 800 \
        --quit-after 4000 --stats | sed -n 's/^renderer: //p')"
    echo "ui-check: the slot menu drew with ${renderer:-no renderer}"
    if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
        echo "ui-check: the forced software run of the slot menu drew with '$renderer'" >&2
        exit 1
    fi
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::fixes::p5_a_slot_menu_shows_whole_at_the_window_foot --nocapture

    shot="$out_dir/p5_library_row_menu$suffix.png"
    rm -f -- "$shot"
    renderer="$(env "${forced[@]}" cargo run -q -p dawsome-app -- \
        --load tests/fixtures/p10_library.json --panels browser \
        --browser-root tests/fixtures/browser --cache tests/fixtures/p7_catalogue.json \
        --midi-ports "Port A,Port B" --window-size 1000x784 --hints on \
        --library-menu "device:Port A" --screenshot "$shot" --screenshot-after 800 \
        --quit-after 4000 --stats | sed -n 's/^renderer: //p')"
    echo "ui-check: the Library row menu drew with ${renderer:-no renderer}"
    if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
        echo "ui-check: the forced software run of the Library row menu drew with '$renderer'" >&2
        exit 1
    fi
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::fixes::p5_the_library_row_menu_shows_whole_at_the_panels_foot --nocapture
done

for case in "playlist p6_the_playlist_scrolled_past_the_song_end" \
    "rack p6_the_racks_steps_scrolled_to_their_end"; do
    panel="${case%% *}"
    probe="${case##* }"
    shot="$out_dir/p6_${panel}_end.png"
    rm -f -- "$shot"
    cargo run -q -p dawsome-app -- --load tests/fixtures/p15_long_song.json --panels "$panel" \
        --window-size 1000x560 --scroll end --screenshot "$shot" --screenshot-after 800 \
        --quit-after 4000
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        "probe::fixes::$probe" --nocapture
done

roll="$out_dir/b2_four_panels.png"
rm -f -- "$roll"
cargo run -q -p dawsome-app -- --load tests/fixtures/p3_roll.json \
    --panels rack,playlist,piano-roll,mixer --window-size 1000x560 --screenshot "$roll" \
    --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$roll" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::ease::b2_the_piano_roll_keeps_its_grid_under_three_other_panels --nocapture

roll="$out_dir/a5_hint_bar.png"
rm -f -- "$roll"
cargo run -q -p dawsome-app -- --load tests/fixtures/p3_roll.json --panels piano-roll \
    --window-size 1000x560 --hints on --screenshot "$roll" --screenshot-after 800 \
    --quit-after 4000
DAWSOME_UI_SHOT="$roll" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::ease::a5_the_hint_bar_sits_under_the_panels --nocapture

for fixture in p4_two_clips p4_loop p4_loop_off; do
    shot="$out_dir/$fixture.png"
    rm -f -- "$shot"
    cargo run -q -p dawsome-app -- --load "tests/fixtures/$fixture.json" --panels playlist \
        --window-size 1000x560 --screenshot "$shot" --screenshot-after 800 --quit-after 4000
done
DAWSOME_UI_SHOT="$out_dir/p4_two_clips.png" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p4_screenshot_shows_the_playlist --nocapture
DAWSOME_UI_SHOT="$out_dir/p4_loop.png" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p4_loop_screenshot_lights_the_loop_strip --nocapture
DAWSOME_UI_SHOT="$out_dir/p4_loop_off.png" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p4_loop_off_screenshot_dims_the_loop_strip --nocapture

# A shot of mixer fixture $1 at $2, printing the renderer that drew it; further words are
# environment assignments for the run.
mixer_shot() {
    local fixture="$1" shot="$2"
    shift 2
    rm -f -- "$shot"
    env "$@" cargo run -q -p dawsome-app -- --load "tests/fixtures/$fixture.json" --panels mixer \
        --window-size 1000x560 --screenshot "$shot" --screenshot-after 800 --quit-after 4000 \
        --stats | sed -n 's/^renderer: //p'
}
# The mixer's probes on the shots of p5_gain and p5_delay_sync named $1 and $2.
mixer_probes() {
    DAWSOME_UI_SHOT="$1" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::tests::p5_screenshot_shows_the_mixer --nocapture
    DAWSOME_UI_SHOT="$1" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::tests::p5_strip_names_stop_at_the_slot_panel --nocapture
    DAWSOME_UI_SHOT="$2" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::tests::p5_delay_screenshot_names_the_effect_in_slot_1 --nocapture
}
for fixture in p5_gain p5_delay_sync; do
    renderer="$(mixer_shot "$fixture" "$out_dir/$fixture.png")"
    echo "ui-check: $fixture drew with ${renderer:-no renderer}"
done
mixer_probes "$out_dir/p5_gain.png" "$out_dir/p5_delay_sync.png"
for fixture in p5_gain p5_delay_sync; do
    renderer="$(mixer_shot "$fixture" "$out_dir/${fixture}_software.png" ICED_BACKEND=tiny-skia)"
    if [ "$renderer" != tiny-skia ]; then
        echo "ui-check: the forced software run of $fixture drew with '$renderer'" >&2
        exit 1
    fi
done
mixer_probes "$out_dir/p5_gain_software.png" "$out_dir/p5_delay_sync_software.png"

shot="$out_dir/p7_picker.png"
dump="$out_dir/p7_picker.txt"
rm -f -- "$shot" "$dump"
cargo run -q -p dawsome-app -- --cache tests/fixtures/p7_catalogue.json --open-picker instrument \
    --screenshot "$shot" --screenshot-after 1500 --quit-after 5000 --dump-picker "$dump"
printf 'Analog Lead\nBell Keys\nDawsome Test Sine\nCould not load (1)\nbroken.clap: crashed: signal 6\n' \
    | cmp - "$dump"
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p7_picker_screenshot_lists_three_instruments_and_dims_the_failed_file --nocapture

shot="$out_dir/p27_picker.png"
dump="$out_dir/p27_picker.txt"
rm -f -- "$shot" "$dump"
cargo run -q -p dawsome-app -- --cache tests/fixtures/p27_catalogue.json --open-picker instrument \
    --screenshot "$shot" --screenshot-after 1500 --quit-after 5000 --dump-picker "$dump"
printf '%s\n' 'Dawsome Test Fail (VST3)' 'Dawsome Test Sine' 'Dawsome Test Tick (VST3)' \
    'Dawsome Test View (VST3)' 'Could not load (1)' 'Dawsome Test VST3 Crash.vst3: crashed: signal 6' |
    cmp - "$dump"
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p27_picker_screenshot_names_each_rows_format --nocapture

adder_data="$out_dir/xdg/data-adder"
rm -rf -- "$adder_data"
mkdir -p -- "$adder_data/dawsome/instruments"
cp -R tests/fixtures/instruments/. "$adder_data/dawsome/instruments/"
shot="$out_dir/a2_adder.png"
dump="$out_dir/a2_adder.txt"
rm -f -- "$shot" "$dump"
XDG_DATA_HOME="$adder_data" cargo run -q -p dawsome-app -- \
    --cache tests/fixtures/p7_catalogue.json --open-adder --window-size 1000x560 \
    --screenshot "$shot" --screenshot-after 1500 --quit-after 5000 --dump-picker "$dump"
printf '%s\n' 'Instrument sets' Layers Macro Pad 'Round robin' 'Starter kit' 'Kick 808' \
    'Kick 909' Snare Clap 'Hat closed' 'Hat open' 'Tom low' 'Tom mid' 'Tom high' Rim Cowbell \
    Crash Plugins 'Analog Lead' 'Bell Keys' 'Dawsome Test Sine' 'Could not load (1)' \
    'broken.clap: crashed: signal 6' 'Sample file…' | cmp - "$dump"
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::ease::a2_the_instrument_picker_shows_its_sections --nocapture

shot="$out_dir/p8_ui.png"
rm -f -- "$shot"
# The playlist's lanes kept at 32 px, as a height chosen with Alt+wheel keeps them (its settings
# file's "lane_height"), the height the clips' probe points were drawn for; the lanes fit the
# panel otherwise.
lanes_config="$out_dir/xdg/config-lanes-32"
rm -rf -- "$lanes_config"
mkdir -p -- "$lanes_config/dawsome"
printf '{"playlist":{"lane_height":32.0}}\n' > "$lanes_config/dawsome/settings.json"
# 1500 ms: the saw's waveform loads in the background; with the Library's rows drawn beside it,
# the debug build had it on screen at 1000 ms but not at 800 (and before the Library, not at 600).
XDG_CONFIG_HOME="$lanes_config" cargo run -q -p dawsome-app -- --load tests/fixtures/p8_ui.json \
    --panels browser,playlist --browser-root tests/fixtures/browser --window-size 1000x722 \
    --screenshot "$shot" --screenshot-after 1500 --quit-after 4000
printf '{"playlist":{"lane_height":32.0}}\n' | cmp - "$lanes_config/dawsome/settings.json"
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p8_screenshot_shows_the_browser_an_audio_clip_and_an_automation_line --nocapture

shot="$out_dir/p8_notice.png"
rm -f -- "$shot"
PIPEWIRE_REMOTE=bogus ALSA_CONFIG_PATH=/dev/null cargo run -q -p dawsome-app -- \
    --load tests/fixtures/p8_notice.json --window-size 1000x560 \
    --screenshot "$shot" --screenshot-after 800 --quit-after 4000 2>/dev/null
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p8_notices_stay_on_one_line_in_the_toolbar --nocapture
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p8_toolbar_keeps_retry_and_file_whole_beside_a_notice --nocapture
for width in 960 930 900 880; do
    shot="$out_dir/p8_notice_$width.png"
    rm -f -- "$shot"
    PIPEWIRE_REMOTE=bogus ALSA_CONFIG_PATH=/dev/null cargo run -q -p dawsome-app -- \
        --load tests/fixtures/p8_notice.json --window-size "${width}x560" \
        --screenshot "$shot" --screenshot-after 800 --quit-after 4000 2>/dev/null
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::tests::p8_toolbar_keeps_retry_and_file_whole_beside_a_notice --nocapture
done
for shot in "$out_dir"/p8_notice.png "$out_dir"/p8_notice_{960,930,900,880}.png; do
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::ease::b4_the_edit_button_is_whole_between_retry_and_file --nocapture
done

shot="$out_dir/b4_edit_menu.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --load tests/fixtures/f1_four_on_floor.json --show edit-menu \
    --window-size 1000x560 --screenshot "$shot" --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::ease::b4_the_edit_menu_opens_under_the_edit_button --nocapture

scripts/build-test-plugins.sh >/dev/null
cache="$out_dir/p7_no_device_catalogue.json"
rm -f -- "$cache"
cargo run -q -p dawsome -- scan --plugin-path "$target_dir/test-plugins" --cache "$cache" >/dev/null
opened="$(PIPEWIRE_REMOTE=bogus ALSA_CONFIG_PATH=/dev/null cargo run -q -p dawsome-app -- \
    --load tests/fixtures/p7_sine.json --plugin-path "$target_dir/test-plugins" --cache "$cache" \
    --open-plugin channel:1 --quit-after 6000 2>/dev/null)"
if [ "$opened" != 'opened Channel(ChannelId(1)): Generic' ]; then
    echo "ui-check: with no audio device the test sine's panel did not open: '$opened'" >&2
    exit 1
fi

shot="$out_dir/p9_settings_none.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --panels settings --midi-ports none --window-size 1000x560 \
    --screenshot "$shot" --screenshot-window settings --screenshot-after 800 --quit-after 4000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p9_settings_screenshot_says_no_port_was_found --nocapture

shot="$out_dir/p9_settings_ports.png"
ports_config="$out_dir/xdg/config-two-ports"
rm -rf -- "$ports_config" "$shot"
mkdir -p -- "$ports_config/dawsome"
printf '{"midi_inputs":["Port A","Port B"]}\n' > "$ports_config/dawsome/settings.json"
XDG_CONFIG_HOME="$ports_config" cargo run -q -p dawsome-app -- --panels settings \
    --midi-ports "Port A,Port B" --window-size 1000x560 \
    --screenshot "$shot" --screenshot-window settings --screenshot-after 800 --quit-after 4000
printf '{"midi_inputs":["Port A","Port B"]}\n' | cmp - "$ports_config/dawsome/settings.json"
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p9_settings_screenshot_ticks_the_two_ports_that_play --nocapture

shot="$out_dir/t2_settings_least.png"
least_config="$out_dir/xdg/config-settings-least"
rm -rf -- "$least_config" "$shot"
mkdir -p -- "$least_config/dawsome"
least='{"midi_inputs":["Port A","Port B"],"settings_window":{"width":420.0,"height":240.0}}'
printf '%s\n' "$least" > "$least_config/dawsome/settings.json"
XDG_CONFIG_HOME="$least_config" cargo run -q -p dawsome-app -- --panels settings \
    --midi-ports "Port A,Port B" \
    --screenshot "$shot" --screenshot-window settings --screenshot-after 800 --quit-after 4000
printf '%s\n' "$least" | cmp - "$least_config/dawsome/settings.json"
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::fixes::t2_the_least_settings_window_keeps_its_tabs_and_first_group --nocapture

shot="$out_dir/p9_stretch_ui.png"
stretch_cache="$out_dir/stretch-cache"
rm -rf -- "$stretch_cache" "$shot"
mkdir -p -- "$stretch_cache"
cargo run -q -p dawsome-app -- --load tests/fixtures/p9_stretch_ui.json --panels playlist \
    --cache-dir "$stretch_cache" --window-size 1000x560 \
    --screenshot "$shot" --screenshot-after 1500 --quit-after 5000
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::tests::p9_stretch_screenshot_names_the_stretch_after_the_file --nocapture

library_config="$out_dir/xdg/config-library"
rm -rf -- "$library_config"
mkdir -p -- "$library_config/dawsome"
printf '{"midi_inputs":["Port A"]}\n' > "$library_config/dawsome/settings.json"
for backend in picked tiny-skia; do
    forced=()
    suffix=
    if [ "$backend" = tiny-skia ]; then
        forced=(ICED_BACKEND=tiny-skia)
        suffix=_software
    fi
    shot="$out_dir/p10_library$suffix.png"
    rm -f -- "$shot"
    renderer="$(env "${forced[@]}" XDG_CONFIG_HOME="$library_config" cargo run -q -p dawsome-app -- \
        --load tests/fixtures/p10_library.json --panels browser --browser-root tests/fixtures/browser \
        --cache tests/fixtures/p7_catalogue.json --midi-ports "Port A,Port B" \
        --window-size 1000x844 --screenshot "$shot" --screenshot-after 800 --quit-after 4000 \
        --stats | sed -n 's/^renderer: //p')"
    echo "ui-check: p10_library drew with ${renderer:-no renderer}"
    if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
        echo "ui-check: the forced software run of p10_library drew with '$renderer'" >&2
        exit 1
    fi
    printf '{"midi_inputs":["Port A"]}\n' | cmp - "$library_config/dawsome/settings.json"
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::tests::p10_library_screenshot_shows_the_sections_counts_and_device_links --nocapture

    shot="$out_dir/p10_library_empty$suffix.png"
    rm -f -- "$shot"
    renderer="$(env "${forced[@]}" cargo run -q -p dawsome-app -- --panels browser \
        --browser-root tests/fixtures/browser --midi-ports none --window-size 1000x800 \
        --screenshot "$shot" --screenshot-after 800 --quit-after 4000 --stats \
        | sed -n 's/^renderer: //p')"
    echo "ui-check: the empty Library drew with ${renderer:-no renderer}"
    if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
        echo "ui-check: the forced software run of the empty Library drew with '$renderer'" >&2
        exit 1
    fi
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::tests::p10_library_screenshot_hints_what_is_empty --nocapture
done

sets_data="$out_dir/xdg/data-sets"
rm -rf -- "$sets_data"
mkdir -p -- "$sets_data/dawsome/instruments"
cp -R tests/fixtures/instruments/. "$sets_data/dawsome/instruments/"
for backend in picked tiny-skia; do
    forced=()
    suffix=
    if [ "$backend" = tiny-skia ]; then
        forced=(ICED_BACKEND=tiny-skia)
        suffix=_software
    fi
    shot="$out_dir/p11_library$suffix.png"
    rm -f -- "$shot"
    renderer="$(env "${forced[@]}" XDG_DATA_HOME="$sets_data" cargo run -q -p dawsome-app -- \
        --panels browser --browser-root tests/fixtures/browser --midi-ports none \
        --window-size 1000x844 --screenshot "$shot" --screenshot-after 800 --quit-after 4000 \
        --stats | sed -n 's/^renderer: //p')"
    echo "ui-check: the Library with sets drew with ${renderer:-no renderer}"
    if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
        echo "ui-check: the forced software run of the Library with sets drew with '$renderer'" >&2
        exit 1
    fi
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::tests::p11_library_screenshot_lists_the_instrument_sets --nocapture
done

for backend in picked tiny-skia; do
    forced=()
    suffix=
    if [ "$backend" = tiny-skia ]; then
        forced=(ICED_BACKEND=tiny-skia)
        suffix=_software
    fi
    shot="$out_dir/v2_library_preview$suffix.png"
    rm -f -- "$shot"
    renderer="$(env "${forced[@]}" XDG_DATA_HOME="$sets_data" cargo run -q -p dawsome-app -- \
        --panels browser --browser-root tests/fixtures/browser --midi-ports none \
        --library-preview "$sets_data/dawsome/instruments/pad/pad.sfz" \
        --window-size 1000x800 --screenshot "$shot" --screenshot-after 800 --quit-after 4000 \
        --stats | sed -n 's/^renderer: //p')"
    echo "ui-check: the Library previewing Pad drew with ${renderer:-no renderer}"
    if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
        echo "ui-check: the forced software run of the Library previewing Pad drew with '$renderer'" >&2
        exit 1
    fi
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::tests::v2_the_library_marks_the_set_it_previews --nocapture
done

for backend in picked tiny-skia; do
    forced=()
    suffix=
    if [ "$backend" = tiny-skia ]; then
        forced=(ICED_BACKEND=tiny-skia)
        suffix=_software
    fi
    for case in "k3_keys_page --panels settings --settings-page keys --screenshot-window settings k3_the_keys_page_draws_the_piano_row" \
        "k4_callout --load tests/fixtures/f1_four_on_floor.json --callouts on k4_the_keyboard_note_rings_the_switch_and_points_at_it"; do
        read -r -a rest <<<"$case"
        name="${rest[0]}"
        probe="${rest[-1]}"
        rest=("${rest[@]:1:${#rest[@]}-2}")
        shot="$out_dir/$name$suffix.png"
        rm -f -- "$shot"
        renderer="$(env "${forced[@]}" cargo run -q -p dawsome-app -- "${rest[@]}" \
            --screenshot "$shot" --screenshot-after 1200 --quit-after 4500 \
            --stats | sed -n 's/^renderer: //p')"
        echo "ui-check: $name drew with ${renderer:-no renderer}"
        if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
            echo "ui-check: the forced software run of $name drew with '$renderer'" >&2
            exit 1
        fi
        DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
            "probe::keys::$probe" --nocapture
    done
done

shot="$out_dir/k6_playlist_maximized.png"
rm -f -- "$shot"
cargo run -q -p dawsome-app -- --load tests/fixtures/k6_forty_tracks.json \
    --panels browser,rack,playlist --window-size 1920x1080 --hints on \
    --screenshot "$shot" --screenshot-after 1200 --quit-after 4500
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::keys::k6_the_playlist_reaches_the_hint_bar --nocapture

shot="$out_dir/h1_help_window.png"
rm -f -- "$shot"
env -u DAWSOME_HELP_DIR cargo run -q -p dawsome-app -- --show help --screenshot-window help \
    --null-output --midi-ports none \
    --screenshot "$shot" --screenshot-after 1200 --quit-after 4500
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::help::h1_the_help_window_shows_its_contents_its_page_and_a_picture --nocapture

# 17. VST3 plugin windows. The windows carry the class dawsome-test (the tests set it), so a desktop
# rule keeps them off the user's screen and focus; any such window still open after a test fails
# the check.
no_test_window_left() {
    if ! command -v xprop >/dev/null; then
        echo "ui-check: xprop not installed: the open-window check after $1 is skipped"
        return
    fi
    local window
    for window in $(xprop -root _NET_CLIENT_LIST | sed -n 's/.*# //p' | tr ',' ' '); do
        if xprop -id "$window" WM_CLASS 2>/dev/null | grep -q '"dawsome-test"'; then
            echo "ui-check: $1 left a window of class dawsome-test open: $window" >&2
            exit 1
        fi
    done
}
window_home="$out_dir/window-home"
# The tests run with a temp HOME; cargo and rustup keep finding the toolchain where it is.
toolchain=(RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}")
for test in w3_an_editor_opens_embedded_in_our_window w4_an_editor_resizes_its_window_inside_its_call \
    w5_a_users_resize_is_held_by_the_editors_constraint \
    w6_the_frames_run_loop_serves_the_editors_file_descriptors \
    w7_our_focus_reaches_the_editor_as_xembed_messages w8_an_editor_closes_on_every_way_out \
    w9_twenty_editors_opened_and_closed_leave_nothing; do
    rm -rf -- "$window_home"
    mkdir -p -- "$window_home"
    env "${toolchain[@]}" HOME="$window_home" cargo test -q -p dawsome-host-vst3 --test windows -- \
        --ignored --exact "$test" --nocapture
    no_test_window_left "$test"
done
# 17b. CLAP plugin windows, each alone with a temp HOME, as the VST3 ones.
for test in c1_an_interface_opens_embedded_in_our_window c2_a_plugins_request_resizes_our_window \
    c3_a_users_resize_is_held_by_the_interface_and_our_window_follows \
    c4_a_resize_asked_of_the_window_manager_ends_where_the_interface_holds_it; do
    rm -rf -- "$window_home"
    mkdir -p -- "$window_home"
    env "${toolchain[@]}" HOME="$window_home" cargo test -q -p dawsome-host-clap --test windows -- \
        --ignored --exact "$test" --nocapture
    no_test_window_left "$test"
done
rm -rf -- "$window_home"
mkdir -p -- "$window_home"
env "${toolchain[@]}" HOME="$window_home" cargo test -q -p dawsome-host-clap --test real_windows -- \
    --ignored --exact surge_xt_holds_our_window_to_its_interface --nocapture
no_test_window_left "Surge XT's CLAP interface"
# W10: the app opens the View's editor embedded from --open-plugin and quits with it open; the
# sine, which has no editor, opens its parameter window. No audio device or MIDI port is opened.
vst3_cache="$out_dir/p29_catalogue.json"
rm -f -- "$vst3_cache"
cargo run -q -p dawsome -- scan --plugin-path "$target_dir/test-plugins-vst3" --cache "$vst3_cache" \
    >/dev/null
for case in "p29_vst3_view.json Embedded" "p27_vst3_sine.json Generic"; do
    read -r fixture editor <<<"$case"
    opened="$(cargo run -q -p dawsome-app -- --load "tests/fixtures/$fixture" \
        --plugin-path "$target_dir/test-plugins-vst3" --cache "$vst3_cache" --null-output \
        --midi-ports none --open-plugin channel:1 --quit-after 3000 2>/dev/null)"
    if [ "$opened" != "opened Channel(ChannelId(1)): $editor" ]; then
        echo "ui-check: $fixture's plugin did not open as $editor: '$opened'" >&2
        exit 1
    fi
    no_test_window_left "the app on $fixture"
done
# 18: the parameter window, on the VST3 test gain, then on Surge XT's VST3 scrolled to its end.
params_dump="$out_dir/p27_params.txt"
shot="$out_dir/p27_params.png"
rm -f -- "$params_dump" "$shot"
cargo run -q -p dawsome-app -- --load tests/fixtures/p27_vst3_fx.json \
    --plugin-path "$target_dir/test-plugins-vst3" --cache "$vst3_cache" --null-output \
    --midi-ports none --open-plugin effect:1 --dump-params "$params_dump" \
    --screenshot "$shot" --screenshot-after 2000 --screenshot-window params --quit-after 3500 \
    >/dev/null 2>&1
python3 -I - "$params_dump" <<'PY'
import json, sys
fixture = json.load(open("tests/fixtures/p27_vst3_fx.json", encoding="utf-8"))
insert = next(insert for insert in fixture["mixer"]["inserts"] if insert["effects"])
effect = insert["effects"][0]
title = f"{insert['name']} slot {effect['slot'] + 1} — {effect['processor']['name']}"
rows = json.load(open("tests/fixtures/p28_expected.json", encoding="utf-8"))["steps"]["gain_rows"]
lines = open(sys.argv[1], encoding="utf-8").read().splitlines()
names = [line.split("\t")[0] for line in lines[1:]]
if lines[:1] != [title] or names != rows:
    sys.exit(f"ui-check: the parameter window lists {lines!r}, not {title!r} over {rows!r}")
PY
DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
    probe::params::p1_the_parameter_window_heads_its_list_and_draws_its_rows --nocapture
no_test_window_left "the parameter window on p27_vst3_fx"
surge_vst3="$target_dir/surge/linux-x86_64-vst3"
if [ -d "$surge_vst3/Surge XT.vst3" ]; then
    params_home="$out_dir/params-home"
    rm -rf -- "$params_home"
    mkdir -p -- "$params_home"
    surge_cache="$out_dir/surge_vst3_catalogue.json"
    surge_project="$out_dir/surge_vst3.json"
    shot="$out_dir/surge_params_end.png"
    rm -f -- "$surge_cache" "$shot"
    env "${toolchain[@]}" HOME="$params_home" cargo run -q -p dawsome -- scan \
        --plugin-path "$surge_vst3" --cache "$surge_cache" >/dev/null
    # p27's VST3 sine project with Surge XT's VST3 on its channel instead, in its default state.
    python3 -I - "$surge_cache" "$surge_project" <<'PY'
import json, sys
catalogue = json.load(open(sys.argv[1], encoding="utf-8"))
plugin = next(plugin for record in catalogue["files"] for plugin in record.get("plugins", [])
              if plugin["name"] == "Surge XT")
project = json.load(open("tests/fixtures/p27_vst3_sine.json", encoding="utf-8"))
project["channels"][0]["name"] = "Surge XT (VST3)"
project["channels"][0]["processor"] = {"id": f"vst3:{plugin['id']}", "name": "Surge XT"}
json.dump(project, open(sys.argv[2], "w", encoding="utf-8"))
PY
    env "${toolchain[@]}" HOME="$params_home" cargo run -q -p dawsome-app -- \
        --load "$surge_project" --plugin-path "$surge_vst3" --cache "$surge_cache" --null-output \
        --midi-ports none --open-params channel:1 --scroll end \
        --screenshot "$shot" --screenshot-after 6000 --screenshot-window params --quit-after 8000 \
        >/dev/null 2>&1
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::params::p2_a_long_list_scrolled_to_its_end_builds_its_last_rows --nocapture
    no_test_window_left "Surge XT's parameter window"
else
    echo "ui-check: Surge XT's VST3 is not fetched (scripts/fetch-surge.sh --vst3): its parameter window is skipped"
fi
# 20: the sample editor on p30_fades_stereo's sampler, frames 2..6 selected, with the renderer
# the toolkit picks and with the software renderer forced.
for backend in picked tiny-skia; do
    forced=()
    suffix=
    if [ "$backend" = tiny-skia ]; then
        forced=(ICED_BACKEND=tiny-skia)
        suffix=_software
    fi
    shot="$out_dir/p30_sample_editor$suffix.png"
    rm -f -- "$shot"
    renderer="$(env "${forced[@]}" cargo run -q -p dawsome-app -- \
        --load tests/fixtures/p30_fades_stereo.json --null-output --midi-ports none \
        --open-sample-editor channel:1 --sample-editor-select 2-6 --screenshot "$shot" \
        --screenshot-after 1500 --screenshot-window sample-editor --quit-after 3000 --stats \
        | sed -n 's/^renderer: //p')"
    echo "ui-check: the sample editor drew with ${renderer:-no renderer}"
    if [ "$backend" = tiny-skia ] && [ "$renderer" != tiny-skia ]; then
        echo "ui-check: the forced software run of the sample editor drew with '$renderer'" >&2
        exit 1
    fi
    DAWSOME_UI_SHOT="$shot" cargo test -q -p dawsome-app -- --ignored --exact \
        probe::sample_editor::e1_the_editor_draws_two_lanes_the_selection_and_its_foot_line \
        --nocapture
    no_test_window_left "the sample editor on p30_fades_stereo"
done
for test in surge_xt_opens_its_editor dexed_opens_its_editor js80p_opens_its_editor \
    dpf_3_band_eq_opens_its_editor lsp_limiter_stereo_opens_its_editor; do
    cargo test -q -p dawsome-host-vst3 --test real_windows -- --ignored --exact "$test" --nocapture
    no_test_window_left "$test"
done
cargo test -q -p dawsome-app --test desktop_drop -- --ignored --nocapture
no_test_window_left "the desktop drop"
