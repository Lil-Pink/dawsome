//! Files dropped on the main window from the desktop that the rack's sampler rows did not take.
//! The drop carries no position of its own (the toolkit's file drop has none), so the window
//! system is asked where the pointer is over the main window (X11's QueryPointer, Win32's cursor
//! position), and only when it cannot say is the cursor the window last saw used; with neither,
//! or anywhere but the playlist, the file goes to the song position on a free track as before
//! ([`App::place_files`]). On the playlist's lanes a sound file is a new audio channel and one
//! clip of the whole file on the track under the pointer, from the tick there floored to the
//! playlist's Snap, as a file dragged there from the Library lands; below the last track a track
//! is added first, as "+ Track" adds one. The window hands a drop of several files over one file
//! at a time, each at the same point: the files after the first stack on the following tracks
//! from the same start, and the whole drop is one undo step. Elsewhere on the playlist (its
//! header strip, ruler, track headers and scroll bar) a drop lands nowhere.

use std::path::Path;

use dawsome_engine::edit::{self, Edit, StateId};
use dawsome_engine::time::Ticks;
use dawsome_host_common::window::WindowId;
use iced::{Point, Rectangle};

use super::App;
use crate::ui::clip_gesture;
use crate::ui::playlist_layout::{self, TRACK_HEADER_WIDTH};

/// A desktop drop on the tracks under way: where it was let go of, where its files start, the
/// track its next file lands on, and the document's state after its last file.
#[derive(Debug, Clone, Copy)]
pub(super) struct TracksDrop {
    /// Tracks canvas point.
    at: Point,
    start: Ticks,
    /// The index, in track order, of the track the next file lands on.
    next: usize,
    /// A file let go of at `at` belongs to this drop only while the document is in this state:
    /// nothing has changed it since the drop's last file.
    state: StateId,
}

/// What the window system said of the pointer as a file was dropped.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Asked {
    /// Nothing was asked: the main window is of no system the app asks (a Wayland window), or
    /// is a test's.
    Nothing,
    /// The system could not say.
    Unknown,
    /// The window's device pixels from its top-left corner, and the window point they are at
    /// the window's scale.
    At { device: (i32, i32), at: Point },
}

/// Where a drop at a window point lands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Place {
    /// On the playlist's lanes, at this tracks canvas point.
    Lanes(Point),
    /// On the playlist outside its lanes: nowhere.
    Playlist,
    /// Anywhere else, or nowhere known: the song position, on a free track.
    SongPosition,
}

/// The main window as the window system names it, from the toolkit's handle: its X11 id, its
/// Win32 handle; none for a Wayland window, whose file drops the toolkit does not deliver.
pub(super) fn native_window(window: &dyn iced::window::Window) -> Option<WindowId> {
    use raw_window_handle::RawWindowHandle;
    match window.window_handle().ok()?.as_raw() {
        #[cfg(unix)]
        RawWindowHandle::Xlib(handle) => WindowId::try_from(handle.window).ok(),
        #[cfg(unix)]
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        #[cfg(windows)]
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

impl App {
    /// A file dropped on the main window from the desktop, the window having seen the cursor at
    /// window point `seen` (none when it saw none, or the drop was not over the playlist): it
    /// lands where the window system says the pointer is, else at `seen`, as [`Place`] says.
    pub(super) fn desktop_drop(&mut self, file: &Path, seen: Option<Point>) {
        let asked = self.ask_pointer();
        // A pointer the system places outside the window cannot be where the drop was.
        let window = Rectangle::with_size(self.window_size);
        let pointer = match asked {
            Asked::At { at, .. } if window.contains(at) => Some(at),
            _ => None,
        };
        let place = self.place_of(pointer.or(seen));
        if self.host.drop_monitor {
            println!("{}", monitor_line(file, asked, seen, place));
        }
        let next_clip = self.session.project().playlist.next_clip_id;
        match place {
            Place::Lanes(at) => self.drop_on_tracks(file, at),
            Place::Playlist => {}
            Place::SongPosition => self.place_files(&[file.to_path_buf()]),
        }
        if self.host.drop_monitor {
            if let Some(clip) = self.session.project().playlist.clips.get(&next_clip) {
                println!(
                    "dawsome-app: drop: clip {} on track {} from tick {}",
                    next_clip.0, clip.track.0, clip.start.0
                );
            }
        }
    }

    /// Where the window system says the pointer is over the main window.
    fn ask_pointer(&self) -> Asked {
        let Some(window) = self.main_native else {
            return Asked::Nothing;
        };
        match (self.host.pointer)(window) {
            Some((x, y)) => {
                let scale = self.scale_factor.max(f32::MIN_POSITIVE);
                Asked::At {
                    device: (x, y),
                    at: Point::new(x as f32 / scale, y as f32 / scale),
                }
            }
            None => Asked::Unknown,
        }
    }

    /// Where a drop at window point `at` lands: the playlist's lanes and the rest of the panel
    /// by where the panel was last laid out ([`playlist_layout::tracks_rect`]).
    fn place_of(&self, at: Option<Point>) -> Place {
        let Some(at) = at else {
            return Place::SongPosition;
        };
        let panel = self.arranger.panel();
        if !self.panels.playlist || !panel.contains(at) {
            return Place::SongPosition;
        }
        let tracks = playlist_layout::tracks_rect(panel);
        let point = Point::new(at.x - tracks.x, at.y - tracks.y);
        if tracks.contains(at) && point.x >= TRACK_HEADER_WIDTH {
            Place::Lanes(point)
        } else {
            Place::Playlist
        }
    }

    /// A file dropped from the desktop at tracks canvas point `at`, right of the track headers:
    /// a new audio channel and its clip on the track there (the next track, for a file after
    /// the first of a drop), a track added under the last when there is none, the new channel
    /// selected; a file the app cannot read says so and takes no track.
    fn drop_on_tracks(&mut self, file: &Path, at: Point) {
        let state = self.session.state();
        let run = self
            .browse
            .tracks_drop
            .filter(|run| run.at == at && run.state == state);
        let (index, start) = match run {
            Some(run) => (run.next, run.start),
            None => match self.arranger.drop_point(self.session.project(), at) {
                Some(point) => point,
                None => return,
            },
        };
        let mut project = self.session.project().clone();
        let mut edits = Vec::new();
        let track = match clip_gesture::track_at(&project, index) {
            Some(track) => track,
            None => {
                let id = project.playlist.next_track_id;
                let add = Edit::AddTrack {
                    id,
                    name: format!("Track {}", id.0),
                };
                // Applied to the copy so the clip's edit is made with the track there; refused
                // (the playlist is full), it is reported as "+ Track"'s would be.
                if let Err(error) = edit::apply(&mut project, add.clone()) {
                    self.report(Err(error.into()));
                    return;
                }
                edits.push(add);
                id
            }
        };
        let channel = project.next_channel_id;
        match self.new_audio_clip(&project, file, start, Some(track)) {
            Ok(Some(add)) => edits.push(add),
            Ok(None) => return,
            Err(notice) => {
                self.notice = Some(notice);
                return;
            }
        }
        let edit = match edits.len() {
            1 => edits.remove(0),
            _ => Edit::Transaction(edits),
        };
        match run {
            Some(_) => self.commit_into_newest(edit),
            None => self.commit(edit),
        }
        if self.session.state() == state {
            // Refused: the error says why.
            return;
        }
        if run.is_none() {
            self.select_channel(channel);
        }
        let project = self.session.project();
        let next = clip_gesture::index_of(project, track).map_or(index, |index| index + 1);
        self.browse.tracks_drop = Some(TracksDrop {
            at,
            start,
            next,
            state: self.session.state(),
        });
    }
}

/// What `--drop-monitor` prints of a drop: the file, the pointer the window system gave, the
/// cursor the window saw, and where it lands.
fn monitor_line(file: &Path, asked: Asked, seen: Option<Point>, place: Place) -> String {
    let pointer = match asked {
        Asked::Nothing => "not asked".to_owned(),
        Asked::Unknown => "unknown".to_owned(),
        Asked::At { device: (x, y), at } => format!("{x},{y} px = {},{}", at.x, at.y),
    };
    let seen = seen.map_or_else(|| "nowhere".to_owned(), |at| format!("{},{}", at.x, at.y));
    let lands = match place {
        Place::Lanes(at) => format!("on the lanes at {},{}", at.x, at.y),
        Place::Playlist => "on the playlist outside its lanes: nowhere".to_owned(),
        Place::SongPosition => "at the song position".to_owned(),
    };
    format!(
        "dawsome-app: drop {}: pointer {pointer}; seen {seen}; lands {lands}",
        file.display()
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use dawsome_engine::project::{ChannelId, ChannelKind, ClipId, TrackId};
    use dawsome_engine::realtime::Player;
    use dawsome_engine::time::Ticks;
    use dawsome_media::write::{Encoding, Sink};
    use iced::{mouse, Point, Rectangle, Size};

    use super::super::tests::app_hosted;
    use super::super::widget_events::Window;
    use super::super::{App, FileMessage, MediaMessage, Message};
    use crate::document::Host;
    use crate::project_file;
    use crate::ui::arranger::PlaylistMessage;
    use crate::ui::clip_gesture::ClipSnap;
    use crate::ui::layout::TOOLBAR_HEIGHT;
    use crate::ui::panels::Panel;
    use crate::ui::playlist_layout::{HEADER_HEIGHT, RULER_HEIGHT};
    use dawsome_host_common::window::WindowId;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    /// The tracks canvas's top in the window: under the 72 px toolbar, the 24 px header strip
    /// and the 28 px ruler, y 116 (`playlist_layout`'s probe coordinates).
    const TRACKS_TOP: f32 = TOOLBAR_HEIGHT + HEADER_HEIGHT + RULER_HEIGHT;

    fn document(app: &App) -> String {
        app.session.project().to_json().expect("serializes")
    }

    /// A folder of the test's own holding p8_ui.json, the sounds its channels play, and under
    /// `sounds/` the browser fixture's tone.wav (one frame of 1.0 at 44.1 kHz) and notes.txt
    /// (no sound).
    fn project_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("dawsome-app-tracks-drop-{}", std::process::id()))
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sounds")).expect("temp dir");
        for name in ["p8_ui.json", "p8_saw.wav", "impulse.wav"] {
            std::fs::copy(Path::new(FIXTURES).join(name), dir.join(name)).expect("copies");
        }
        for name in ["tone.wav", "notes.txt"] {
            let from = Path::new(FIXTURES).join("browser").join(name);
            std::fs::copy(from, dir.join("sounds").join(name)).expect("copies");
        }
        dir
    }

    /// The app on `dir`'s p8_ui (44.1 kHz, 120 BPM, PPQ 96: a bar is 384 ticks; eight tracks,
    /// track 1 holding an audio clip over 0..768 and track 2 an automation clip over 0..384),
    /// the playlist alone in a 1000 × 560 window, the hint bar off, its canvases laid out: the
    /// tracks area is 560 − 64 − 24 − 28 − 10 = 434 px tall (the toolbar 64 px since 2026-10-27),
    /// so the eight tracks and the "+ Track" row fit lanes of floor(434 / 9) − 1 = 47 px, a row
    /// every 48 px; 64 px a bar
    /// from x 120, scrolled to tick 0.
    fn drop_app(dir: &Path) -> (App, Player, Window) {
        let path = dir.join("p8_ui.json");
        let project = project_file::open(&path).expect("p8_ui opens").project;
        let (mut app, player) = app_hosted(project, &path, Host::offline());
        let _ = app.update(Message::TogglePanel(Panel::Playlist));
        let _ = app.update(Message::TogglePanel(Panel::Rack));
        let _ = app.update(Message::WindowResized(Size::new(1000.0, 560.0)));
        app.hints_flag = Some(false);
        let panels = app.panels;
        assert!(panels.playlist && !panels.rack && !panels.piano_roll && !panels.mixer);
        assert!(!panels.browser);
        let mut window = Window::new(&app);
        // The tracks canvas reports its size on its first event.
        window.mouse(
            &mut app,
            mouse::Event::CursorLeft,
            mouse::Cursor::Unavailable,
        );
        assert_eq!(app.arranger.tracks_size(), Size::new(1000.0, 434.0));
        assert_eq!(
            app.arranger.panel(),
            Rectangle::new(Point::new(0.0, TOOLBAR_HEIGHT), Size::new(1000.0, 496.0))
        );
        assert_eq!(app.arranger.grid.lane, 47.0);
        (app, player, window)
    }

    /// The window's cursor at window point `at`, as the window saw it last.
    fn seen(at: Point) -> mouse::Cursor {
        mouse::Cursor::Available(at)
    }

    fn snap(app: &mut App, snap: ClipSnap) {
        let _ = app.update(Message::Playlist(PlaylistMessage::SnapSelected(snap)));
    }

    /// Track 3's lane (row 2, tracks y 96..143, window y 212..259) at window x 270, 150 px into
    /// the timeline: tick floor(150 · 384 / 64) = 900, which Beat (96 ticks) floors to 864.
    const ON_TRACK_3: Point = Point::new(270.0, TRACKS_TOP + 114.0);

    fn clip_of(app: &App, id: u32) -> (TrackId, Ticks, Ticks, Option<ChannelId>) {
        let clip = &app.session.project().playlist.clips[&ClipId(id)];
        (clip.track, clip.start, clip.length, clip.audio_channel())
    }

    fn channel_of(app: &App, id: u32) -> (String, ChannelKind, PathBuf) {
        let channel = &app.session.project().channels[&ChannelId(id)];
        (channel.name.clone(), channel.kind, channel.sample.clone())
    }

    // Known answer: tone.wav dropped from the desktop on track 3 at x 270 with the Snap on
    // Beat is p8_ui's channel 3, an audio channel "tone" playing sounds/tone.wav (named
    // relative to the project's folder, as the Library's drop names a file inside it), and its
    // clip 3 on track 3 from tick 864, one tick long: tick 864 is sample 864 · 229.6875 =
    // 198450 at 120 BPM and 44.1 kHz, tick 865 sample 198679, so one tick holds the one frame.
    // The canvas takes the drop (the window places nothing more), the new channel is selected,
    // the whole drop is one undo step, the saved file holds the channel and the clip, and one
    // undo gives back the document exactly.
    #[test]
    fn p8_a_wav_dropped_from_the_desktop_on_track_3_lands_there_on_the_snap() {
        let dir = project_dir("wav_on_track_3");
        let (mut app, _player, mut window) = drop_app(&dir);
        snap(&mut app, ClipSnap::Beat);
        let before = document(&app);
        let tone = dir.join("sounds/tone.wav");
        let published = window.drop_files(&mut app, std::slice::from_ref(&tone), seen(ON_TRACK_3));
        assert!(
            matches!(
                published.as_slice(),
                [Message::Media(MediaMessage::DroppedOnPlaylist { file, seen })]
                    if *file == tone && *seen == ON_TRACK_3
            ),
            "{published:?}"
        );
        assert_eq!(app.session.project().channels.len(), 3);
        assert_eq!(
            channel_of(&app, 3),
            ("tone".into(), ChannelKind::Audio, "sounds/tone.wav".into())
        );
        assert_eq!(
            clip_of(&app, 3),
            (TrackId(3), Ticks(864), Ticks(1), Some(ChannelId(3)))
        );
        assert_eq!(app.session.project().playlist.clips.len(), 3);
        assert_eq!(app.selected_channel, Some(ChannelId(3)));
        assert_eq!(app.session.history().undo_len(), 1);
        assert_eq!(app.notice, None);

        let _ = app.update(Message::File(FileMessage::Save));
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("p8_ui.json")).expect("saved"))
                .expect("JSON");
        let channel = saved["channels"]
            .as_array()
            .and_then(|channels| channels.iter().find(|channel| channel["id"] == 3))
            .expect("channel 3 saved");
        assert_eq!(
            (&channel["name"], &channel["kind"], &channel["sample"]),
            (
                &serde_json::json!("tone"),
                &serde_json::json!("audio"),
                &serde_json::json!("sounds/tone.wav")
            )
        );
        let clip = saved["playlist"]["clips"]
            .as_array()
            .and_then(|clips| clips.iter().find(|clip| clip["id"] == 3))
            .expect("clip 3 saved");
        let field = |name: &str| clip[name].as_i64();
        assert_eq!(
            (
                field("track"),
                field("start"),
                field("length"),
                field("channel")
            ),
            (Some(3), Some(864), Some(1), Some(3))
        );
        assert_eq!(clip["kind"], "audio");

        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // A WAV and an MP3 let go of together on track 3 (the MP3 written here,
    // 4410 frames at 44.1 kHz). The window hands them over one at a time at the same point:
    // tone lands as above, and hum on the next track, track 4, from the same tick 864, as
    // channel 4 and clip 4, at least 20 ticks long (4410 frames from sample 198450 end past
    // tick 883's sample 202814, by tick 884's 203043; the encoder may add a few frames). One
    // undo step for both, the first file's channel selected, and one undo gives back the
    // document exactly.
    #[test]
    fn p8_a_wav_and_an_mp3_dropped_together_stack_on_tracks_3_and_4() {
        let dir = project_dir("wav_and_mp3");
        let hum = dir.join("sounds/hum.mp3");
        let mut sink = Sink::create(&hum, 44_100, Encoding::Mp3).expect("creates");
        sink.write(&[[0.25f32, 0.25]; 4410]).expect("encodes");
        sink.finish().expect("flushes");
        let (mut app, _player, mut window) = drop_app(&dir);
        snap(&mut app, ClipSnap::Beat);
        let before = document(&app);
        let files = [dir.join("sounds/tone.wav"), hum];
        window.drop_files(&mut app, &files, seen(ON_TRACK_3));
        assert_eq!(app.session.project().channels.len(), 4);
        assert_eq!(
            clip_of(&app, 3),
            (TrackId(3), Ticks(864), Ticks(1), Some(ChannelId(3)))
        );
        assert_eq!(
            channel_of(&app, 4),
            ("hum".into(), ChannelKind::Audio, "sounds/hum.mp3".into())
        );
        let (track, start, length, channel) = clip_of(&app, 4);
        assert_eq!(
            (track, start, channel),
            (TrackId(4), Ticks(864), Some(ChannelId(4)))
        );
        assert!(length >= Ticks(20), "{length:?}");
        assert_eq!(app.selected_channel, Some(ChannelId(3)));
        assert_eq!(app.session.history().undo_len(), 1, "one step for the drop");
        assert_eq!(app.notice, None);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // A file that is no sound (notes.txt) dropped on track 3 says "Cannot read <file>", as the
    // window's drop of such a file always has, and adds nothing; dropped together with tone.wav
    // it takes no track, so tone lands on track 3, the track under the cursor.
    #[test]
    fn p8_a_file_that_is_no_sound_says_so_and_adds_nothing() {
        let dir = project_dir("no_sound");
        let (mut app, _player, mut window) = drop_app(&dir);
        snap(&mut app, ClipSnap::Beat);
        let before = document(&app);
        let notes = dir.join("sounds/notes.txt");
        window.drop_files(&mut app, std::slice::from_ref(&notes), seen(ON_TRACK_3));
        assert_eq!(
            app.notice.as_deref(),
            Some(format!("Cannot read {}", notes.display()).as_str())
        );
        assert_eq!(document(&app), before);
        assert_eq!(app.session.history().undo_len(), 0);

        let files = [notes, dir.join("sounds/tone.wav")];
        window.drop_files(&mut app, &files, seen(ON_TRACK_3));
        assert_eq!(
            clip_of(&app, 3),
            (TrackId(3), Ticks(864), Ticks(1), Some(ChannelId(3)))
        );
        assert_eq!(app.session.history().undo_len(), 1);
    }

    // Below the last track (tracks y 400 is row 8, the "+ Track" row's, past the eight tracks)
    // the drop adds track 9, "Track 9", as "+ Track" would, and lands on it. At x 220, 100 px
    // into the timeline, tick floor(100 · 384 / 64) = 600, which the Snap's Bar floors to 384
    // (rounded to the nearest bar it would be 768). One undo takes the track, the channel and
    // the clip back.
    #[test]
    fn p8_a_drop_below_the_last_track_adds_a_track_for_it() {
        let dir = project_dir("below_the_last_track");
        let (mut app, _player, mut window) = drop_app(&dir);
        assert_eq!(app.arranger.snap, ClipSnap::Bar);
        let before = document(&app);
        let tone = dir.join("sounds/tone.wav");
        let below = Point::new(220.0, TRACKS_TOP + 400.0);
        window.drop_files(&mut app, std::slice::from_ref(&tone), seen(below));
        let tracks = &app.session.project().playlist.tracks;
        assert_eq!(tracks.len(), 9);
        assert_eq!(tracks[&TrackId(9)].name, "Track 9");
        assert_eq!(
            clip_of(&app, 3),
            (TrackId(9), Ticks(384), Ticks(1), Some(ChannelId(3)))
        );
        assert_eq!(app.session.history().undo_len(), 1);
        let _ = app.update(Message::Undo);
        assert_eq!(document(&app), before);
    }

    // Outside the lanes a drop lands nowhere: on the header strip (window y 64..88), the ruler
    // (88..116), the track headers (x 0..120) and the scroll bar (550..560) the playlist takes
    // the file, so the window is not handed it, and nothing is added. Over the toolbar, which takes no file, the window is,
    // and places it as it always has (a new audio channel at the song position).
    #[test]
    fn p8_a_drop_outside_the_lanes_lands_nowhere() {
        let dir = project_dir("outside_the_lanes");
        let (mut app, _player, mut window) = drop_app(&dir);
        let before = document(&app);
        let tone = dir.join("sounds/tone.wav");
        let files = std::slice::from_ref(&tone);
        for at in [
            Point::new(500.0, 84.0),
            Point::new(300.0, 110.0),
            Point::new(60.0, TRACKS_TOP + 114.0),
            Point::new(500.0, 555.0),
        ] {
            let published = window.drop_files(&mut app, files, seen(at));
            assert!(
                matches!(
                    published.as_slice(),
                    [Message::Media(MediaMessage::DroppedOnPlaylist { seen, .. })] if *seen == at
                ),
                "{at:?}: {published:?}"
            );
            assert_eq!(document(&app), before, "{at:?}");
            assert_eq!(app.notice, None);
        }
        let published = window.drop_files(&mut app, files, seen(Point::new(500.0, 36.0)));
        assert!(
            published
                .iter()
                .any(|message| matches!(message, Message::FileDroppedOn(_, file) if *file == tone)),
            "{published:?}"
        );
        assert_eq!(app.session.project().channels.len(), 3);
    }

    /// The app as `drop_app` makes it, its main window one the window system knows (an id the
    /// fake queries ignore) at scale 1.5, the window system's answer for the pointer `pointer`.
    fn asking_app(
        dir: &Path,
        pointer: fn(WindowId) -> Option<(i32, i32)>,
    ) -> (App, Player, Window) {
        let (mut app, player, window) = drop_app(dir);
        app.main_native = Some(1);
        app.scale_factor = 1.5;
        app.host.pointer = pointer;
        (app, player, window)
    }

    // Known answer: the window saw no cursor (as during a drag from another window) and the
    // window system says the pointer is at device pixel (405, 357) of the main window: at scale
    // 1.5 that is window point (270, 238), track 3's lane at x 270, so with the Snap on Beat the
    // clip lands on track 3 from tick 864, as above. No widget takes the drop; the window's
    // listener hands it on, and the pointer asked decides.
    #[test]
    fn p8_a_drop_lands_where_the_window_system_says_the_pointer_is() {
        let dir = project_dir("pointer_asked");
        let (mut app, _player, mut window) = asking_app(&dir, |_| Some((405, 357)));
        snap(&mut app, ClipSnap::Beat);
        let tone = dir.join("sounds/tone.wav");
        let files = std::slice::from_ref(&tone);
        let published = window.drop_files(&mut app, files, mouse::Cursor::Unavailable);
        assert!(
            matches!(published.as_slice(), [Message::FileDroppedOn(_, file)] if *file == tone),
            "{published:?}"
        );
        assert_eq!(
            clip_of(&app, 3),
            (TrackId(3), Ticks(864), Ticks(1), Some(ChannelId(3)))
        );
        assert_eq!(app.selected_channel, Some(ChannelId(3)));
    }

    // The pointer the window system gives wins over the cursor the window saw last: seen over
    // the ruler (window 300, 110), where a drop lands nowhere, while the system says device
    // pixel (330, 786), window point (220, 524) at scale 1.5: tracks y 400, row 8, below the
    // last of the eight tracks, tick 600 floored to the Bar, 384. So track 9 is added and the
    // clip lands on it from tick 384, as a drop there with the cursor seen there does.
    #[test]
    fn p8_the_window_systems_pointer_wins_over_the_cursor_seen() {
        let dir = project_dir("pointer_wins");
        let (mut app, _player, mut window) = asking_app(&dir, |_| Some((330, 786)));
        let tone = dir.join("sounds/tone.wav");
        let files = std::slice::from_ref(&tone);
        window.drop_files(&mut app, files, seen(Point::new(300.0, 110.0)));
        assert_eq!(app.session.project().playlist.tracks.len(), 9);
        assert_eq!(
            clip_of(&app, 3),
            (TrackId(9), Ticks(384), Ticks(1), Some(ChannelId(3)))
        );
    }

    // When the window system cannot say where the pointer is, or says it is outside the window
    // (device pixel (-45, 1350) is window point (-30, 900), off the 1000 × 560 window), the
    // cursor the window saw is where the drop lands: track 3 at tick 864. With neither, the file
    // goes where the window always put it: one clip from the song position, tick 0, on the
    // first track free there, track 3 (track 1 holds an audio clip over 0..768, track 2 an
    // automation clip over 0..384).
    #[test]
    fn p8_without_the_pointer_the_cursor_seen_or_the_song_position_decides() {
        let dir = project_dir("pointer_unknown");
        let tone = dir.join("sounds/tone.wav");
        let files = std::slice::from_ref(&tone);
        let unknown: fn(WindowId) -> Option<(i32, i32)> = |_| None;
        let outside: fn(WindowId) -> Option<(i32, i32)> = |_| Some((-45, 1350));
        for pointer in [unknown, outside] {
            let (mut app, _player, mut window) = asking_app(&dir, pointer);
            snap(&mut app, ClipSnap::Beat);
            window.drop_files(&mut app, files, seen(ON_TRACK_3));
            assert_eq!(
                clip_of(&app, 3),
                (TrackId(3), Ticks(864), Ticks(1), Some(ChannelId(3)))
            );
        }
        let (mut app, _player, mut window) = asking_app(&dir, unknown);
        window.drop_files(&mut app, files, mouse::Cursor::Unavailable);
        assert_eq!(
            clip_of(&app, 3),
            (TrackId(3), Ticks(0), Ticks(1), Some(ChannelId(3)))
        );
    }
}
