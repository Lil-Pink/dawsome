//! Mixer edits: insert faders, pans, mutes, names and the solo set, channel routing, sends, and
//! the effects in the slots. Each one checks everything before it changes anything, so a refused
//! edit leaves the project as it was, and each returns its exact inverse.
//!
//! Parameter values are refused only when they are not finite numbers: a processor clamps its
//! own values to its ranges, so this module needs no knowledge of the processors there are.

use std::collections::BTreeSet;
use std::mem;

use super::{channel_mut, Applied, Edit, EditError};
use crate::project::{
    ChannelId, Effect, EffectId, Insert, InsertId, ProcessorRef, Project, ValidationError,
    EFFECT_SLOTS, FADER_RANGE, PAN_RANGE, SEND_RANGE,
};

fn insert_mut(project: &mut Project, id: InsertId) -> Result<&mut Insert, EditError> {
    project
        .mixer
        .get_mut(id)
        .ok_or(EditError::UnknownInsert(id.0))
}

pub(super) fn set_fader(
    project: &mut Project,
    insert: InsertId,
    fader: f32,
) -> Result<Applied, EditError> {
    // `contains` is false for NaN, which is not a position.
    if !FADER_RANGE.contains(&fader) {
        return Err(EditError::InvalidFader(fader));
    }
    let old = mem::replace(&mut insert_mut(project, insert)?.fader, fader);
    Ok(Applied::mixer(Edit::SetInsertVolume { insert, fader: old }))
}

pub(super) fn set_pan(
    project: &mut Project,
    insert: InsertId,
    pan: f32,
) -> Result<Applied, EditError> {
    if !PAN_RANGE.contains(&pan) {
        return Err(EditError::InvalidPan(pan));
    }
    let old = mem::replace(&mut insert_mut(project, insert)?.pan, pan);
    Ok(Applied::mixer(Edit::SetInsertPan { insert, pan: old }))
}

pub(super) fn set_mute(
    project: &mut Project,
    insert: InsertId,
    mute: bool,
) -> Result<Applied, EditError> {
    let old = mem::replace(&mut insert_mut(project, insert)?.mute, mute);
    Ok(Applied::mixer(Edit::MuteInsert { insert, mute: old }))
}

pub(super) fn rename(
    project: &mut Project,
    insert: InsertId,
    name: String,
) -> Result<Applied, EditError> {
    let old = mem::replace(&mut insert_mut(project, insert)?.name, name);
    Ok(Applied::mixer(Edit::RenameInsert { insert, name: old }))
}

pub(super) fn solo(project: &mut Project, set: BTreeSet<InsertId>) -> Result<Applied, EditError> {
    if let Some(&missing) = set.iter().find(|&&id| project.mixer.get(id).is_none()) {
        return Err(EditError::UnknownInsert(missing.0));
    }
    let old = mem::replace(&mut project.mixer.solo, set);
    Ok(Applied::mixer(Edit::SoloInsert { set: old }))
}

pub(super) fn route_channel(
    project: &mut Project,
    channel: ChannelId,
    insert: InsertId,
) -> Result<Applied, EditError> {
    if project.mixer.get(insert).is_none() {
        return Err(EditError::UnknownInsert(insert.0));
    }
    let old = mem::replace(&mut channel_mut(project, channel)?.insert, insert);
    let inverse = Edit::RouteChannel {
        channel,
        insert: old,
    };
    Ok(Applied::channel(inverse, channel))
}

pub(super) fn set_send(
    project: &mut Project,
    from: InsertId,
    to: InsertId,
    amount: Option<f32>,
) -> Result<Applied, EditError> {
    let mixer = &project.mixer;
    if let Some(missing) = [from, to].into_iter().find(|&id| mixer.get(id).is_none()) {
        return Err(EditError::UnknownInsert(missing.0));
    }
    if from.is_master() {
        return Err(EditError::SendFromMaster);
    }
    if to == from {
        return Err(EditError::SendToItself(from.0));
    }
    if let Some(amount) = amount {
        if !SEND_RANGE.contains(&amount) {
            return Err(EditError::InvalidSendAmount(amount));
        }
        // Why only when adding or changing a send: removing one can never close a loop.
        if mixer.feeds(to, from) {
            return Err(EditError::RoutingCycle { from, to });
        }
    }
    let routes = &mut insert_mut(project, from)?.routes;
    let old = match amount {
        Some(amount) => routes.insert(to, amount),
        None => routes.remove(&to),
    };
    Ok(Applied::mixer(Edit::SetSend {
        from,
        to,
        amount: old,
    }))
}

/// Where effect `id` is: its insert's position and its index in that insert's list.
fn locate(project: &Project, id: EffectId) -> Result<(usize, usize), EditError> {
    project
        .mixer
        .inserts
        .iter()
        .enumerate()
        .find_map(|(insert, strip)| {
            let index = strip.effects.iter().position(|effect| effect.id == id)?;
            Some((insert, index))
        })
        .ok_or(EditError::UnknownEffect(id.0))
}

/// Checks that `slot` of `insert` can take a new effect, and that no effect has `id` already.
fn check_free_slot(
    project: &Project,
    insert: InsertId,
    slot: u8,
    id: EffectId,
    processor: &ProcessorRef,
) -> Result<(), EditError> {
    let strip = project
        .mixer
        .get(insert)
        .ok_or(EditError::UnknownInsert(insert.0))?;
    if slot >= EFFECT_SLOTS {
        return Err(EditError::InvalidEffectSlot(slot));
    }
    if strip.effects.iter().any(|effect| effect.slot == slot) {
        return Err(EditError::SlotTaken {
            insert: insert.0,
            slot,
        });
    }
    // Only a project that never passed `validate` can hold the ID somewhere already.
    if project.mixer.effect(id).is_some() {
        return Err(EditError::EffectExists(id.0));
    }
    if let Some(&value) = processor.params.values().find(|value| !value.is_finite()) {
        return Err(EditError::InvalidParamValue(value));
    }
    Ok(())
}

/// Puts `effect` into `insert` at its sorted place. The slot was checked to be free.
fn place(project: &mut Project, insert: InsertId, effect: Effect) -> Result<(), EditError> {
    let effects = &mut insert_mut(project, insert)?.effects;
    let at = effects.partition_point(|other| other.slot < effect.slot);
    effects.insert(at, effect);
    Ok(())
}

/// Takes effect `id` out, returning where it was and what it was.
fn take(project: &mut Project, id: EffectId) -> Result<(InsertId, Effect), EditError> {
    let (insert, index) = locate(project, id)?;
    let effect = project.mixer.inserts[insert].effects.remove(index);
    // Positions come from a list that `validate` keeps far below u16::MAX entries long.
    let insert = InsertId(u16::try_from(insert).map_err(|_| EditError::UnknownEffect(id.0))?);
    Ok((insert, effect))
}

pub(super) fn add_effect(
    project: &mut Project,
    insert: InsertId,
    slot: u8,
    id: EffectId,
    processor: ProcessorRef,
) -> Result<Applied, EditError> {
    let expected = project.mixer.next_effect_id;
    if id != expected {
        return Err(EditError::NotNextEffectId {
            got: id.0,
            expected: expected.0,
        });
    }
    let next = id.0.checked_add(1).ok_or(EditError::EffectIdsExhausted)?;
    check_free_slot(project, insert, slot, id, &processor)?;
    place(
        project,
        insert,
        Effect {
            id,
            slot,
            processor,
        },
    )?;
    project.mixer.next_effect_id = EffectId(next);
    Ok(Applied::effect(Edit::UnaddEffect { id }, id))
}

pub(super) fn unadd_effect(project: &mut Project, id: EffectId) -> Result<Applied, EditError> {
    if id.0.checked_add(1) != Some(project.mixer.next_effect_id.0) {
        return Err(EditError::NotNewestEffect(id.0));
    }
    let (insert, effect) = take(project, id)?;
    // Giving the ID back is safe: it was the newest, and nothing refers to it any more.
    project.mixer.next_effect_id = id;
    let inverse = Edit::AddEffect {
        insert,
        slot: effect.slot,
        id,
        processor: effect.processor,
    };
    Ok(Applied::effect(inverse, id))
}

pub(super) fn remove_effect(project: &mut Project, id: EffectId) -> Result<Applied, EditError> {
    let (insert, effect) = take(project, id)?;
    let inverse = Edit::RestoreEffect {
        insert,
        slot: effect.slot,
        id,
        processor: effect.processor,
    };
    Ok(Applied::effect(inverse, id))
}

pub(super) fn restore_effect(
    project: &mut Project,
    insert: InsertId,
    slot: u8,
    id: EffectId,
    processor: ProcessorRef,
) -> Result<Applied, EditError> {
    let next = project.mixer.next_effect_id;
    if id >= next {
        return Err(ValidationError::UnallocatedEffectId {
            id: id.0,
            next: next.0,
        }
        .into());
    }
    check_free_slot(project, insert, slot, id, &processor)?;
    place(
        project,
        insert,
        Effect {
            id,
            slot,
            processor,
        },
    )?;
    Ok(Applied::effect(Edit::RemoveEffect { id }, id))
}

pub(super) fn set_effect_param(
    project: &mut Project,
    effect: EffectId,
    key: String,
    value: Option<f32>,
) -> Result<Applied, EditError> {
    if let Some(value) = value.filter(|value| !value.is_finite()) {
        return Err(EditError::InvalidParamValue(value));
    }
    let (insert, index) = locate(project, effect)?;
    let params = &mut project.mixer.inserts[insert].effects[index]
        .processor
        .params;
    let old = match value {
        Some(value) => params.insert(key.clone(), value),
        None => params.remove(&key),
    };
    let inverse = Edit::SetEffectParam {
        effect,
        key,
        value: old,
    };
    Ok(Applied::params(inverse, effect))
}

pub(super) fn move_effect(
    project: &mut Project,
    effect: EffectId,
    slot: u8,
) -> Result<Applied, EditError> {
    if slot >= EFFECT_SLOTS {
        return Err(EditError::InvalidEffectSlot(slot));
    }
    let (insert, index) = locate(project, effect)?;
    let effects = &mut project.mixer.inserts[insert].effects;
    let old = effects[index].slot;
    if let Some(other) = effects.iter_mut().find(|other| other.slot == slot) {
        other.slot = old;
    }
    effects[index].slot = slot;
    effects.sort_by_key(|effect| effect.slot);
    // The effect keeps its ID, so its processor keeps running: only the mixing order changed.
    Ok(Applied::mixer(Edit::MoveEffect { effect, slot: old }))
}
