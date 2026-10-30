//! Pattern copies: a clip given its own copy of the pattern it plays ([`unlink`]) and given
//! back to the pattern its copy was made from ([`link_back`]), each one
//! transaction of plain edits with its exact inverse, so one undo step gives the document back
//! byte for byte. A copy remembers the pattern it was made from (`Pattern::copied_from`), and
//! pattern IDs are never given out twice, so that memory never names another pattern.

use std::collections::BTreeMap;

use super::{Edit, EditError};
use crate::project::{ClipId, PatternId, Project};

/// How a pattern clip shares its pattern, as the piano roll's link control shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Its pattern plays in `clips` clips, more than one: an edit to it changes all of them.
    Linked { clips: usize },
    /// It plays alone a copy made from `original`, which still exists.
    Unlinked { original: PatternId },
}

/// A clip unlinked: the edit (one transaction), and the copy the clip plays from then on.
#[derive(Debug, Clone, PartialEq)]
pub struct Unlink {
    pub edit: Edit,
    pub copy: PatternId,
}

/// A clip linked back: the edit, and the pattern the clip plays from then on.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkBack {
    pub edit: Edit,
    pub original: PatternId,
}

/// How many clips play each pattern; a pattern no clip plays is left out. One pass over the
/// playlist, so the playlist can badge every clip from it.
pub fn clip_counts(project: &Project) -> BTreeMap<PatternId, usize> {
    let mut counts = BTreeMap::new();
    for pattern in project
        .playlist
        .clips
        .values()
        .filter_map(|clip| clip.pattern())
    {
        *counts.entry(pattern).or_insert(0) += 1;
    }
    counts
}

/// The pattern clip `clip`'s pattern was copied from, when that pattern still exists.
pub fn original_of(project: &Project, clip: ClipId) -> Option<PatternId> {
    let pattern = project.playlist.clips.get(&clip)?.pattern()?;
    let original = project.patterns.get(&pattern)?.copied_from?;
    project.patterns.contains_key(&original).then_some(original)
}

/// How pattern clip `clip` shares its pattern: linked while more than one clip plays it, else
/// unlinked when it is a copy of a pattern that still exists; `None` otherwise, and for a clip
/// that is not a pattern clip.
///
/// Why linked first: the link control warns before an edit, and a copy that other clips play
/// too changes them all, whatever it was copied from.
pub fn link_of(project: &Project, clip: ClipId) -> Option<Link> {
    let pattern = project.playlist.clips.get(&clip)?.pattern()?;
    let clips = project
        .playlist
        .clips
        .values()
        .filter(|other| other.pattern() == Some(pattern))
        .count();
    if clips > 1 {
        return Some(Link::Linked { clips });
    }
    original_of(project, clip).map(|original| Link::Unlinked { original })
}

/// "<name> (copy)", or "<name> (copy N)" for the first N from 2 that no pattern is named.
pub fn copy_name(project: &Project, name: &str) -> String {
    let taken = |candidate: &str| {
        project
            .patterns
            .values()
            .any(|pattern| pattern.name == candidate)
    };
    let first = format!("{name} (copy)");
    if !taken(&first) {
        return first;
    }
    (2u32..)
        .map(|number| format!("{name} (copy {number})"))
        .find(|candidate| !taken(candidate))
        .unwrap_or(first)
}

/// The pattern clip `clip` plays, refusing a clip that is not there or not a pattern clip.
fn pattern_of(project: &Project, clip: ClipId) -> Result<PatternId, EditError> {
    project
        .playlist
        .clips
        .get(&clip)
        .ok_or(EditError::UnknownClip(clip.0))?
        .pattern()
        .ok_or(EditError::NotAPatternClip(clip.0))
}

/// Gives pattern clip `clip` its own copy of its pattern: a new pattern named by [`copy_name`],
/// with the same length and notes (every channel's), made from it, and the clip playing it. One
/// transaction (`AddPattern`, `ReplaceNotes` when there are notes, `SetClipPattern`), whose
/// inverse takes the clip back, the notes out and the copy away, giving its ID back.
pub fn unlink(project: &Project, clip: ClipId) -> Result<Unlink, EditError> {
    let source = pattern_of(project, clip)?;
    let shared = project
        .patterns
        .get(&source)
        .ok_or(EditError::UnknownPattern(source.0))?;
    let copy = project
        .next_pattern_id()
        .ok_or(EditError::PatternIdsExhausted)?;
    let mut edits = vec![Edit::AddPattern {
        id: copy,
        name: copy_name(project, &shared.name),
        bars: shared.length_bars,
        copied_from: Some(source),
    }];
    if !shared.notes.is_empty() {
        edits.push(Edit::ReplaceNotes {
            pattern: copy,
            remove: Vec::new(),
            add: shared.notes.clone(),
        });
    }
    edits.push(Edit::SetClipPattern {
        id: clip,
        pattern: copy,
    });
    Ok(Unlink {
        edit: Edit::Transaction(edits),
        copy,
    })
}

/// Gives pattern clip `clip`, playing a copy, back to the pattern the copy was made from; the
/// copy goes in the same step when no other clip plays it, else it stays for them. The inverse
/// puts the copy back (its notes and edits with it) and the clip on it.
pub fn link_back(project: &Project, clip: ClipId) -> Result<LinkBack, EditError> {
    let copy = pattern_of(project, clip)?;
    let original = original_of(project, clip).ok_or(EditError::NotACopy(clip.0))?;
    let point = Edit::SetClipPattern {
        id: clip,
        pattern: original,
    };
    let others = project
        .playlist
        .clips
        .iter()
        .any(|(&id, other)| id != clip && other.pattern() == Some(copy));
    let edit = if others {
        point
    } else {
        Edit::Transaction(vec![point, Edit::DeletePattern { id: copy }])
    };
    Ok(LinkBack { edit, original })
}
