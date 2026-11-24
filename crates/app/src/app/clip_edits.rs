//! Slip, split and warp markers in the window. Split (the Edit menu, Ctrl+E) cuts every selected
//! clip the song marker falls inside at the marker, or, with nothing selected, the clip under the
//! pointer at the pointer's tick (snapped, Alt free); all its cuts are one undo step and the right
//! parts become the selection. Slip (Ctrl+Shift dragged on an audio clip's body) moves the audio
//! inside the clip, its start and length kept. Warp mode (the properties strip's Warp,
//! Ctrl+Shift+W) shows a selected audio clip's markers to add, move and remove; Quantize audio
//! (the Edit menu, Ctrl+Shift+Q) moves the selected audio clips' hits onto the snap's lines with
//! markers, one undo step. The edits themselves are worked out in
//! [`clip_gesture`](crate::ui::clip_gesture) and [`warp_gesture`](crate::ui::warp_gesture).

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{ClipId, MAX_AUDIO_CLIPS};
use dawsome_engine::time::Ticks;

use super::App;
use crate::ui::clip_gesture::{self, CrossfadeRefused, SplitRefused};
use crate::ui::clips::{self, FileLook};
use crate::ui::warp_gesture::{self, ClipMap, WarpRefused};

/// What the toolbar says when a split finds no clip to cut.
pub const NOTHING_TO_SPLIT: &str = "Nothing to split: no selected clip crosses the song marker";
/// What it says when nothing is selected and the pointer is over no clip it can cut there.
pub const NOTHING_UNDER_POINTER: &str =
    "Nothing to split: select clips, or point at a clip where it should be cut";
/// What it says when warp mode finds no one audio clip to show.
pub const NOTHING_TO_WARP: &str = "Warp needs one audio clip selected";
/// What Quantize audio says with no audio clip selected.
pub const NOTHING_TO_QUANTIZE: &str = "Quantize audio needs audio clips selected";
/// What it says when the selected clips' sounds hold no hit it can find.
pub const NO_HITS: &str = "Quantize audio found no hits in the selected clips";
/// What it says while the hits of the selected clips' sounds are still being found.
pub const HITS_PENDING: &str = "The clips' hits are still being found: try again in a moment";

impl App {
    /// Split: the selected clips cut at the song marker, or the clip under the pointer at the
    /// pointer's tick; the right parts become the selection. One undo step; a notice says why
    /// when nothing is cut.
    pub(super) fn split_clips(&mut self) {
        if !self.panels.playlist {
            return;
        }
        let project = self.session.project();
        let free = self.modifiers.alt();
        let Some((ids, tick)) = self.arranger.split_target(project, self.song_marker, free) else {
            self.notice = Some(NOTHING_UNDER_POINTER.to_owned());
            return;
        };
        let pointed = self.arranger.selection.is_empty();
        match clip_gesture::split(project, &ids, tick) {
            Ok(Some((edit, rights))) => {
                self.commit(edit);
                // Kept only when the split went through: refresh drops IDs that do not exist.
                self.arranger.selection = rights;
                self.arranger.refresh(self.session.project());
            }
            Ok(None) if pointed => self.notice = Some(NOTHING_UNDER_POINTER.to_owned()),
            Ok(None) => self.notice = Some(NOTHING_TO_SPLIT.to_owned()),
            Err(SplitRefused::TooManyAudioClips) => {
                self.notice = Some(format!(
                    "Split refused: a song holds {MAX_AUDIO_CLIPS} audio clips at most"
                ));
            }
        }
    }

    /// Crossfade: the two selected audio clips that overlap on one track, the earlier given a
    /// fade out and the later a fade in across the overlap. One undo step; a notice says why
    /// when they are not such a pair.
    pub(super) fn crossfade_clips(&mut self) {
        if !self.panels.playlist {
            return;
        }
        let project = self.session.project();
        match clip_gesture::crossfade(project, &self.arranger.selection) {
            Ok(Some(edit)) => self.commit(edit),
            Ok(None) => {}
            Err(refused) => {
                self.notice = Some(
                    match refused {
                        CrossfadeRefused::NotTwoAudioClips => {
                            "Crossfade needs two audio clips selected"
                        }
                        CrossfadeRefused::OtherTracks => {
                            "Crossfade needs the two clips on one track"
                        }
                        CrossfadeRefused::NoOverlap => "Crossfade needs the two clips to overlap",
                        CrossfadeRefused::Contained => {
                            "Crossfade needs the later clip to end after the earlier one"
                        }
                    }
                    .to_owned(),
                );
            }
        }
    }

    /// Warp mode on the one selected audio clip, or off; a notice when there is no such clip.
    pub(super) fn toggle_warp(&mut self) {
        if !self.panels.playlist {
            return;
        }
        let project = self.session.project();
        if self.arranger.toggle_warp(project, None).is_err() {
            self.notice = Some(NOTHING_TO_WARP.to_owned());
        }
        self.tracks_cache.clear();
    }

    /// A double press on the body of audio clip `id` in warp mode, at song frame `song`: a
    /// marker on the transient within `reach` frames, else on the frame there, pinned where it
    /// plays now; a notice when it cannot be added.
    pub(super) fn add_warp_marker(&mut self, id: ClipId, song: u64, reach: u64) {
        let Some((frames, shortest)) = self.clip_file_frames(id) else {
            return;
        };
        let project = self.session.project();
        let onsets = project
            .playlist
            .clips
            .get(&id)
            .and_then(|clip| clip.audio_channel())
            .and_then(|channel| project.channels.get(&channel))
            .and_then(|channel| clips::onsets_of(&self.looks.files, channel))
            .unwrap_or_default();
        let Some(map) = ClipMap::of(project, id, Some(frames)) else {
            return;
        };
        let source = warp_gesture::pinned_source(&map, onsets, song, reach);
        match warp_gesture::added(project, id, &map, source, shortest) {
            Ok(Some(edit)) => self.commit(edit),
            Ok(None) => {}
            Err(refused) => self.notice = Some(self.warp_refusal(refused)),
        }
    }

    /// The toolbar's words for a marker gesture refused.
    fn warp_refusal(&self, refused: WarpRefused) -> String {
        match refused {
            WarpRefused::TooShort { shortest } => {
                let rate = u64::from(self.session.project().sample_rate.max(1));
                format!(
                    "Too short to warp: the first marker turns stretching on, {} ms at least",
                    shortest * 1000 / rate
                )
            }
            WarpRefused::Crowded => {
                "A warp marker needs 64 frames from the next one and from the clip's sound's ends"
                    .to_owned()
            }
            WarpRefused::Full => "A clip holds 256 warp markers at most".to_owned(),
        }
    }

    /// Quantize audio: on each selected audio clip, its hits moved onto the nearest line of the
    /// snap (a step when it is None) with warp markers; every clip's in one undo step. A notice
    /// says how many hits moved, or why none could.
    pub(super) fn quantize_audio(&mut self) {
        if !self.panels.playlist {
            return;
        }
        let project = self.session.project();
        let audio: Vec<ClipId> = self
            .arranger
            .selection
            .iter()
            .copied()
            .filter(|id| {
                project
                    .playlist
                    .clips
                    .get(id)
                    .is_some_and(|clip| clip.audio_channel().is_some())
            })
            .collect();
        if audio.is_empty() {
            self.notice = Some(NOTHING_TO_QUANTIZE.to_owned());
            return;
        }
        let grid = warp_gesture::quantize_grid(self.arranger.snap, &self.arranger.meter);
        let rate = project.sample_rate;
        let (mut edits, mut hits, mut kept, mut pending) = (Vec::new(), 0, 0, false);
        for id in audio {
            let look = project
                .playlist
                .clips
                .get(&id)
                .and_then(|clip| clip.audio_channel())
                .and_then(|channel| project.channels.get(&channel))
                .and_then(|channel| clips::look_of(&self.looks.files, channel));
            let Some(FileLook::Ready(peaks, Some(onsets))) = look else {
                pending = true;
                continue;
            };
            let Some(map) = ClipMap::of(project, id, Some(peaks.frames)) else {
                continue;
            };
            let sides = u8::try_from(peaks.channels.len()).unwrap_or(2);
            let shortest = dawsome_media::stretch::shortest_stretch(sides, rate);
            let found = warp_gesture::quantized(project, &map, onsets, grid);
            hits += found.hits;
            // A clip too short to stretch keeps its hits where they are: counted, not moved.
            if let Ok(edit) = warp_gesture::with_markers(project, id, &map, found.markers, shortest)
            {
                kept += found.kept;
                edits.extend(edit);
            }
        }
        self.notice = Some(match hits {
            0 if pending => HITS_PENDING.to_owned(),
            0 => NO_HITS.to_owned(),
            _ => format!("Quantize audio: {kept} of {hits} hits moved to the grid"),
        });
        if !edits.is_empty() {
            self.commit(Edit::Transaction(edits));
        }
    }

    /// A Ctrl+Shift drag of audio clip `id` ended `ticks` along: its audio moved inside it,
    /// stopping at the sound's last frame once the sound is loaded.
    pub(super) fn slip_clip(&mut self, id: ClipId, ticks: Ticks) {
        let project = self.session.project();
        let sound = project
            .playlist
            .clips
            .get(&id)
            .and_then(|clip| clip.audio_channel())
            .and_then(|channel| project.channels.get(&channel))
            .and_then(|channel| clips::look_of(&self.looks.files, channel));
        let frames = match sound {
            Some(FileLook::Ready(peaks, _)) => Some(peaks.frames),
            Some(FileLook::Loading | FileLook::Missing) | None => None,
        };
        if let Some(edit) = clip_gesture::slipped(project, id, ticks, frames) {
            self.commit(edit);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Slip and split through the window's widgets on gen_p37_expected.py's projects, the
    //! playlist alone under the toolbar of a 1000 x 560 window at the default zoom (64 px a bar:
    //! x = 120 + tick / 6), the hint bar off. Expected documents are the generator's.

    use std::collections::BTreeSet;
    use std::path::Path;

    use dawsome_engine::project::ClipId;
    use dawsome_engine::realtime::Player;
    use dawsome_engine::time::Ticks;
    use iced::keyboard::key::Code;
    use iced::keyboard::Modifiers;
    use iced::{mouse, Point, Size};

    use super::super::tests::app_on;
    use super::super::widget_events::Window;
    use super::super::{App, Message, Panel};
    use super::NOTHING_UNDER_POINTER;
    use crate::ui::arranger::PlaylistMessage;
    use crate::ui::clip_gesture::{self, ClipSnap};
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::playlist_layout::{self, HEADER_HEIGHT, RULER_HEIGHT};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn text(name: &str) -> String {
        std::fs::read_to_string(Path::new(FIXTURES).join(name)).expect("a p37 fixture")
    }

    fn p37() -> serde_json::Value {
        serde_json::from_str(&text("p37_expected.json")).expect("p37_expected.json parses")
    }

    /// `fixture` with the playlist alone in the window, snapping to `snap`.
    fn opened(fixture: &str, snap: ClipSnap) -> (App, Player, Window) {
        let (mut app, player) = app_on(&Path::new(FIXTURES).join(fixture));
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
        app.hints_flag = Some(false);
        let _ = app.update(Message::Playlist(PlaylistMessage::SnapSelected(snap)));
        assert!(app.panels.playlist && !app.panels.rack && !app.panels.piano_roll);
        let mut window = Window::new(&app);
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        (app, player, window)
    }

    /// The window point at song tick `tick` low in `clip`'s body, read off the playlist's layout.
    fn on_clip(app: &App, clip: ClipId, tick: i64) -> Point {
        let project = app.session.project();
        let shown = &project.playlist.clips[&clip];
        let index = clip_gesture::index_of(project, shown.track).expect("its track");
        let grid = &app.arranger.grid;
        let top = grid.row_top_of(index).expect("its row is shown");
        let rect = playlist_layout::clip_rect(grid, shown, top);
        let below = TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT;
        Point::new(grid.x_of(Ticks(tick)), rect.y + rect.height * 0.75 + below)
    }

    fn saved(app: &App) -> String {
        app.session.project().to_json().expect("saves")
    }

    fn moved(window: &mut Window, app: &mut App, at: Point) {
        let event = mouse::Event::CursorMoved { position: at };
        window.mouse(app, event, mouse::Cursor::Available(at));
    }

    fn ctrl_e(window: &mut Window, app: &mut App) {
        window.chord(app, 'e', Code::KeyE, Modifiers::CTRL);
    }

    fn ids(list: &[u32]) -> BTreeSet<ClipId> {
        list.iter().copied().map(ClipId).collect()
    }

    // Ctrl+Shift held, clip 1's body dragged a beat right (16 px) on the Beat snap: one trim
    // with the clip's start and length, its audio 16505 frames earlier (gen_p37's first slip,
    // 20000 to 3495); one undo gives back the file's bytes.
    #[test]
    fn p37_ctrl_shift_drag_slips_an_audio_clip_in_one_step() {
        let (mut app, _player, mut window) = opened("p37_slip.json", ClipSnap::Beat);
        let want = p37();
        let from = on_clip(&app, ClipId(1), 150);
        window.hold(&mut app, Modifiers::CTRL | Modifiers::SHIFT);
        window.drag(&mut app, from, Point::new(from.x + 16.0, from.y));
        window.hold(&mut app, Modifiers::empty());
        let after = want["slip"]["steps"][0]["after"].as_str().expect("after");
        assert_eq!(saved(&app), after);
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app) + "\n", text("p37_slip.json"));
    }

    // The same drag on a pattern clip changes nothing: pattern clips do not slip.
    #[test]
    fn ctrl_shift_drag_on_a_pattern_clip_slips_nothing() {
        let (mut app, _player, mut window) = opened("p37_cut_pattern.json", ClipSnap::Beat);
        let before = saved(&app);
        let from = on_clip(&app, ClipId(1), 150);
        window.hold(&mut app, Modifiers::CTRL | Modifiers::SHIFT);
        window.drag(&mut app, from, Point::new(from.x + 32.0, from.y));
        window.hold(&mut app, Modifiers::empty());
        assert_eq!(saved(&app), before);
    }

    // p37_cut_two: clip 2 clicked, clips 3, 4 and 5 Shift-clicked, the song marker at 150, then
    // Ctrl+E: the generator's document (clips 2 and 5 cut, their rights 6 and 7), the rights
    // selected; one undo gives back the file's bytes.
    #[test]
    fn p37_ctrl_e_splits_the_selected_clips_at_the_marker() {
        let (mut app, _player, mut window) = opened("p37_cut_two.json", ClipSnap::Bar);
        let want = p37();
        let at = on_clip(&app, ClipId(2), 200);
        let _ = window.click(&mut app, at);
        window.hold(&mut app, Modifiers::SHIFT);
        for (clip, tick) in [(3, 48), (4, 200), (5, 100)] {
            let at = on_clip(&app, ClipId(clip), tick);
            let _ = window.click(&mut app, at);
        }
        window.hold(&mut app, Modifiers::empty());
        assert_eq!(app.arranger.selection, ids(&[2, 3, 4, 5]));
        app.move_song_marker(Ticks(150));
        ctrl_e(&mut window, &mut app);
        let after = want["cuts"]["two"]["after"].as_str().expect("after");
        assert_eq!(saved(&app), after);
        assert_eq!(app.arranger.selection, ids(&[6, 7]));
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app) + "\n", text("p37_cut_two.json"));
    }

    // p37_cut_audio with nothing selected and no snap: the pointer over the empty lane right of
    // clip 1, Ctrl+E cuts nothing and says so; over clip 1 at tick 216 (x 156) it cuts there:
    // the generator's document, the right part selected; one undo gives back the file's bytes.
    #[test]
    fn p37_ctrl_e_with_nothing_selected_splits_the_clip_under_the_pointer() {
        let (mut app, _player, mut window) = opened("p37_cut_audio.json", ClipSnap::None);
        let want = p37();
        let on = on_clip(&app, ClipId(1), 216);
        assert_eq!(on.x, 156.0);
        moved(&mut window, &mut app, Point::new(on.x + 300.0, on.y));
        ctrl_e(&mut window, &mut app);
        assert_eq!(saved(&app) + "\n", text("p37_cut_audio.json"));
        assert_eq!(app.notice.as_deref(), Some(NOTHING_UNDER_POINTER));
        moved(&mut window, &mut app, on);
        ctrl_e(&mut window, &mut app);
        let after = want["cuts"]["audio"]["after"].as_str().expect("after");
        assert_eq!(saved(&app), after);
        assert_eq!(app.arranger.selection, ids(&[2]));
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app) + "\n", text("p37_cut_audio.json"));
    }

    /// The window point at the center of clip `clip`'s fade handle (`fade_in`: the left one),
    /// read off the playlist's layout.
    fn on_handle(app: &App, clip: ClipId, fade_in: bool) -> Point {
        use dawsome_engine::project::ClipItem;
        let project = app.session.project();
        let shown = &project.playlist.clips[&clip];
        let index = clip_gesture::index_of(project, shown.track).expect("its track");
        let grid = &app.arranger.grid;
        let top = grid.row_top_of(index).expect("its row is shown");
        let rect = playlist_layout::clip_rect(grid, shown, top);
        let ClipItem::Audio {
            fade_in: head,
            fade_out: tail,
            ..
        } = shown.item
        else {
            panic!("an audio clip");
        };
        let frames = project.clip_frames(shown).expect("frames");
        let reach = |fade| playlist_layout::fade_reach(fade, frames, rect.width);
        let body = top + crate::ui::playlist_layout::TITLE_HEIGHT;
        let (left, right) =
            playlist_layout::fade_handles(rect, body, reach(head), reach(tail)).expect("handles");
        let centre = if fade_in { left } else { right };
        let below = TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT;
        Point::new(centre.x, centre.y + below)
    }

    /// `fixture` opened as [`opened`] does, zoomed in as far as the playlist goes.
    fn zoomed(fixture: &str) -> (App, Player, Window) {
        let (mut app, player, window) = opened(fixture, ClipSnap::Beat);
        app.arranger.grid.zoom = 384.0;
        (app, player, window)
    }

    // p38_seam_whole's clip (ticks 200..384 at 120 BPM, 48 kHz: 250 frames a tick) zoomed in:
    // its fade in's handle dragged to tick 290 lands on the nearest beat, 288, so the fade
    // is s(288) - s(200) = 22000 frames; with Alt the drag is free, 290: 22500. One step each,
    // undone to the file's bytes.
    #[test]
    fn p38_a_fade_handle_drag_sets_the_fade_on_the_snap() {
        use dawsome_engine::project::ClipItem;
        let (mut app, _player, mut window) = zoomed("p38_seam_whole.json");
        let fade = |app: &App| match app.session.project().playlist.clips[&ClipId(1)].item {
            ClipItem::Audio {
                fade_in, fade_out, ..
            } => (fade_in, fade_out),
            _ => panic!("an audio clip"),
        };
        let from = on_handle(&app, ClipId(1), true);
        // A third of a pixel into tick 290 (two thirds of a pixel a tick at this zoom).
        let to = Point::new(app.arranger.grid.x_of(Ticks(290)) + 0.3, from.y);
        window.drag(&mut app, from, to);
        assert_eq!(fade(&app), (22000, 0));
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app) + "\n", text("p38_seam_whole.json"));
        // A fresh window: a second press on the handle this soon would be a double press.
        let (mut app, _player, mut window) = zoomed("p38_seam_whole.json");
        window.hold(&mut app, Modifiers::ALT);
        window.drag(&mut app, from, to);
        window.hold(&mut app, Modifiers::empty());
        assert_eq!(fade(&app), (22500, 0), "free");
    }

    // p38_fade_out's clip (304..384, faded out over 1500 frames): a double press on its fade
    // out's handle takes the fade off, leaving p38_plain.json's document (version 2 again).
    #[test]
    fn p38_a_double_press_on_a_fade_handle_takes_the_fade_off() {
        let (mut app, _player, mut window) = zoomed("p38_fade_out.json");
        let at = on_handle(&app, ClipId(1), false);
        window.click(&mut app, at);
        window.click(&mut app, at);
        assert_eq!(saved(&app) + "\n", text("p38_plain.json"));
    }

    /// The markers of clip 1, as (source, out) pairs.
    fn markers(app: &App) -> Vec<(u64, u64)> {
        app.session
            .project()
            .playlist
            .warp(ClipId(1))
            .iter()
            .map(|marker| (marker.source, marker.out))
            .collect()
    }

    /// The window point on the center of clip 1's warp marker handle at song tick `tick` (its
    /// line where the marker plays), read off the playlist's layout.
    fn on_marker(app: &App, tick: i64) -> Point {
        let project = app.session.project();
        let shown = &project.playlist.clips[&ClipId(1)];
        let index = clip_gesture::index_of(project, shown.track).expect("its track");
        let grid = &app.arranger.grid;
        let top = grid.row_top_of(index).expect("its row is shown");
        let below = TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT;
        let half = playlist_layout::WARP_HANDLE_HEIGHT / 2.0;
        Point::new(
            grid.x_of(Ticks(tick)),
            top + playlist_layout::TITLE_HEIGHT + half + below,
        )
    }

    // p39_plain's clip (ticks 240..384, 250 frames a tick: song frames 60000..96000, its 24000
    // frames stretched to 36000, no markers) zoomed in to a pixel a tick, on the Beat snap.
    // Selected and Ctrl+Shift+W: warp mode on it. A double press low in its body at tick 312
    // (song frame 78000, out 18000; its sound's one transient, at its start, is far off) adds
    // the marker (12000, 18000): 18000 / 1.5, on the line, so nothing moves; one step. Its
    // handle dragged to tick 330 lands on the nearest beat, 288 (song frame 72000, out 12000):
    // (12000, 12000), one step, which undoes to the list before. A double press on the handle
    // takes it out: p39_plain's bytes again. Esc leaves warp mode, the clip still selected.
    #[test]
    fn p39_warp_mode_adds_moves_and_removes_a_marker_one_step_each() {
        let (mut app, _player, mut window) = zoomed("p39_plain.json");
        app.arranger.selection = ids(&[1]);
        window.chord(
            &mut app,
            'W',
            Code::KeyW,
            Modifiers::CTRL | Modifiers::SHIFT,
        );
        assert_eq!(app.arranger.warping(), Some(ClipId(1)));
        let body = on_clip(&app, ClipId(1), 312);
        window.click(&mut app, body);
        assert!(markers(&app).is_empty(), "a single press adds nothing");
        window.click(&mut app, body);
        assert_eq!(markers(&app), [(12_000, 18_000)], "{:?}", app.notice);
        let added = saved(&app);
        let handle = on_marker(&app, 312);
        let to = Point::new(app.arranger.grid.x_of(Ticks(330)), handle.y);
        window.drag(&mut app, handle, to);
        assert_eq!(markers(&app), [(12_000, 12_000)]);
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app), added, "one step");
        let _ = app.update(Message::Redo);
        let moved = on_marker(&app, 288);
        window.click(&mut app, moved);
        window.click(&mut app, moved);
        assert_eq!(saved(&app) + "\n", text("p39_plain.json"));
        window.tap(&mut app, iced::keyboard::key::Named::Escape, Code::Escape);
        assert_eq!(app.arranger.warping(), None);
        assert_eq!(app.arranger.selection, ids(&[1]));
        // On again, then a Ctrl+drag box over empty lane (the selection emptied) ends it; the
        // clip selected again later is not in warp mode.
        let _ = app.update(Message::ToggleWarp);
        assert_eq!(app.arranger.warping(), Some(ClipId(1)));
        let lane = on_clip(&app, ClipId(1), 120);
        window.hold(&mut app, Modifiers::CTRL);
        window.drag(&mut app, lane, Point::new(lane.x + 4.0, lane.y));
        window.hold(&mut app, Modifiers::empty());
        assert!(
            app.arranger.selection.is_empty(),
            "{:?}",
            app.arranger.selection
        );
        app.arranger.selection = ids(&[1]);
        assert_eq!(app.arranger.warping(), None);
    }

    // In warp mode a drag of the body moves nothing: p39_plain's clip dragged a bar right stays.
    #[test]
    fn a_body_drag_in_warp_mode_moves_nothing() {
        let (mut app, _player, mut window) = zoomed("p39_plain.json");
        app.arranger.selection = ids(&[1]);
        let _ = app.update(Message::ToggleWarp);
        let from = on_clip(&app, ClipId(1), 300);
        window.drag(&mut app, from, Point::new(from.x + 96.0, from.y));
        assert_eq!(saved(&app) + "\n", text("p39_plain.json"));
    }

    // gen_p39's quantize through the window: p39_quantize's clip selected with the snap None,
    // then Ctrl+Shift+Q: the generator's document (the clip stretched to its 96000 frames and
    // the six markers), "6 of 7"; one undo gives back the file's bytes. With nothing selected it
    // says so.
    #[test]
    fn p39_quantize_audio_moves_the_hits_onto_the_sixteenths() {
        let want: serde_json::Value =
            serde_json::from_str(&text("p39_expected.json")).expect("p39_expected.json parses");
        let case = &want["quantize"];
        let (mut app, _player, mut window) = opened("p39_quantize.json", ClipSnap::None);
        app.arranger.selection = ids(&[1]);
        window.chord(
            &mut app,
            'Q',
            Code::KeyQ,
            Modifiers::CTRL | Modifiers::SHIFT,
        );
        assert_eq!(saved(&app), case["after"].as_str().expect("after"));
        assert_eq!(
            app.notice.as_deref(),
            Some("Quantize audio: 6 of 7 hits moved to the grid")
        );
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app) + "\n", text("p39_quantize.json"));
        app.arranger.selection.clear();
        let _ = app.update(Message::QuantizeAudio);
        assert_eq!(app.notice.as_deref(), Some(super::NOTHING_TO_QUANTIZE));
    }

    // gen_p39's "last_segment_ratio" as the Alt drag commits it: p39_warp1's clip stretched to
    // end at tick 269 is refused with the notice, nothing changed.
    #[test]
    fn p39_a_stretch_that_breaks_the_warp_markers_says_so() {
        let (mut app, _player, _window) = opened("p39_warp1.json", ClipSnap::None);
        app.stretch_clip(ClipId(1), Ticks(269));
        assert_eq!(saved(&app) + "\n", text("p39_warp1.json"));
        assert_eq!(
            app.notice.as_deref(),
            Some(super::super::browse::WARP_STRETCH_REFUSED)
        );
    }

    // p38_crossfade's two clips with their fades taken off, both selected, then Crossfade (the
    // Edit menu's, Ctrl+Shift+X): the generator's p38_crossfade document, the earlier faded out
    // and the later faded in over their 6000 shared frames; one undo step.
    #[test]
    fn p38_crossfade_fades_two_overlapping_clips_across_each_other() {
        let (mut app, _player, _window) = zoomed("p38_crossfade.json");
        let faded = saved(&app);
        for clip in [ClipId(1), ClipId(2)] {
            app.commit(dawsome_engine::edit::Edit::SetClipFades {
                clip,
                fade_in: 0,
                fade_out: 0,
            });
        }
        let plain = saved(&app);
        app.arranger.selection = ids(&[1, 2]);
        let _ = app.update(Message::Crossfade);
        assert_eq!(saved(&app), faded);
        let _ = app.update(Message::Undo);
        assert_eq!(saved(&app), plain, "one step");
        app.arranger.selection = ids(&[1]);
        let _ = app.update(Message::Crossfade);
        assert_eq!(saved(&app), plain);
        assert_eq!(
            app.notice.as_deref(),
            Some("Crossfade needs two audio clips selected")
        );
    }
}
