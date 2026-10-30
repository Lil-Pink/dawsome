//! Channel and track edits a user makes on the rack and the playlist: a channel renamed, given
//! another sound (another plugin taking the old one's parameter automations and links with it,
//! a file taking its edits: the source's, none for another file),
//! deleted (its notes in every pattern with it) and restored, cloned and its
//! clone taken out again; a track deleted with its clips and restored. Each checks everything before it changes
//! anything, so a refused edit leaves the project as it was, and each returns its exact inverse:
//! no ID counter moves on a delete, so undoing one gives the project back byte for byte, and a
//! clone's inverse gives back the ID it took.

use std::collections::BTreeSet;
use std::mem;

use super::{channel_mut, Applied, Edit, EditError, Touched};
use crate::project::{
    AutomationTarget, Channel, ChannelId, ChannelKind, Clip, ClipId, Note, PatternId,
    ProcessorOwner, Project, Sound, SoundSource, Track, TrackId, ValidationError,
};

pub(super) fn rename(
    project: &mut Project,
    id: ChannelId,
    name: String,
) -> Result<Applied, EditError> {
    let old = mem::replace(&mut channel_mut(project, id)?.name, name);
    // A name is not in the plan.
    Ok(Applied {
        inverse: Edit::RenameChannel { id, name: old },
        touched: Touched::default(),
    })
}

/// Gives channel `id` the sound `source`, named `name`, with the settings `sound`. When its
/// plugin changes (another plugin, or a sampler or a set in its place), what names the old
/// plugin's parameters goes in the same step, as one transaction: their automations with every
/// clip of them, and the controller links to them. Why: a plugin's parameter numbers are its
/// own, so they would move the new plugin's parameter of the same number (the channel's volume
/// and pan are the channel's, and stay). So does the channel's MIDI input when the new sound
/// plays no notes (an input): a port linked to it would play nothing, and `validate` refuses
/// that link. An input channel given a sound of another kind lets its MIDI out go first (only
/// an input channel sends notes out), and with it the link it took. The transaction's inverse
/// replaces the sound back before it restores them, so their target is a plugin channel, or a
/// channel that takes notes, again.
pub(super) fn replace_sound(
    project: &mut Project,
    id: ChannelId,
    name: String,
    source: SoundSource,
    sound: Sound,
) -> Result<Applied, EditError> {
    let mut untie = untie_parameters(project, id, &source);
    untie.extend(untie_midi_out(project, id, &source));
    untie.extend(untie_port(project, id, &source));
    if !untie.is_empty() {
        // The edit again goes last, when nothing names the parameters any more, so it takes the
        // path below.
        let replace = Edit::ReplaceChannelSound {
            channel: id,
            name,
            source: Box::new(source),
            sound,
        };
        return super::transaction(project, untie.into_iter().chain([replace]).collect());
    }
    let shown = channel_mut(project, id)?;
    if shown.kind == ChannelKind::Audio || source.kind == ChannelKind::Audio {
        return Err(EditError::AudioSound(id.0));
    }
    if let Some(setting) = sound.problem(source.kind) {
        return Err(EditError::InvalidSound {
            channel: id.0,
            setting,
        });
    }
    let replaced = Channel {
        name,
        kind: source.kind,
        sample: source.sample,
        processor: source.processor,
        instrument: source.instrument,
        input: source.input,
        sound,
        edits: source.edits,
        ..shown.clone()
    };
    if !replaced.sound_is_consistent() {
        return Err(ValidationError::InconsistentChannel {
            channel: replaced.name,
        }
        .into());
    }
    if let Some(problem) = replaced.edits_problem() {
        return Err(problem.into());
    }
    let plugin = replaced.processor.is_some() || shown.processor.is_some();
    let old = mem::replace(shown, replaced);
    // A plugin's state is the instance's: the plugin gone, come or another hands it over.
    let processors = if plugin {
        BTreeSet::from([ProcessorOwner::Channel(id)])
    } else {
        BTreeSet::new()
    };
    let touched = Touched {
        channels: BTreeSet::from([id]),
        processors,
        ..Touched::default()
    };
    let inverse = Edit::ReplaceChannelSound {
        channel: id,
        source: Box::new(old.source()),
        name: old.name,
        sound: old.sound,
    };
    Ok(Applied { inverse, touched })
}

/// The edit that lets channel `id`'s MIDI out go when `source` is not an input; none when it is,
/// the channel has no MIDI out, or the channel is not there.
fn untie_midi_out(project: &Project, id: ChannelId, source: &SoundSource) -> Option<Edit> {
    let shown = project.channels.get(&id)?;
    shown.midi_out.as_ref()?;
    (source.kind != ChannelKind::Input).then_some(Edit::SetChannelMidiOut {
        channel: id,
        out: None,
    })
}

/// The edit that lets channel `id`'s MIDI input go when the channel given `source` takes no
/// notes: it plays none and keeps no MIDI out (an input keeps the channel's); none when it takes
/// them, the channel holds no port, or the channel is not there.
fn untie_port(project: &Project, id: ChannelId, source: &SoundSource) -> Option<Edit> {
    let shown = project.channels.get(&id)?;
    shown.midi_input.as_ref()?;
    let keeps_midi_out = source.kind == ChannelKind::Input && shown.midi_out.is_some();
    let takes_notes = source.kind.plays_notes() || keeps_midi_out;
    (!takes_notes).then_some(Edit::SetChannelMidiInput {
        channel: id,
        port: None,
    })
}

/// The edits that take out what names channel `id`'s plugin parameters (their automations and
/// controller links) when `source`'s plugin is not the channel's; none when it is the same
/// plugin, or the channel is not there.
fn untie_parameters(project: &Project, id: ChannelId, source: &SoundSource) -> Vec<Edit> {
    let Some(shown) = project.channels.get(&id) else {
        return Vec::new();
    };
    let plugin = shown
        .processor
        .as_ref()
        .map(|processor| processor.id.as_str());
    if plugin == source.plugin_id() {
        return Vec::new();
    }
    let names_parameter = |target: &AutomationTarget| matches!(target, AutomationTarget::ChannelParam { channel, .. } if *channel == id);
    let automations = project
        .automation
        .list
        .iter()
        .filter(|(_, automation)| names_parameter(&automation.target))
        .map(|(&automation, _)| Edit::DeleteAutomationClip { automation });
    let links = project
        .midi_links
        .targets()
        .filter(|target| names_parameter(target))
        .map(|target| Edit::SetMidiLink {
            target: target.clone(),
            link: None,
        });
    automations.chain(links).collect()
}

pub(super) fn delete(project: &mut Project, id: ChannelId) -> Result<Applied, EditError> {
    if !project.channels.contains_key(&id) {
        return Err(EditError::UnknownChannel(id.0));
    }
    if project
        .playlist
        .clips
        .values()
        .any(|clip| clip.audio_channel() == Some(id))
    {
        return Err(EditError::ChannelPlaysClips(id.0));
    }
    let mut notes: Vec<(PatternId, Vec<Note>)> = Vec::new();
    let mut touched = Touched::default();
    for (&pattern, shown) in &mut project.patterns {
        let (gone, kept): (Vec<Note>, Vec<Note>) = mem::take(&mut shown.notes)
            .into_iter()
            .partition(|note| note.channel == id);
        shown.notes = kept;
        if !gone.is_empty() {
            touched.patterns.insert(pattern);
            notes.push((pattern, gone));
        }
    }
    let channel = project
        .channels
        .remove(&id)
        .ok_or(EditError::UnknownChannel(id.0))?;
    touched.channels.insert(id);
    Ok(Applied {
        inverse: Edit::RestoreChannel {
            id,
            channel: Box::new(channel),
            notes,
        },
        touched,
    })
}

pub(super) fn restore(
    project: &mut Project,
    id: ChannelId,
    channel: Channel,
    notes: Vec<(PatternId, Vec<Note>)>,
) -> Result<Applied, EditError> {
    if project.channels.contains_key(&id) || id >= project.next_channel_id {
        return Err(EditError::ChannelExists(id.0));
    }
    if let Some((pattern, _)) = notes
        .iter()
        .find(|(pattern, _)| !project.patterns.contains_key(pattern))
    {
        return Err(EditError::UnknownPattern(pattern.0));
    }
    let mut touched = Touched::default();
    for (pattern, restored) in notes {
        let shown = project
            .patterns
            .get_mut(&pattern)
            .ok_or(EditError::UnknownPattern(pattern.0))?;
        shown.notes.extend(restored);
        shown.notes.sort_by_key(Note::order_key);
        touched.patterns.insert(pattern);
    }
    project.channels.insert(id, channel);
    touched.channels.insert(id);
    Ok(Applied {
        inverse: Edit::DeleteChannel { id },
        touched,
    })
}

pub(super) fn clone(
    project: &mut Project,
    id: ChannelId,
    from: ChannelId,
) -> Result<Applied, EditError> {
    if id != project.next_channel_id || project.channels.contains_key(&id) {
        return Err(EditError::NotNextChannelId {
            got: id.0,
            expected: project.next_channel_id.0,
        });
    }
    let source = project
        .channels
        .get(&from)
        .ok_or(EditError::UnknownChannel(from.0))?;
    let next = id.0.checked_add(1).ok_or(EditError::ChannelIdsExhausted)?;
    // A device plays one channel at most: the clone plays the selected one's notes alone, and
    // sends none to the source's MIDI out, which would play every note twice.
    let copy = Channel {
        name: clone_name(&source.name),
        midi_input: None,
        midi_out: None,
        ..source.clone()
    };
    project.channels.insert(id, copy);
    project.next_channel_id = ChannelId(next);
    Ok(Applied {
        inverse: Edit::Unclone { id, from },
        touched: Touched {
            channels: [id].into(),
            ..Touched::default()
        },
    })
}

pub(super) fn unclone(
    project: &mut Project,
    id: ChannelId,
    from: ChannelId,
) -> Result<Applied, EditError> {
    let newest = id.0.checked_add(1) == Some(project.next_channel_id.0);
    let has_notes = project
        .patterns
        .values()
        .any(|pattern| pattern.notes.iter().any(|note| note.channel == id));
    if !newest || has_notes || !project.channels.contains_key(&id) {
        return Err(EditError::ChannelNotRemovable(id.0));
    }
    project.channels.remove(&id);
    project.next_channel_id = id;
    Ok(Applied {
        inverse: Edit::CloneChannel { id, from },
        touched: Touched {
            channels: [id].into(),
            ..Touched::default()
        },
    })
}

/// A clone's name: the source's with " #2", or the number after its " #" raised by one.
fn clone_name(name: &str) -> String {
    match name
        .rsplit_once(" #")
        .map(|(stem, number)| (stem, number.parse::<u32>()))
    {
        Some((stem, Ok(number))) => format!("{stem} #{}", number.saturating_add(1)),
        _ => format!("{name} #2"),
    }
}

pub(super) fn delete_track(project: &mut Project, id: TrackId) -> Result<Applied, EditError> {
    let playlist = &mut project.playlist;
    if !playlist.tracks.contains_key(&id) {
        return Err(EditError::UnknownTrack(id.0));
    }
    let gone: Vec<ClipId> = playlist
        .clips
        .iter()
        .filter(|(_, clip)| clip.track == id)
        .map(|(&clip, _)| clip)
        .collect();
    let clips: Vec<(ClipId, Clip)> = gone
        .into_iter()
        .filter_map(|clip| Some((clip, playlist.clips.remove(&clip)?)))
        .collect();
    let track = playlist
        .tracks
        .remove(&id)
        .ok_or(EditError::UnknownTrack(id.0))?;
    Ok(Applied::playlist(Edit::RestoreTrack { id, track, clips }))
}

pub(super) fn restore_track(
    project: &mut Project,
    id: TrackId,
    track: Track,
    clips: Vec<(ClipId, Clip)>,
) -> Result<Applied, EditError> {
    let playlist = &mut project.playlist;
    if playlist.tracks.contains_key(&id) || id >= playlist.next_track_id {
        return Err(EditError::TrackExists(id.0));
    }
    if let Some((clip, _)) = clips
        .iter()
        .find(|(clip, _)| playlist.clips.contains_key(clip))
    {
        return Err(EditError::ClipExists(clip.0));
    }
    playlist.tracks.insert(id, track);
    playlist.clips.extend(clips);
    Ok(Applied::playlist(Edit::DeleteTrack { id }))
}

#[cfg(test)]
mod tests {
    use super::clone_name;

    #[test]
    fn a_clone_counts_up_its_name() {
        assert_eq!(clone_name("Kick"), "Kick #2");
        assert_eq!(clone_name("Kick #2"), "Kick #3");
        assert_eq!(clone_name("Bass 808"), "Bass 808 #2");
        assert_eq!(clone_name("Odd #name"), "Odd #name #2");
    }
}
