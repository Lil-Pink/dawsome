//! Pattern edits: a pattern added after the others (a copy's too) and taken out again, deleted
//! and restored, and renamed. Each one checks everything before it changes anything, so a refused edit leaves
//! the project as it was, and each returns its exact inverse. The four that add or remove a
//! pattern keep the project's pattern ID counter: only taking out a pattern just added gives
//! its ID back.

use std::mem;

use super::{pattern_mut, Applied, Edit, EditError, Touched};
use crate::project::{Pattern, PatternId, Project, MAX_PATTERNS};

pub(super) fn add(
    project: &mut Project,
    id: PatternId,
    name: String,
    bars: u32,
    copied_from: Option<PatternId>,
) -> Result<Applied, EditError> {
    let expected = project
        .next_pattern_id()
        .ok_or(EditError::PatternIdsExhausted)?;
    if id != expected {
        return Err(EditError::NotNextPatternId {
            got: id.0,
            expected: expected.0,
        });
    }
    if bars == 0 {
        return Err(EditError::NoBars);
    }
    if project.patterns.len() >= MAX_PATTERNS {
        return Err(EditError::TooManyPatterns);
    }
    let pattern = Pattern {
        name,
        length_bars: bars,
        copied_from,
        notes: Vec::new(),
    };
    // The rule the file is checked by: a copy names a pattern made before it.
    project.validate_pattern(id, &pattern)?;
    project.patterns.insert(id, pattern);
    // `id` is below u32::MAX: `next_pattern_id` never gives that one out.
    project.set_pattern_counter(PatternId(id.0.saturating_add(1)));
    Ok(Applied::pattern(Edit::UnaddPattern { id }, id))
}

pub(super) fn unadd(project: &mut Project, id: PatternId) -> Result<Applied, EditError> {
    let pattern = project
        .patterns
        .get(&id)
        .ok_or(EditError::UnknownPattern(id.0))?;
    // Added last: its ID is the last given out. Why not "the highest ID there": with a newer
    // pattern deleted above it, giving its ID back would make the deleted one's ID the next
    // but one, and the add that undid this would not restore the counter.
    let added_last = id.0.checked_add(1) == Some(project.pattern_counter().0);
    if !added_last || !pattern.notes.is_empty() || is_played(project, id) {
        return Err(EditError::PatternNotRemovable(id.0));
    }
    let Some(pattern) = project.patterns.remove(&id) else {
        return Err(EditError::UnknownPattern(id.0));
    };
    project.set_pattern_counter(id);
    let inverse = Edit::AddPattern {
        id,
        name: pattern.name,
        bars: pattern.length_bars,
        copied_from: pattern.copied_from,
    };
    Ok(Applied::pattern(inverse, id))
}

pub(super) fn delete(project: &mut Project, id: PatternId) -> Result<Applied, EditError> {
    if !project.patterns.contains_key(&id) {
        return Err(EditError::UnknownPattern(id.0));
    }
    // Why keep one: the rack, the piano roll and pattern mode all show a pattern, and a project
    // without one has nothing to put a step in.
    if project.patterns.len() == 1 {
        return Err(EditError::LastPattern);
    }
    if is_played(project, id) {
        return Err(EditError::PatternInUse(id.0));
    }
    let counter = project.pattern_counter();
    let Some(pattern) = project.patterns.remove(&id) else {
        return Err(EditError::UnknownPattern(id.0));
    };
    project.set_pattern_counter(counter);
    Ok(Applied::pattern(Edit::RestorePattern { id, pattern }, id))
}

pub(super) fn restore(
    project: &mut Project,
    id: PatternId,
    pattern: Pattern,
) -> Result<Applied, EditError> {
    if project.patterns.contains_key(&id) {
        return Err(EditError::PatternExists(id.0));
    }
    project.validate_pattern(id, &pattern)?;
    let counter = project.pattern_counter();
    project.patterns.insert(id, pattern);
    // The counter stays above every ID: a delete's inverse puts back an ID below it.
    project.set_pattern_counter(counter.max(PatternId(id.0.saturating_add(1))));
    Ok(Applied::pattern(Edit::DeletePattern { id }, id))
}

pub(super) fn rename(
    project: &mut Project,
    id: PatternId,
    name: String,
) -> Result<Applied, EditError> {
    let old = mem::replace(&mut pattern_mut(project, id)?.name, name);
    // A name is not in the plan, so nothing is compiled again.
    Ok(Applied {
        inverse: Edit::RenamePattern { id, name: old },
        touched: Touched::default(),
    })
}

/// Whether a clip plays pattern `id`.
fn is_played(project: &Project, id: PatternId) -> bool {
    project
        .playlist
        .clips
        .values()
        .any(|clip| clip.pattern() == Some(id))
}
