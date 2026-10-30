//! Automation from the controls: a right press on a rack knob, a mixer fader, pan or send knob,
//! an effect parameter's slider, or a plugin parameter's row (in the parameter window or the
//! mixer's slot panel, by the plugin's own id for it) offers "Create automation clip", or
//! "Show automation clip" when the control has an automation already, and under it the
//! controller link row (see
//! `controllers`); while an automation governs a control, the control shows the automation's
//! value, and every automated control carries a dot.

use std::collections::{BTreeMap, BTreeSet};

use dawsome_engine::edit::Edit;
use dawsome_engine::project::{
    AutomationPoint, AutomationTarget, ProcessorOwner, Project, TrackId,
};
use dawsome_engine::time::{self, Ticks};
use iced::widget::{column, pin};
use iced::{Element, Point};

use super::{App, Message};
use crate::ui::menu;
use crate::ui::note_gesture::snap_down;
use crate::ui::panels::Panel;

/// How long a new automation clip is, and where its second point sits: four bars.
const NEW_CLIP_BARS: u32 = 4;

/// The menu's width: its longest row, "Create automation clip", and a margin.
const MENU_WIDTH: f32 = 180.0;

/// The menu a right press on a control opens: what it would automate, the control's value now
/// (normalized over its range), and where the press was, in the coordinates of the panel's
/// canvas (of the parameter window's list, under the row's slider, when it is open there).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AutomateMenu {
    pub target: AutomationTarget,
    pub value: f32,
    pub panel: Panel,
    /// Open over the parameter window's list rather than on `panel`.
    pub over_params: bool,
    pub at: Point,
}

impl AutomateMenu {
    /// The menu for `target`, `value` of the way along its range, at `at` on `panel`'s canvas.
    fn new(target: AutomationTarget, value: f32, panel: Panel, at: Point) -> Self {
        Self {
            target,
            value: value.clamp(0.0, 1.0),
            panel,
            over_params: false,
            at,
        }
    }
}

/// The automated controls, each with the value its automation set in the player's last
/// buffer while it governs (`None` while it does not).
pub(super) type Automated = BTreeMap<AutomationTarget, Option<f32>>;

impl App {
    /// Opens the automation menu for `target`, whose value is `value` of the way along its
    /// range, at `at` on `panel`'s canvas.
    pub(super) fn open_automate_menu(
        &mut self,
        target: AutomationTarget,
        value: f32,
        panel: Panel,
        at: Point,
    ) {
        self.automate = Some(AutomateMenu::new(target, value, panel, at));
    }

    /// Opens the automation menu for row `index` of `owner`'s plugin (among its parameters
    /// that are not hidden), by the plugin's own id for it, at `at`: on `panel`'s canvas, or
    /// over the parameter window's list when `over_params`. A read-only parameter has none.
    pub(super) fn open_plugin_automate_menu(
        &mut self,
        (owner, index): (ProcessorOwner, usize),
        panel: Panel,
        over_params: bool,
        at: Point,
    ) {
        let Some(drag) = self.plugin_drag(owner, index) else {
            return;
        };
        let Some(value) = self.plugin_value(drag) else {
            return;
        };
        let value = drag.fraction(value);
        let target = match owner {
            ProcessorOwner::Channel(channel) => AutomationTarget::ChannelParam {
                channel,
                param_id: drag.id,
            },
            ProcessorOwner::Effect(effect) => AutomationTarget::EffectPlugin {
                effect,
                param_id: drag.id,
            },
        };
        self.automate = Some(AutomateMenu {
            over_params,
            ..AutomateMenu::new(target, value, panel, at)
        });
    }

    /// The menu's one row: shows the control's automation clip, or creates one.
    pub(super) fn automate_chosen(&mut self) {
        let Some(menu) = self.automate.take() else {
            return;
        };
        let project = self.session.project();
        match project.automation.of_target(&menu.target) {
            Some(id) => self.show_automation(id),
            None => {
                let edit = new_automation_clip(project, menu.target, menu.value, self.song_marker);
                self.commit(edit);
            }
        }
    }

    /// Opens the playlist on the first clip of automation `id`, selected.
    fn show_automation(&mut self, id: dawsome_engine::project::AutomationId) {
        let project = self.session.project();
        let first = project
            .playlist
            .clips
            .iter()
            .filter(|(_, clip)| clip.automation() == Some(id))
            .min_by_key(|(&clip_id, clip)| (clip.start, clip_id));
        let Some((&clip_id, clip)) = first else {
            return;
        };
        let index = project
            .playlist
            .tracks
            .keys()
            .position(|&track| track == clip.track);
        let bar = time::bars_to_ticks(1, project.ppq);
        self.arranger.grid.scroll = Ticks(snap_down(clip.start, bar).0.max(0));
        if let Some(index) = index {
            self.arranger.grid.top = index;
        }
        self.arranger.selection = BTreeSet::from([clip_id]);
        let opened = !self.panels.playlist;
        self.panels.playlist = true;
        self.focus = Panel::Playlist;
        if opened {
            self.make_room_for(Panel::Playlist);
        }
    }

    /// The menu over `panel`'s canvas, when it is open there.
    pub(super) fn automate_menu_view(&self, panel: Panel) -> Option<Element<'_, Message>> {
        let menu = self
            .automate
            .as_ref()
            .filter(|menu| menu.panel == panel && !menu.over_params)?;
        Some(self.menu_rows(menu, self.hint_bar_height()))
    }

    /// The menu over the parameter window's list, when it is open there, keeping clear of the
    /// window's last `foot` pixels (its foot line).
    pub(super) fn params_automate_menu_view(&self, foot: f32) -> Option<Element<'_, Message>> {
        let menu = self.automate.as_ref().filter(|menu| menu.over_params)?;
        Some(self.menu_rows(menu, foot))
    }

    /// The menu's rows, opened at its place on the window's top layer, keeping clear of the
    /// window's last `foot` pixels.
    fn menu_rows(&self, menu: &AutomateMenu, foot: f32) -> Element<'_, Message> {
        let automated = self
            .session
            .project()
            .automation
            .of_target(&menu.target)
            .is_some();
        let label = if automated {
            "Show automation clip"
        } else {
            "Create automation clip"
        };
        let rows = column![
            menu::row(label, Some(Message::AutomateChosen)),
            menu::row(self.link_row_label(&menu.target), Some(Message::LinkChosen)),
        ];
        pin(menu::floating(menu::panel(rows, MENU_WIDTH), foot))
            .x(menu.at.x)
            .y(menu.at.y)
            .into()
    }

    /// Reads each automation's value from the player's last buffer; the panels redraw when
    /// one moved.
    pub(super) fn read_automation(&mut self) {
        let project = self.session.project();
        let automated: Automated = project
            .automation
            .list
            .iter()
            .map(|(&id, automation)| (automation.target.clone(), self.session.automation_value(id)))
            .collect();
        if automated != self.automated {
            self.automated = automated;
            self.rack_cache.clear();
            self.mixer_cache.clear();
        }
    }
}

/// The edit that adds an automation of `target` holding `value` (normalized): one clip of four
/// bars, with a point at each end, from the bar `marker` is in, on the first track below the
/// last one with any clip on it that holds no clip at all (a new track added with it when there
/// is none).
pub(super) fn new_automation_clip(
    project: &Project,
    target: AutomationTarget,
    value: f32,
    marker: Ticks,
) -> Edit {
    let ppq = project.ppq;
    let length = time::bars_to_ticks(NEW_CLIP_BARS, ppq);
    let start = Ticks(snap_down(marker, time::bars_to_ticks(1, ppq)).0.max(0));
    let (track, add_track) = match free_track_below_clips(project) {
        Some(track) => (track, None),
        None => {
            let id = project.playlist.next_track_id;
            let add = Edit::AddTrack {
                id,
                name: format!("Track {}", id.0),
            };
            (id, Some(add))
        }
    };
    let add = Edit::AddAutomationClip {
        automation: project.automation.next_id,
        clip: project.playlist.next_clip_id,
        target,
        track,
        start,
        length,
        points: vec![
            AutomationPoint::linear(Ticks(0), value),
            AutomationPoint::linear(length, value),
        ],
    };
    match add_track {
        Some(track) => Edit::Transaction(vec![track, add]),
        None => add,
    }
}

/// The first track below the last track holding any clip that holds no clip itself; the first
/// track when no track holds one.
fn free_track_below_clips(project: &Project) -> Option<TrackId> {
    let playlist = &project.playlist;
    let used: BTreeSet<TrackId> = playlist.clips.values().map(|clip| clip.track).collect();
    let tracks: Vec<TrackId> = playlist.tracks.keys().copied().collect();
    let below = tracks
        .iter()
        .rposition(|track| used.contains(track))
        .map_or(0, |last| last + 1);
    tracks[below.min(tracks.len())..]
        .iter()
        .copied()
        .find(|track| !used.contains(track))
}

#[cfg(test)]
mod tests {
    use dawsome_engine::project::ChannelId;

    use super::*;

    fn p8_ui() -> Project {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/p8_ui.json"
        );
        Project::load_json(&std::fs::read_to_string(path).expect("reads")).expect("loads")
    }

    // p8_ui holds clips on tracks 1 and 2 of 8: a new automation clip goes on track 3, from
    // the bar the marker (tick 500) is in, bar 2 at tick 384, for 1536 ticks, its two points
    // at the control's value.
    #[test]
    fn a_new_automation_clip_goes_below_the_last_used_track() {
        let project = p8_ui();
        let target = AutomationTarget::ChannelPan(ChannelId(1));
        let edit = new_automation_clip(&project, target.clone(), 0.5, Ticks(500));
        assert_eq!(
            edit,
            Edit::AddAutomationClip {
                automation: dawsome_engine::project::AutomationId(2),
                clip: dawsome_engine::project::ClipId(3),
                target,
                track: TrackId(3),
                start: Ticks(384),
                length: Ticks(1536),
                points: vec![
                    AutomationPoint::linear(Ticks(0), 0.5),
                    AutomationPoint::linear(Ticks(1536), 0.5),
                ],
            }
        );
    }

    // With every track below the last used one taken (here: the last track holds a clip), a
    // track is added in the same step.
    #[test]
    fn with_no_free_track_below_one_is_added() {
        let mut project = p8_ui();
        project.playlist.tracks.retain(|id, _| id.0 <= 2);
        let edit = new_automation_clip(
            &project,
            AutomationTarget::ChannelPan(ChannelId(1)),
            0.0,
            Ticks(0),
        );
        let Edit::Transaction(edits) = edit else {
            panic!("a track and a clip");
        };
        assert!(matches!(edits[0], Edit::AddTrack { id: TrackId(9), .. }));
        assert!(matches!(
            edits[1],
            Edit::AddAutomationClip {
                track: TrackId(9),
                ..
            }
        ));
    }
}
