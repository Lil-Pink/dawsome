//! MIDI link edits: a parameter linked to a MIDI controller, moved to another, or unlinked; a
//! channel linked to the notes of a MIDI input port; and an input channel's MIDI out. Each checks
//! everything before it changes anything and returns its exact inverse.

use std::collections::BTreeSet;

use super::{Applied, Edit, EditError, Touched};
use crate::project::{
    same_port, AutomationTarget, ChannelId, ChannelKind, Controller, MidiOut, Project,
    ValidationError, MAX_CONTROLLER, MIDI_CHANNELS,
};

/// Links `target` to `link`, or unlinks it (`None`). A controller already linked to another
/// parameter moves to this one, so one controller always moves one parameter; the inverse then
/// puts both back.
///
/// Links touch nothing the player reads: a controller's moves reach the player as live values,
/// so no compile follows.
pub(super) fn set(
    project: &mut Project,
    target: AutomationTarget,
    link: Option<Controller>,
) -> Result<Applied, EditError> {
    if let Some(controller) = link {
        let Controller { cc, channel } = controller;
        if cc > MAX_CONTROLLER || channel.is_some_and(|channel| channel >= MIDI_CHANNELS) {
            return Err(ValidationError::InvalidMidiLink { cc, channel }.into());
        }
        if !project.has_target(&target) {
            return Err(EditError::UnknownTarget);
        }
    }
    let links = &mut project.midi_links;
    // The parameter this controller moves now, unless it is this one.
    let moved_from = link
        .and_then(|controller| links.target_of(controller))
        .filter(|other| **other != target)
        .cloned();
    if let Some(other) = &moved_from {
        links.remove(other);
    }
    let old = match link {
        Some(controller) => links.insert(target.clone(), controller),
        None => links.remove(&target),
    };
    let restore = Edit::SetMidiLink { target, link: old };
    let inverse = match moved_from {
        // This parameter first, so the controller is free again when the other takes it back.
        Some(other) => Edit::Transaction(vec![
            restore,
            Edit::SetMidiLink {
                target: other,
                link,
            },
        ]),
        None => restore,
    };
    Ok(Applied {
        inverse,
        touched: Touched::default(),
    })
}

/// Links `channel` to MIDI input `port`, or unlinks it (`None`). A port linked to another
/// channel (compared by [`stable_port_name`]) moves to this one, so one port always plays one
/// channel; the inverse then puts both back. `port` is stored as given: the app gives the stable
/// name, and an inverse gives back the name that was there.
///
/// [`stable_port_name`]: crate::project::stable_port_name
///
/// Like a controller link it touches nothing the player reads: the window routes each port's
/// notes to their channel as they arrive, so no compile follows.
pub(super) fn set_input(
    project: &mut Project,
    channel: ChannelId,
    port: Option<String>,
) -> Result<Applied, EditError> {
    let target = project
        .channels
        .get(&channel)
        .ok_or(EditError::UnknownChannel(channel.0))?;
    if port.is_some() && !target.takes_midi_input() {
        return Err(EditError::PlaysNoNotes(channel.0));
    }
    if port.as_deref() == Some("") {
        return Err(EditError::EmptyPortName);
    }
    // The channel this port plays now, unless it is this one: it lets the port go, and the
    // inverse gives it back the port's name as it held it.
    let moved_from = port.as_deref().and_then(|port| {
        project.channels.iter_mut().find_map(|(&id, other)| {
            let holds = other
                .midi_input
                .as_deref()
                .is_some_and(|linked| same_port(linked, port));
            (id != channel && holds).then(|| (id, other.midi_input.take()))
        })
    });
    let target = project
        .channels
        .get_mut(&channel)
        .ok_or(EditError::UnknownChannel(channel.0))?;
    let old = std::mem::replace(&mut target.midi_input, port.clone());
    let restore = Edit::SetChannelMidiInput { channel, port: old };
    let inverse = match moved_from {
        // This channel first, so the port is free again when the other takes it back.
        Some((other, held)) => Edit::Transaction(vec![
            restore,
            Edit::SetChannelMidiInput {
                channel: other,
                port: held,
            },
        ]),
        None => restore,
    };
    Ok(Applied {
        inverse,
        touched: Touched::default(),
    })
}

/// Gives input channel `channel` the MIDI out `out`, changes it, or takes it away (`None`).
/// Taken away from a channel linked to a MIDI input, the link goes first, in one transaction:
/// the channel then takes no notes, and `validate` refuses a link on it. The transaction's
/// inverse gives the MIDI out back before the link.
///
/// It touches the channel: the plan carries its MIDI channel and latency.
pub(super) fn set_output(
    project: &mut Project,
    channel: ChannelId,
    out: Option<MidiOut>,
) -> Result<Applied, EditError> {
    let target = project
        .channels
        .get(&channel)
        .ok_or(EditError::UnknownChannel(channel.0))?;
    if let Some(midi_out) = &out {
        if target.kind != ChannelKind::Input {
            return Err(EditError::NoMidiOut(channel.0));
        }
        if let Some(setting) = midi_out.problem() {
            return Err(EditError::InvalidMidiOut {
                channel: channel.0,
                setting,
            });
        }
    }
    let unlinks = out.is_none() && !target.plays_notes() && target.midi_input.is_some();
    if unlinks {
        // The edit again goes last, when nothing links the channel any more, so it takes the
        // path below.
        let edits = vec![
            Edit::SetChannelMidiInput {
                channel,
                port: None,
            },
            Edit::SetChannelMidiOut { channel, out },
        ];
        return super::transaction(project, edits);
    }
    let target = project
        .channels
        .get_mut(&channel)
        .ok_or(EditError::UnknownChannel(channel.0))?;
    let old = std::mem::replace(&mut target.midi_out, out);
    Ok(Applied {
        inverse: Edit::SetChannelMidiOut { channel, out: old },
        touched: Touched {
            channels: BTreeSet::from([channel]),
            ..Touched::default()
        },
    })
}
