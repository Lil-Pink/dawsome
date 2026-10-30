//! The rack's rows: one per channel in the rack's order, but the audio takes (audio channels
//! "Take N" playing their take's "<project> take N.wav", one per take's file, since an audio
//! clip plays its channel's file) under one
//! header, Recordings, after the other channels, folded unless opened. Why: each audio take
//! needs a channel of its own in this model, and a row each would bury the instruments under
//! takes. A project with no take has exactly the rows it always had.

use dawsome_engine::project::{Channel, ChannelId, ChannelKind, Project};

use crate::record;

/// One row of the rack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RackRow {
    Channel(ChannelId),
    /// The Recordings header over `count` audio takes, opened or folded.
    Takes {
        count: usize,
        open: bool,
    },
}

/// The rack's rows, top to bottom.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RackRows {
    rows: Vec<RackRow>,
}

/// Whether `channel` is an audio take's, listed under Recordings: an audio channel "Take N"
/// playing the "<project> take N.wav" its take wrote ([`record::is_take`]).
pub fn is_take(channel: &Channel) -> bool {
    channel.kind == ChannelKind::Audio && record::is_take(&channel.name, &channel.sample)
}

impl RackRows {
    /// The rows of `project`'s rack, the takes shown under their header when `takes_open`.
    pub fn of(project: &Project, takes_open: bool) -> Self {
        let mut rows: Vec<RackRow> = Vec::with_capacity(project.channels.len() + 1);
        let mut takes = Vec::new();
        for (&id, channel) in &project.channels {
            if is_take(channel) {
                takes.push(RackRow::Channel(id));
            } else {
                rows.push(RackRow::Channel(id));
            }
        }
        if !takes.is_empty() {
            rows.push(RackRow::Takes {
                count: takes.len(),
                open: takes_open,
            });
            if takes_open {
                rows.extend(takes);
            }
        }
        Self { rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Row `row`, when there is one.
    pub fn get(&self, row: usize) -> Option<RackRow> {
        self.rows.get(row).copied()
    }

    /// The channel of row `row`; none for the header or past the last row.
    pub fn channel(&self, row: usize) -> Option<ChannelId> {
        match self.get(row)? {
            RackRow::Channel(id) => Some(id),
            RackRow::Takes { .. } => None,
        }
    }

    /// The row channel `id` is shown on; none when it is folded away or not in the rack.
    pub fn row_of(&self, id: ChannelId) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| *row == RackRow::Channel(id))
    }

    /// The channels' rows, top to bottom, with their row numbers.
    pub fn channels(&self) -> impl Iterator<Item = (usize, ChannelId)> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(row, item)| match *item {
                RackRow::Channel(id) => Some((row, id)),
                RackRow::Takes { .. } => None,
            })
    }

    /// The Recordings header's row, while there are takes.
    pub fn takes_header(&self) -> Option<(usize, usize, bool)> {
        self.rows
            .iter()
            .enumerate()
            .find_map(|(row, item)| match *item {
                RackRow::Takes { count, open } => Some((row, count, open)),
                RackRow::Channel(_) => None,
            })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dawsome_engine::edit::{self, Edit};
    use dawsome_engine::project::TrackId;
    use dawsome_engine::time::Ticks;

    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture(name: &str) -> Project {
        let text =
            std::fs::read_to_string(std::path::Path::new(FIXTURES).join(name)).expect("reads");
        Project::load_json(&text).expect("loads")
    }

    /// `project` with an audio channel `name` playing `sample` (and its clip) added.
    fn with_audio(project: &mut Project, name: &str, sample: &str) -> ChannelId {
        let channel = project.next_channel_id;
        edit::apply(
            project,
            Edit::AddAudioClip {
                channel,
                name: name.to_owned(),
                sample: PathBuf::from(sample),
                clip: project.playlist.next_clip_id,
                track: TrackId(2),
                start: Ticks(0),
                length: Ticks(96),
            },
        )
        .expect("adds");
        channel
    }

    // A rack with no take: one row per channel, in order, as it always was; an audio channel
    // of another name (a file dropped) keeps its row.
    #[test]
    fn without_takes_every_channel_has_its_row() {
        let mut project = fixture("p17_audio48.json");
        let dropped = with_audio(&mut project, "kick loop", "kick loop.wav");
        let rows = RackRows::of(&project, false);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.channel(0), Some(ChannelId(1)));
        assert_eq!(rows.channel(1), Some(dropped));
        assert_eq!(rows.takes_header(), None);
    }

    // Takes 1 and 2 (channels 2 and 3) with an instrument added after them (channel 4): the
    // instrument's row comes before the header, the takes fold under it; opened, they follow it.
    #[test]
    fn takes_fold_under_one_header_after_the_channels() {
        let mut project = fixture("p17_audio48.json");
        let one = with_audio(&mut project, "Take 1", "samples/song take 1.wav");
        let two = with_audio(&mut project, "Take 2", "samples/song take 2.wav");
        let id = project.next_channel_id;
        edit::apply(
            &mut project,
            Edit::AddSamplerChannel {
                id,
                name: "Snare".into(),
                sample: PathBuf::new(),
            },
        )
        .expect("adds");
        let folded = RackRows::of(&project, false);
        assert_eq!(folded.len(), 3);
        assert_eq!(
            (folded.channel(0), folded.channel(1)),
            (Some(ChannelId(1)), Some(id))
        );
        assert_eq!(
            folded.get(2),
            Some(RackRow::Takes {
                count: 2,
                open: false
            })
        );
        assert_eq!((folded.row_of(one), folded.row_of(id)), (None, Some(1)));
        let open = RackRows::of(&project, true);
        assert_eq!(open.len(), 5);
        assert_eq!((open.channel(3), open.channel(4)), (Some(one), Some(two)));
        assert_eq!(open.takes_header(), Some((2, 2, true)));
        assert_eq!(
            open.channels().map(|(row, _)| row).collect::<Vec<_>>(),
            [0, 1, 3, 4]
        );
    }

    // An audio channel the user named "Take 2" (a file "Take 2.wav"
    // dropped, or a channel renamed) is no take: it keeps its row. A take is "Take N" playing
    // "<project> take N.wav", the file its take wrote; a number that does not match is none.
    #[test]
    fn a_channel_named_like_a_take_keeps_its_row() {
        let mut project = fixture("p17_audio48.json");
        let named = with_audio(&mut project, "Take 2", "Take 2.wav");
        let other = with_audio(&mut project, "Take 3", "samples/song take 1.wav");
        let take = with_audio(
            &mut project,
            "Take 1",
            "dawsome:recordings/Untitled take 1.wav",
        );
        let rows = RackRows::of(&project, false);
        assert_eq!(
            (rows.row_of(named), rows.row_of(other), rows.row_of(take)),
            (Some(1), Some(2), None)
        );
        assert_eq!(rows.takes_header(), Some((3, 1, false)));
    }
}
