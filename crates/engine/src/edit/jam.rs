//! A Jam written into the project ([`Edit::Jam`]): the edits that make each of its changes, applied
//! as one transaction, so its inverse is exact by construction.

use super::{Applied, Edit, EditError};
use crate::jam::{JamPattern, PPQ_MULTIPLE};
use crate::project::{KeySignature, PatternId, Project};

pub(super) fn apply(
    project: &mut Project,
    pattern_name: String,
    replace: Option<PatternId>,
    pattern: JamPattern,
    set_tempo: bool,
    key: KeySignature,
) -> Result<Applied, EditError> {
    // Why these checks live here and not only in their callers: the notes are 4/4 bars at the
    // pattern's PPQ, and any other meter or PPQ would put them past the pattern's end, between
    // its steps or on top of each other.
    if !project.ppq.is_multiple_of(PPQ_MULTIPLE) {
        return Err(EditError::JamPpq(project.ppq));
    }
    if pattern.ppq != project.ppq {
        return Err(EditError::JamPpqMismatch {
            written: pattern.ppq,
            project: project.ppq,
        });
    }
    let signature = match replace {
        None => project.time_signature,
        Some(id) if project.patterns.contains_key(&id) => project.pattern_signature(id),
        Some(id) => return Err(EditError::UnknownPattern(id.0)),
    };
    if !signature.is_common() {
        return Err(EditError::JamMeter(signature));
    }
    let notes = pattern.notes();
    let mut edits = Vec::new();
    let id = match replace {
        None => {
            let id = project
                .next_pattern_id()
                .ok_or(EditError::PatternIdsExhausted)?;
            edits.push(Edit::AddPattern {
                id,
                name: pattern_name,
                bars: pattern.bars,
                copied_from: None,
            });
            if !notes.is_empty() {
                edits.push(Edit::ReplaceNotes {
                    pattern: id,
                    remove: Vec::new(),
                    add: notes,
                });
            }
            id
        }
        Some(id) => {
            let old = project
                .patterns
                .get(&id)
                .ok_or(EditError::UnknownPattern(id.0))?
                .notes
                .clone();
            // Why every note goes: the pattern is the last roll's, and a roll replaces it whole.
            if !(old.is_empty() && notes.is_empty()) {
                edits.push(Edit::ReplaceNotes {
                    pattern: id,
                    remove: old,
                    add: notes,
                });
            }
            edits.push(Edit::SetPatternBars {
                pattern: id,
                bars: pattern.bars,
            });
            id
        }
    };
    edits.push(Edit::SetPatternSwing {
        pattern: id,
        swing_tenths: pattern.swing_tenths,
    });
    edits.push(Edit::SetKey { key: Some(key) });
    if set_tempo {
        edits.push(Edit::SetTempo {
            mbpm: pattern.tempo,
        });
    }
    super::apply(project, Edit::Transaction(edits))
}
