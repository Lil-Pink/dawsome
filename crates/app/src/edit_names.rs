//! What each edit is called where the window names a step of the history: the Edit menu's
//! "Undo <name>" and "Redo <name>". One table, so every kind of edit has its name in one place,
//! and a new kind fails to compile here until it has one.

use dawsome_engine::edit::Edit;
use dawsome_engine::project::ChannelKind;

/// The name of what applying `edit` does, as a short phrase starting with a capital.
///
/// A transaction is named after the first edit in it that is not one of those made only to make
/// room for it (a track added, clips deleted first): a drop of a set on a track adds a channel
/// and places its clip ("Add channel"); deleting a pattern deletes its clips first ("Delete
/// pattern").
pub fn name(edit: &Edit) -> &'static str {
    let plural = |count: usize, one: &'static str, many: &'static str| {
        if count == 1 {
            one
        } else {
            many
        }
    };
    match edit {
        Edit::ToggleStep { .. } => "Step",
        Edit::ReplaceNotes { remove, add, .. } => match (remove.len(), add.len()) {
            (0, added) => plural(added, "Add note", "Add notes"),
            (removed, 0) => plural(removed, "Delete note", "Delete notes"),
            (removed, _) => plural(removed, "Change note", "Change notes"),
        },
        Edit::SetChannelVolume { .. } => "Channel volume",
        Edit::SetChannelPan { .. } => "Channel pan",
        Edit::SetChannelMute { value: true, .. } => "Mute channel",
        Edit::SetChannelMute { value: false, .. } => "Unmute channel",
        Edit::SetChannelSample { .. } => "Change sample",
        Edit::SetChannelSound { .. } => "Sound settings",
        // One op appended is named after it; the list taken back to none is a revert.
        Edit::SetChannelEdits { edits, .. } => {
            edits.ops.last().map_or("Revert sound", |op| op.name())
        }
        Edit::SetTempo { .. } => "Tempo",
        Edit::SetPatternBars { .. } => "Pattern length",
        Edit::AddPattern { .. } => "New pattern",
        Edit::UnaddPattern { .. } => "Remove pattern",
        Edit::DeletePattern { .. } => "Delete pattern",
        Edit::RestorePattern { .. } => "Restore pattern",
        Edit::RenamePattern { .. } => "Rename pattern",
        Edit::AddSamplerChannel { .. }
        | Edit::AddPluginChannel { .. }
        | Edit::AddInstrumentChannel { .. }
        | Edit::AddInputChannel { .. } => "Add channel",
        Edit::RemoveChannel { .. } => "Remove channel",
        Edit::SetProcessorState { .. } => "Plugin settings",
        Edit::PlaceClips { clips, .. } => plural(clips.len(), "Place clip", "Place clips"),
        Edit::UnplaceClips { .. } => "Remove clips",
        Edit::DuplicateClips { .. } => "Duplicate clips",
        Edit::DeleteClips { ids } => plural(ids.len(), "Delete clip", "Delete clips"),
        Edit::RestoreClips { .. } => "Restore clips",
        Edit::MoveClips { moves } => plural(moves.len(), "Move clip", "Move clips"),
        Edit::ResizeClip { .. } => "Resize clip",
        Edit::SetLoop { value: None } => "Remove loop",
        Edit::SetLoop { value: Some(_) } => "Loop",
        Edit::AddAudioClip { .. } => "Add audio clip",
        Edit::UnaddAudioClip { .. } => "Remove audio clip",
        Edit::TrimAudioClip { .. } => "Trim clip",
        Edit::SetClipGain { .. } => "Clip gain",
        Edit::SetClipStretch { .. } => "Stretch clip",
        Edit::SetMidiLink { link: Some(_), .. } => "Link controller",
        Edit::SetMidiLink { link: None, .. } => "Unlink controller",
        Edit::SetChannelMidiInput { port: Some(_), .. } => "Link MIDI input",
        Edit::SetChannelMidiInput { port: None, .. } => "Unlink MIDI input",
        Edit::SetChannelMidiOut { .. } => "Change MIDI out",
        // Alone it is a link back that keeps the copy another clip plays (`copies::link_back`).
        Edit::SetClipPattern { .. } => "Link clip back",
        Edit::SetClipMute { muted: true, .. } => "Mute clips",
        Edit::SetClipMute { muted: false, .. } => "Unmute clips",
        Edit::AddAutomationClip { .. } => "Add automation",
        Edit::UnaddAutomationClip { .. } => "Remove automation",
        Edit::DeleteAutomationClip { .. } => "Delete automation",
        Edit::RestoreAutomationClip { .. } => "Restore automation",
        Edit::AddAutomationPoint { .. } => "Add point",
        Edit::MoveAutomationPoint { .. } => "Move point",
        Edit::SetSegmentCurve { .. } => "Curve",
        Edit::DeleteAutomationPoint { .. } => "Delete point",
        Edit::AddTrack { .. } => "Add track",
        Edit::RenameChannel { .. } => "Rename channel",
        Edit::ReplaceChannelSound { source, .. } if source.kind == ChannelKind::Input => {
            "Change input"
        }
        Edit::ReplaceChannelSound { .. } => "Replace sound",
        Edit::DeleteChannel { .. } => "Delete channel",
        Edit::RestoreChannel { .. } => "Restore channel",
        Edit::CloneChannel { .. } => "Clone channel",
        Edit::Unclone { .. } => "Remove clone",
        Edit::DeleteTrack { .. } => "Delete track",
        Edit::RestoreTrack { .. } => "Restore track",
        Edit::RemoveTrack { .. } => "Remove track",
        Edit::RenameTrack { .. } => "Rename track",
        Edit::SetTrackMute { mute: true, .. } => "Mute track",
        Edit::SetTrackMute { mute: false, .. } => "Unmute track",
        Edit::SetInsertVolume { .. } => "Mixer volume",
        Edit::SetInsertPan { .. } => "Mixer pan",
        Edit::MuteInsert { mute: true, .. } => "Mute insert",
        Edit::MuteInsert { mute: false, .. } => "Unmute insert",
        Edit::RenameInsert { .. } => "Rename insert",
        Edit::SoloInsert { .. } => "Solo",
        Edit::RouteChannel { .. } => "Route channel",
        Edit::SetSend { .. } => "Send",
        Edit::AddEffect { .. } => "Add effect",
        Edit::UnaddEffect { .. } | Edit::RemoveEffect { .. } => "Remove effect",
        Edit::RestoreEffect { .. } => "Restore effect",
        Edit::SetEffectParam { .. } => "Effect setting",
        Edit::MoveEffect { .. } => "Move effect",
        // Clips given their own copies (each copy added, its clip pointed at it) or linked
        // back, one or several (the playlist header's link control acts on the selection). Why
        // no count in the name ("Unlink 3 clips"): the history keeps names as `&'static str`.
        Edit::Transaction(edits) if edits.iter().any(points_a_clip) => {
            let clips = edits.iter().filter(|edit| points_a_clip(edit)).count();
            if edits
                .iter()
                .any(|edit| matches!(edit, Edit::AddPattern { .. }))
            {
                plural(clips, "Unlink clip", "Unlink clips")
            } else {
                plural(clips, "Link clip back", "Link clips back")
            }
        }
        Edit::Transaction(edits) => edits
            .iter()
            .find(|edit| !makes_room(edit))
            .or(edits.first())
            .map_or("Edit", name),
    }
}

/// Whether `edit` gives a clip another pattern: what unlinking a clip and linking it back do.
fn points_a_clip(edit: &Edit) -> bool {
    matches!(edit, Edit::SetClipPattern { .. })
}

/// Whether `edit` is one a transaction makes only to make room for the edit it is for.
fn makes_room(edit: &Edit) -> bool {
    matches!(edit, Edit::AddTrack { .. } | Edit::DeleteClips { .. })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dawsome_engine::project::{ChannelId, ClipId, Note, PatternId, TrackId};
    use dawsome_engine::time::Ticks;

    use super::*;

    fn note(key: u8) -> Note {
        Note {
            channel: ChannelId(1),
            start: Ticks(0),
            length: Ticks(24),
            velocity: 0.78125,
            key,
        }
    }

    // A note gesture is named by what it adds and removes; a transaction by the edit it is for,
    // past those that make room for it.
    #[test]
    fn edits_are_named_by_what_they_do() {
        let replace = |remove: Vec<Note>, add: Vec<Note>| Edit::ReplaceNotes {
            pattern: PatternId(1),
            remove,
            add,
        };
        assert_eq!(name(&replace(vec![], vec![note(60)])), "Add note");
        assert_eq!(
            name(&replace(vec![], vec![note(60), note(64)])),
            "Add notes"
        );
        assert_eq!(name(&replace(vec![note(60)], vec![])), "Delete note");
        assert_eq!(
            name(&replace(vec![note(60)], vec![note(62)])),
            "Change note"
        );
        assert_eq!(
            name(&Edit::DeleteChannel { id: ChannelId(2) }),
            "Delete channel"
        );
        let add = Edit::AddInstrumentChannel {
            id: ChannelId(3),
            name: "Piano".into(),
            instrument: PathBuf::from("piano.sfz"),
        };
        let place = Edit::PlaceClips {
            first: ClipId(1),
            clips: vec![],
        };
        assert_eq!(name(&Edit::Transaction(vec![add, place])), "Add channel");
        let delete = Edit::Transaction(vec![
            Edit::DeleteClips {
                ids: vec![ClipId(1)],
            },
            Edit::DeletePattern { id: PatternId(2) },
        ]);
        assert_eq!(name(&delete), "Delete pattern");
        let track = Edit::AddTrack {
            id: TrackId(9),
            name: "Track 9".into(),
        };
        assert_eq!(name(&Edit::Transaction(vec![track.clone()])), "Add track");
        assert_eq!(name(&Edit::Transaction(vec![])), "Edit");
        // Mute and unmute are told apart.
        let mute = |value| Edit::SetChannelMute {
            channel: ChannelId(1),
            value,
        };
        assert_eq!(
            (name(&mute(true)), name(&mute(false))),
            ("Mute channel", "Unmute channel")
        );
    }
}
