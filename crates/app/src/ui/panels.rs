//! The main window's panels. The Library is a column down the left; the other open panels stack
//! under the toolbar to its right in a fixed order (rack, playlist, piano roll, mixer). Each
//! function key toggles one. Settings is no panel of the main window's: its key, digit and
//! button toggle the Settings window (it once stacked under the mixer, where its
//! lower options could not be scrolled to and nothing closed it but its key).
//!
//! Stacked panels keep a least height each ([`min_height`]): when a stacked panel just opened
//! leaves the stack no room for every open panel's, the most recently used of the others stay
//! while they fit and the rest close ([`closed_to_fit`]); a stack that is too short all the
//! same (a window made smaller, a scripted run's panels) keeps the piano roll's grid whole and
//! squeezes the others ([`stack_heights`]). With room, one panel takes what the others leave
//! ([`flexible`]: the playlist, else the piano roll, else the rack), the others their kept
//! heights, else their natural ones ([`natural_height`]).
//!
//! Every stacked panel under another starts with a divider ([`DIVIDER_BAND`], inside its
//! height): dragged, the line between the two moves ([`dragged`]) and the heights it leaves are
//! kept ([`Kept`], in the settings); a double-click puts the two back ([`reset`]). The mixer's
//! top edge is that band even alone, where nothing drags it (its desk is laid out under it).

use crate::ui::layout::{self, RACK_HEADER_HEIGHT};
use crate::ui::mixer_layout;
use crate::ui::piano_layout::{rack_height_above_roll, ROW_HEIGHT, STRIP_HEIGHT, UNDER_GRID};

/// A panel of the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// The channel rack (1, F6).
    Rack,
    /// The playlist (2, F5).
    Playlist,
    /// The piano roll (3, F7).
    PianoRoll,
    /// The mixer (4, F9).
    Mixer,
    /// The Library (F8): instruments, sample folders and MIDI devices. Named the browser on the
    /// command line, as before it had more than sample folders.
    Browser,
    /// The settings: MIDI inputs, the typing keyboard, the audio devices, the keys (6, F10). A
    /// window of its own, not stacked.
    Settings,
}

impl Panel {
    /// The panels stacked under the toolbar, top to bottom. The Library is a column of its own
    /// and Settings a window of its own.
    pub const STACKED: [Panel; 4] = [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer];

    /// Every panel, in the order the toolbar's panel buttons show them: the Library, the rack,
    /// the playlist on the first row; the piano roll, the mixer and the settings on the second.
    /// Why this order: the rack, the playlist, the piano roll and the mixer then read as they
    /// stack down the window and as the digits 1 to 4 toggle them.
    pub const ALL: [Panel; 6] = [
        Panel::Browser,
        Panel::Rack,
        Panel::Playlist,
        Panel::PianoRoll,
        Panel::Mixer,
        Panel::Settings,
    ];

    /// The name the window shows for it.
    pub fn title(self) -> &'static str {
        match self {
            Panel::Rack => "Channel rack",
            Panel::Playlist => "Playlist",
            Panel::PianoRoll => "Piano roll",
            Panel::Mixer => "Mixer",
            Panel::Browser => "Library",
            Panel::Settings => "Settings",
        }
    }

    /// Its name on the command line (`--panels rack,playlist,piano-roll,mixer,browser`).
    pub fn from_name(name: &str) -> Option<Self> {
        Panel::ALL.into_iter().find(|panel| panel.name() == name)
    }

    /// Its name on the command line and in the saved window layout.
    pub fn name(self) -> &'static str {
        match self {
            Panel::Rack => "rack",
            Panel::Playlist => "playlist",
            Panel::PianoRoll => "piano-roll",
            Panel::Mixer => "mixer",
            Panel::Browser => "browser",
            Panel::Settings => "settings",
        }
    }
}

/// Which panels are open. The window opens with the rack alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panels {
    pub rack: bool,
    pub playlist: bool,
    pub piano_roll: bool,
    pub mixer: bool,
    pub browser: bool,
    pub settings: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            rack: true,
            playlist: false,
            piano_roll: false,
            mixer: false,
            browser: false,
            settings: false,
        }
    }
}

impl Panels {
    /// No panel open.
    pub const NONE: Self = Self {
        rack: false,
        playlist: false,
        piano_roll: false,
        mixer: false,
        browser: false,
        settings: false,
    };

    pub fn is_open(self, panel: Panel) -> bool {
        match panel {
            Panel::Rack => self.rack,
            Panel::Playlist => self.playlist,
            Panel::PianoRoll => self.piano_roll,
            Panel::Mixer => self.mixer,
            Panel::Browser => self.browser,
            Panel::Settings => self.settings,
        }
    }

    pub fn set(&mut self, panel: Panel, open: bool) {
        match panel {
            Panel::Rack => self.rack = open,
            Panel::Playlist => self.playlist = open,
            Panel::PianoRoll => self.piano_roll = open,
            Panel::Mixer => self.mixer = open,
            Panel::Browser => self.browser = open,
            Panel::Settings => self.settings = open,
        }
    }

    pub fn toggle(&mut self, panel: Panel) {
        self.set(panel, !self.is_open(panel));
    }

    /// The panels open, in [`Panel::ALL`]'s order.
    pub fn open(self) -> impl Iterator<Item = Panel> {
        Panel::ALL
            .into_iter()
            .filter(move |&panel| self.is_open(panel))
    }

    /// The stacked panels open, top to bottom ([`Panel::STACKED`]'s order).
    pub fn stacked(self) -> impl Iterator<Item = Panel> {
        Panel::STACKED
            .into_iter()
            .filter(move |&panel| self.is_open(panel))
    }

    /// The panels a first start opens: the Library, the rack and the playlist, as a beginner
    /// needs them to find a sound, put it in a pattern and arrange it.
    pub const FIRST_START: Self = Self {
        rack: true,
        playlist: true,
        piano_roll: false,
        mixer: false,
        browser: true,
        settings: false,
    };
}

/// The least height of a stacked panel: room for its header and a few rows of what it shows.
pub const MIN_HEIGHT: f32 = 180.0;

/// The rows of the piano roll's grid it always keeps.
pub const MIN_ROLL_ROWS: f32 = 5.0;

/// The piano roll's least height: its strip, [`MIN_ROLL_ROWS`] rows of its grid, and the most
/// the divider and the velocity lane take ([`UNDER_GRID`]), so the grid keeps its rows however
/// tall the lane is dragged, and more of them at the lane's default height. Why more than
/// [`MIN_HEIGHT`]: at 180 px the grid kept one row under the lane. Under another panel the
/// divider over it adds its band ([`least_heights`]).
pub const MIN_ROLL_HEIGHT: f32 = STRIP_HEIGHT + MIN_ROLL_ROWS * ROW_HEIGHT + UNDER_GRID;

/// The band along the line between two stacked panels, the lower one's top edge: the divider
/// that drags the line ([`crate::ui::grip::Grip`], its cursor and grip). It is part of the
/// lower panel's height. The mixer's handle was this band before it was every panel's,
/// so the mixer's heights hold it already, as the playlist's least height
/// does; the piano roll's heights are counted in grid rows, so its band comes on top of them.
pub const DIVIDER_BAND: f32 = mixer_layout::HANDLE_HEIGHT;

/// The least height of `panel` in the stack, the rack's content being `rack_content` tall (its
/// header and rows): the rack keeps its header and one row (it scrolls its rows under the
/// header), or its content when that is shorter; the piano roll [`MIN_ROLL_HEIGHT`]; every
/// other panel [`MIN_HEIGHT`]. Under another panel the piano roll needs its divider's band too
/// ([`least_heights`]).
pub fn min_height(panel: Panel, rack_content: f32) -> f32 {
    match panel {
        Panel::Rack => rack_content.min(RACK_HEADER_HEIGHT + layout::row_top(1)),
        Panel::PianoRoll => MIN_ROLL_HEIGHT,
        _ => MIN_HEIGHT,
    }
}

/// Whether a panel of `stacked` (in any order) stacks above `panel`, so a divider is over it.
fn under_another(panel: Panel, stacked: &[Panel]) -> bool {
    let place = |panel: Panel| Panel::STACKED.iter().position(|&stacked| stacked == panel);
    stacked.iter().any(|&other| place(other) < place(panel))
}

/// What the divider over `panel` adds to the heights it counts: the piano roll's band, when a
/// panel stacks above it; nothing for the others, whose heights hold it ([`DIVIDER_BAND`]).
fn divider_room(panel: Panel, under: bool) -> f32 {
    if under && panel == Panel::PianoRoll {
        DIVIDER_BAND
    } else {
        0.0
    }
}

/// The least height of each of `stacked` (in any order), the piano roll's with its divider's
/// band when a panel stacks above it, so its grid keeps [`MIN_ROLL_ROWS`] rows under it.
fn least_heights(stacked: &[Panel], rack_content: f32) -> Vec<f32> {
    stacked
        .iter()
        .map(|&panel| {
            let under = under_another(panel, stacked);
            min_height(panel, rack_content) + divider_room(panel, under)
        })
        .collect()
}

/// The grid rows the piano roll shows when another panel takes the height left, under the
/// tallest lane: an octave (more at the lane's default height).
pub const NATURAL_ROLL_ROWS: f32 = 12.0;

/// The heights the dividers left the rack, the piano roll and the mixer, the one source of
/// them (kept in the settings): what [`stack_heights`] gives each while another panel is the
/// flexible one, each none until a divider beside it is dragged and after a double-click on
/// it. The playlist has none: open, it takes what the others leave.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Kept {
    pub rack: Option<f32>,
    pub piano_roll: Option<f32>,
    pub mixer: Option<f32>,
}

impl Kept {
    /// The height kept for `panel`, if any.
    pub fn of(&self, panel: Panel) -> Option<f32> {
        match panel {
            Panel::Rack => self.rack,
            Panel::PianoRoll => self.piano_roll,
            Panel::Mixer => self.mixer,
            Panel::Playlist | Panel::Browser | Panel::Settings => None,
        }
    }

    /// Keeps `height` for `panel`; none puts it back to its natural height. The playlist, the
    /// Library and Settings keep none.
    pub fn set(&mut self, panel: Panel, height: Option<f32>) {
        match panel {
            Panel::Rack => self.rack = height,
            Panel::PianoRoll => self.piano_roll = height,
            Panel::Mixer => self.mixer = height,
            Panel::Playlist | Panel::Browser | Panel::Settings => {}
        }
    }

    /// The mixer's height: as a divider left it, else [`mixer_layout::DEFAULT_HEIGHT`]. Its
    /// desk is laid out from it ([`mixer_layout::desk_height`]).
    pub fn mixer_height(&self) -> f32 {
        self.mixer.unwrap_or(mixer_layout::DEFAULT_HEIGHT)
    }
}

/// The panel of `stacked` that takes the height the others leave: the playlist (its tracks
/// scroll inside it), else the piano roll, else the rack; none with only the mixer open.
pub fn flexible(stacked: &[Panel]) -> Option<Panel> {
    [Panel::Playlist, Panel::PianoRoll, Panel::Rack]
        .into_iter()
        .find(|panel| stacked.contains(panel))
}

/// The height `panel` takes in an area `area` tall when another panel is the flexible one, a
/// divider over it when `under` another panel: the height kept for it ([`Kept`]), else the
/// rack its content up to half the area, the piano roll its strip, an octave of rows and the
/// room under its grid ([`UNDER_GRID`]) (and its band under a divider), the mixer
/// [`mixer_layout::DEFAULT_HEIGHT`]; never less than its least height.
pub fn natural_height(panel: Panel, rack_content: f32, kept: &Kept, area: f32, under: bool) -> f32 {
    let band = divider_room(panel, under);
    let natural = kept
        .of(panel)
        .filter(|height| height.is_finite())
        .unwrap_or_else(|| match panel {
            Panel::Rack => rack_height_above_roll(rack_content, area),
            Panel::PianoRoll => STRIP_HEIGHT + NATURAL_ROLL_ROWS * ROW_HEIGHT + UNDER_GRID + band,
            Panel::Mixer => mixer_layout::DEFAULT_HEIGHT,
            Panel::Playlist | Panel::Browser | Panel::Settings => 0.0,
        });
    natural.max(min_height(panel, rack_content) + band)
}

/// Whether the stacked panels `stacked` all get their least height in an area `area` tall.
pub fn fits(stacked: &[Panel], rack_content: f32, area: f32) -> bool {
    least_total(stacked, rack_content) <= area
}

fn least_total(stacked: &[Panel], rack_content: f32) -> f32 {
    least_heights(stacked, rack_content).iter().sum()
}

/// The height of each of the stacked panels `stacked` (top to bottom) in an area `area` tall,
/// the rack's content being `rack_content` tall and the dividers having left `kept`, in their
/// order; they add up to `area`.
///
/// When they fit, the [`flexible`] panel takes what the others leave, and they their kept or
/// natural heights ([`natural_height`]); when those leave it less than its least height, each
/// of the others gives up the same share of what it has above its own least height, so the
/// flexible one keeps its least. A rack alone takes the whole area, and so does the mixer
/// alone.
///
/// When they do not fit: the piano roll takes its least height (or the whole area when that is
/// shorter), so its grid stays whole, and the others share the rest in proportion to their
/// least heights.
pub fn stack_heights(stacked: &[Panel], rack_content: f32, kept: &Kept, area: f32) -> Vec<f32> {
    let least = least_heights(stacked, rack_content);
    let total: f32 = least.iter().sum();
    if total > area {
        let roll_least = stacked
            .iter()
            .zip(&least)
            .find(|(&panel, _)| panel == Panel::PianoRoll)
            .map(|(_, &least)| least);
        let roll = roll_least.map(|least| least.min(area));
        let rest = area - roll.unwrap_or(0.0);
        let others = total - roll_least.unwrap_or(0.0);
        return stacked
            .iter()
            .zip(&least)
            .map(|(&panel, &least)| match (panel, roll) {
                (Panel::PianoRoll, Some(roll)) => roll,
                _ if others > 0.0 => least * rest / others,
                _ => 0.0,
            })
            .collect();
    }
    let Some(flex) = flexible(stacked) else {
        let mut heights = least.clone();
        share(&mut heights, &least, area);
        return heights;
    };
    let others = |panel: Panel| panel != flex;
    let natural: Vec<f32> = stacked
        .iter()
        .map(|&panel| {
            if others(panel) {
                let under = under_another(panel, stacked);
                natural_height(panel, rack_content, kept, area, under)
            } else {
                0.0
            }
        })
        .collect();
    let flex_least = stacked
        .iter()
        .zip(&least)
        .find(|(&panel, _)| panel == flex)
        .map_or(0.0, |(_, &least)| least);
    let room = area - flex_least;
    let natural_total: f32 = natural.iter().sum();
    let least_total = total - flex_least;
    // What each of the others keeps of its height above its least: all of it with room, else
    // the same share each, so the flexible panel keeps its least height.
    let kept_share = if natural_total <= room || natural_total <= least_total {
        1.0
    } else {
        (room - least_total) / (natural_total - least_total)
    };
    let mut heights: Vec<f32> = stacked
        .iter()
        .zip(natural.iter().zip(&least))
        .map(|(&panel, (&natural, &least))| {
            if others(panel) {
                least + (natural - least) * kept_share
            } else {
                0.0
            }
        })
        .collect();
    let used: f32 = heights.iter().sum();
    if let Some(at) = stacked.iter().position(|&panel| panel == flex) {
        heights[at] = area - used;
    }
    heights
}

/// The heights kept once the divider over `stacked[lower]` (`stacked` top to bottom) was
/// dragged `moved` px down (up when negative) from where it was pressed, `kept` being what was
/// kept at the press: the line between the two panels follows, the one above growing by what
/// the one under it gives up or the other way round, in whole pixels, neither under its least
/// height, the others as they were. Each of the two keeps the height it shows, but the
/// [`flexible`] panel, which takes what the others leave.
///
/// Nothing moves while the least heights do not fit. While the others squeeze the flexible
/// panel (a window made smaller, a panel opened), every panel but it keeps the height it shows
/// first, so the line under the cursor is the only one that moves.
pub fn dragged(
    stacked: &[Panel],
    rack_content: f32,
    kept: &Kept,
    area: f32,
    lower: usize,
    moved: f32,
) -> Kept {
    let Some(upper) = lower.checked_sub(1) else {
        return *kept;
    };
    if lower >= stacked.len() || !fits(stacked, rack_content, area) {
        return *kept;
    }
    let shown = stack_heights(stacked, rack_content, kept, area);
    let least = least_heights(stacked, rack_content);
    let flex = flexible(stacked);
    let mut out = *kept;
    let squeezed = stacked.iter().zip(&shown).any(|(&panel, &height)| {
        let under = under_another(panel, stacked);
        let natural = natural_height(panel, rack_content, kept, area, under);
        Some(panel) != flex && height < natural - 0.01
    });
    if squeezed {
        for (&panel, &height) in stacked.iter().zip(&shown) {
            if Some(panel) != flex {
                out.set(panel, Some(height));
            }
        }
    }
    let pair = shown[upper] + shown[lower];
    let top = (shown[upper] + moved)
        .round()
        .clamp(least[upper], (pair - least[lower]).max(least[upper]));
    for (at, height) in [(upper, top), (lower, pair - top)] {
        if Some(stacked[at]) != flex {
            out.set(stacked[at], Some(height));
        }
    }
    out
}

/// The heights kept once the divider over `stacked[lower]` was double-clicked: the two panels
/// it parts back to their natural heights, the [`flexible`] one as it was (it keeps none it
/// shows, and what it keeps for when another panel is the flexible one stays).
pub fn reset(stacked: &[Panel], kept: &Kept, lower: usize) -> Kept {
    let flex = flexible(stacked);
    let mut out = *kept;
    let pair = lower.checked_sub(1).map_or(&[][..], |upper| {
        stacked.get(upper..=lower).unwrap_or_default()
    });
    for &panel in pair {
        if Some(panel) != flex {
            out.set(panel, None);
        }
    }
    out
}

/// Shares `rest` among panels of least heights `least`: equally, except that a panel whose
/// share would be under its least height takes that and the others share what is left. The
/// least heights add up to no more than `rest`.
fn share(heights: &mut [f32], least: &[f32], rest: f32) {
    let mut fixed = vec![false; least.len()];
    loop {
        let free = fixed.iter().filter(|&&fixed| !fixed).count();
        if free == 0 {
            return;
        }
        let taken: f32 = least
            .iter()
            .zip(&fixed)
            .filter(|(_, &fixed)| fixed)
            .map(|(least, _)| least)
            .sum();
        let each = (rest - taken) / free as f32;
        let mut changed = false;
        for (index, &least) in least.iter().enumerate() {
            if !fixed[index] && least > each {
                fixed[index] = true;
                changed = true;
            }
        }
        if !changed {
            for (index, height) in heights.iter_mut().enumerate() {
                *height = if fixed[index] { least[index] } else { each };
            }
            return;
        }
    }
}

/// The stacked panels from the least recently used (opened or pressed) to the most: which
/// close first when a panel opened leaves no room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelUse([Panel; 4]);

impl Default for PanelUse {
    /// Top to bottom: the rack the least recently used.
    fn default() -> Self {
        Self(Panel::STACKED)
    }
}

impl PanelUse {
    /// `panel` was just used. The Library and Settings are no stacked panels and change
    /// nothing.
    pub fn touch(&mut self, panel: Panel) {
        if let Some(at) = self.0.iter().position(|&used| used == panel) {
            self.0[at..].rotate_left(1);
        }
    }

    /// The stacked panels, least recently used first.
    pub fn order(&self) -> [Panel; 4] {
        self.0
    }
}

/// The panels the make-room rule closed, each against the panel whose opening closed it, so
/// that closing that panel by its own key or button opens them again: a panel opened by a stray
/// key is undone by the same key. A panel opened again since waits no more.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Displaced(Vec<(Panel, Panel)>);

impl Displaced {
    /// `closed` were closed to make room for `by`, which was just opened.
    pub fn record(&mut self, by: Panel, closed: &[Panel]) {
        for &panel in closed {
            self.forget(panel);
            self.0.push((panel, by));
        }
    }

    /// `panel` was opened: it waits for no panel to close.
    pub fn forget(&mut self, panel: Panel) {
        self.0.retain(|&(displaced, _)| displaced != panel);
    }

    /// `by` was closed by hand: the panels its opening closed, which wait no more.
    pub fn take(&mut self, by: Panel) -> Vec<Panel> {
        let back = self
            .0
            .iter()
            .filter(|&&(_, displacer)| displacer == by)
            .map(|&(displaced, _)| displaced)
            .collect();
        self.0.retain(|&(_, displacer)| displacer != by);
        back
    }
}

/// The panels to close, least recently used first, so that every stacked panel left open in
/// `open` gets its least height in an area `area` tall after `opened` (which `open` holds)
/// was opened: beside it, the most recently used of the others stay while they fit, the rest
/// close. None when they all fit, never `opened`, all the others when it alone does not fit,
/// and none when `opened` is not stacked: the Library and the Settings window take no height of
/// the stack.
pub fn closed_to_fit(
    open: Panels,
    opened: Panel,
    used: &PanelUse,
    rack_content: f32,
    area: f32,
) -> Vec<Panel> {
    if !Panel::STACKED.contains(&opened) {
        return Vec::new();
    }
    let mut kept = vec![opened];
    let mut closed = Vec::new();
    for panel in used.order().into_iter().rev() {
        if panel == opened || !open.is_open(panel) {
            continue;
        }
        kept.push(panel);
        if !fits(&kept, rack_content, area) {
            kept.pop();
            closed.push(panel);
        }
    }
    closed.reverse();
    closed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::piano_layout::{visible_rows, DIVIDER_HEIGHT, LANE_HEIGHT};

    /// The mixer's height in these tests: the 504 px it had before its handle, so
    /// their answers stand; the default's are tested apart.
    const MIXER: Kept = Kept {
        rack: None,
        piano_roll: None,
        mixer: Some(504.0),
    };

    /// Nothing kept: every panel at its natural height.
    const NATURAL: Kept = Kept {
        rack: None,
        piano_roll: None,
        mixer: None,
    };

    /// The grid rows a piano roll `height` tall under another panel shows: what is left under
    /// the divider's band and its strip and above its lane's divider and lane.
    fn roll_rows(height: f32) -> u8 {
        visible_rows(height - DIVIDER_BAND - STRIP_HEIGHT - DIVIDER_HEIGHT - LANE_HEIGHT)
    }

    fn near(heights: &[f32], expected: &[f32]) -> bool {
        heights.len() == expected.len()
            && heights
                .iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 1e-3)
    }

    // The starter document's rack (its header, 8 px, four rows and the add button's row of
    // 38 px each: 24 + 8 + 5 × 38 = 222 px) with the playlist, the piano roll and the mixer in
    // a 1000 × 560 window (504 px under the toolbar): their least heights (70, 180, 230, 180)
    // add up to 660 px, so the roll keeps its 230 px and its grid rows (nine over the 10 px
    // scroll bar, the lane's 6 px handle and the lane's default 64 px; pin moved: five under
    // the 128 px lane and 8 px divider before, ten with the half lane, nine with the scroll
    // bar), and the rest's 274 px go 70 : 180 : 180 to the others:
    // 44.60, 114.70 and 114.70 px. (Resizable dividers, pins moved: the roll's least was 224
    // and the rest 280; under another panel it now has the divider's 6 px band over its strip,
    // which its least height counts so its grid keeps the same rows.)
    #[test]
    fn the_piano_roll_keeps_its_grid_with_four_panels_in_a_small_window() {
        assert_eq!(MIN_ROLL_HEIGHT, 224.0);
        let stacked = [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        assert!(!fits(&stacked, 222.0, 504.0));
        let heights = stack_heights(&stacked, 222.0, &MIXER, 504.0);
        assert_eq!(heights[2], 230.0);
        assert_eq!(roll_rows(heights[2]), 9);
        let expected = [
            70.0 * 274.0 / 430.0,
            180.0 * 274.0 / 430.0,
            230.0,
            180.0 * 274.0 / 430.0,
        ];
        for (height, expected) in heights.iter().zip(expected) {
            assert!((height - expected).abs() < 1e-3, "{heights:?}");
        }
        assert!((heights.iter().sum::<f32>() - 504.0).abs() < 1e-3);
        // Shorter than the roll's least height, the roll takes the whole area. At the top of
        // the stack it has no divider over it: its least is 224.
        assert_eq!(
            stack_heights(&[Panel::PianoRoll, Panel::Mixer], 0.0, &MIXER, 200.0),
            [200.0, 0.0]
        );
        assert!(fits(&[Panel::PianoRoll, Panel::Mixer], 0.0, 404.0));
        assert!(!fits(&[Panel::Playlist, Panel::PianoRoll], 0.0, 409.0));
    }

    // With room for every least height: a rack of two channels (24 + 8 + 3 × 38 = 146 px)
    // over the playlist and the piano roll. 6d K6, pins moved (were [100, 180, 224] and [146,
    // 299, 299]): the playlist takes what the others leave. In 504 px the rack (146) and the
    // roll (an octave: 28 + 12 × 12 + the 136 under its grid = 308, and the divider's 6 px band
    // over it: 314) would leave it under its 180, so each gives up the same share of what it
    // has above its least (70 and 230): (504 − 180 − 300) / (460 − 300) = 0.15 of it is kept,
    // 81.4 and 242.6 px; at 744 px (a 1280 × 800 window) they keep their natural heights and
    // the playlist takes the 284 px left. (Resizable dividers, pins moved: [84.25, 180, 239.75]
    // and [146, 290, 308] before the roll's band.) A rack with one panel keeps its content up
    // to half the area, and a rack alone takes it all.
    #[test]
    fn panels_that_fit_share_the_stack_as_before_with_their_least_heights_kept() {
        let three = [Panel::Rack, Panel::Playlist, Panel::PianoRoll];
        assert!(fits(&three, 146.0, 504.0));
        let heights = stack_heights(&three, 146.0, &MIXER, 504.0);
        assert!(near(&heights, &[81.4, 180.0, 242.6]), "{heights:?}");
        assert_eq!(
            stack_heights(&three, 146.0, &MIXER, 744.0),
            [146.0, 284.0, 314.0]
        );
        let two = [Panel::Rack, Panel::Playlist];
        assert_eq!(stack_heights(&two, 146.0, &MIXER, 504.0), [146.0, 358.0]);
        assert_eq!(stack_heights(&two, 400.0, &MIXER, 504.0), [252.0, 252.0]);
        assert_eq!(stack_heights(&[Panel::Rack], 400.0, &MIXER, 504.0), [504.0]);
        // No rack: the mixer alone takes it all. (The settings once shared it with
        // the mixer, 200 px each at 400; they are a window of their own now.)
        assert_eq!(stack_heights(&[Panel::Mixer], 0.0, &MIXER, 400.0), [400.0]);
        // A rack shorter than a row keeps it whole.
        assert_eq!(min_height(Panel::Rack, 40.0), 40.0);
        assert_eq!(min_height(Panel::Rack, 222.0), 70.0);
    }

    // Opening the mixer over the rack, the playlist and the piano roll in 504 px: 654 px of
    // least heights do not fit, so the most recently used of the others stay while they fit
    // beside it. Used last: the roll, then the mixer; the rack and the playlist least. The roll
    // (404 px with the mixer) and then the rack (480: the roll has a divider over it now, 6 px)
    // fit, the playlist (660) does not, so it alone closes; with the rack pressed after the
    // roll, the same. (Resizable dividers: 474 and 654 before the roll's band; same answers.)
    #[test]
    fn a_panel_opened_without_room_closes_the_least_recently_used() {
        let mut open = Panels {
            rack: true,
            playlist: true,
            piano_roll: true,
            mixer: true,
            browser: true,
            settings: false,
        };
        let mut used = PanelUse::default();
        used.touch(Panel::Playlist);
        used.touch(Panel::PianoRoll);
        used.touch(Panel::Browser);
        used.touch(Panel::Mixer);
        assert_eq!(
            used.order(),
            [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer]
        );
        let closed = closed_to_fit(open, Panel::Mixer, &used, 108.0, 504.0);
        assert_eq!(closed, [Panel::Playlist]);
        used.touch(Panel::Rack);
        let closed = closed_to_fit(open, Panel::Mixer, &used, 108.0, 504.0);
        assert_eq!(closed, [Panel::Playlist]);
        // With room, nothing closes; and the panel opened never does, even alone too tall.
        assert!(closed_to_fit(open, Panel::Mixer, &used, 108.0, 744.0).is_empty());
        open.rack = false;
        open.playlist = false;
        let closed = closed_to_fit(open, Panel::Mixer, &used, 108.0, 150.0);
        assert_eq!(closed, [Panel::PianoRoll]);
    }

    // The Library is a column, so opening it closes no stacked panel,
    // even when the stack is already too short (the rack, the playlist and the piano roll, 480
    // px of least heights with the divider over the roll, after the window was made 364 px
    // tall under its toolbar).
    #[test]
    fn opening_the_library_closes_no_stacked_panel() {
        let open = Panels {
            rack: true,
            playlist: true,
            piano_roll: true,
            mixer: false,
            browser: true,
            settings: false,
        };
        let stacked: Vec<Panel> = open.stacked().collect();
        assert!(!fits(&stacked, 108.0, 364.0));
        let used = PanelUse::default();
        assert!(closed_to_fit(open, Panel::Browser, &used, 108.0, 364.0).is_empty());
    }

    // Beside the panel opened, the most recently used of the others stay
    // open while they fit, so a small panel used long ago stays when it fits. The stacked
    // panels opened in turn in 504 px (least heights: rack 70, playlist 180, piano roll 224,
    // and 230 under another panel with its divider, mixer 180): the playlist and the roll fit
    // (480); the mixer keeps the roll (404) and the rack (480) but not the playlist (660).
    // Settings then closes nothing (it kept the mixer and the rack and closed
    // the roll while it was stacked, 180 px).
    #[test]
    fn the_most_recently_used_panels_that_fit_stay_open() {
        let mut open = Panels::default();
        let mut used = PanelUse::default();
        let mut open_in_turn = |panel: Panel| {
            open.set(panel, true);
            let closed = closed_to_fit(open, panel, &used, 108.0, 504.0);
            for &shut in &closed {
                open.set(shut, false);
            }
            used.touch(panel);
            (closed, open.stacked().collect::<Vec<Panel>>())
        };
        assert_eq!(
            open_in_turn(Panel::Playlist),
            (vec![], vec![Panel::Rack, Panel::Playlist])
        );
        assert_eq!(
            open_in_turn(Panel::PianoRoll),
            (vec![], vec![Panel::Rack, Panel::Playlist, Panel::PianoRoll])
        );
        assert_eq!(
            open_in_turn(Panel::Mixer),
            (
                vec![Panel::Playlist],
                vec![Panel::Rack, Panel::PianoRoll, Panel::Mixer]
            )
        );
        assert_eq!(
            open_in_turn(Panel::Settings),
            (vec![], vec![Panel::Rack, Panel::PianoRoll, Panel::Mixer])
        );
        assert!(open.settings, "open, as a window");
    }

    // 6d K6: the playlist is the flexible panel, else the piano roll, else the rack. Maximized
    // on 1920 × 1080 with the hint bar (1080 − 64 − 22 = 994 px under the toolbar), the starter
    // rack (222) over the playlist leaves it 772; with the mixer under them it keeps its 400 px
    // and the playlist 372; with the piano roll instead of the playlist, the roll takes what the
    // rack leaves; the mixer and the settings alone share as before. (Pins moved:
    // 1002 px under the 56 px toolbar gave 780, and the 504 px mixer left 276; 986 under the
    // 72 px one gave 764 and 364; the
    // mixer's handle dragged to 600 px leaves the playlist 172, under its least, so the rack and
    // the mixer give up the same share of their height above their least: (994 − 180 − 250) /
    // (822 − 250) of it.)
    #[test]
    fn the_playlist_takes_the_height_the_others_leave() {
        let area = 1080.0 - crate::ui::layout::TOOLBAR_HEIGHT - crate::ui::layout::HINT_BAR_HEIGHT;
        assert_eq!(area, 994.0);
        let rack = 222.0;
        let mixer = &NATURAL;
        assert_eq!(mixer.mixer_height(), 400.0);
        assert_eq!(
            flexible(&[Panel::Rack, Panel::Playlist]),
            Some(Panel::Playlist)
        );
        assert_eq!(
            stack_heights(&[Panel::Rack, Panel::Playlist], rack, mixer, area),
            [222.0, 772.0]
        );
        assert_eq!(
            stack_heights(
                &[Panel::Rack, Panel::Playlist, Panel::Mixer],
                rack,
                mixer,
                area
            ),
            [222.0, 372.0, 400.0]
        );
        let tall = Kept {
            mixer: Some(600.0),
            ..NATURAL
        };
        let tall = stack_heights(
            &[Panel::Rack, Panel::Playlist, Panel::Mixer],
            rack,
            &tall,
            area,
        );
        let kept = (994.0 - 180.0 - 250.0) / (822.0 - 250.0);
        let expected = [70.0 + 152.0 * kept, 180.0, 180.0 + 420.0 * kept];
        for (height, expected) in tall.iter().zip(expected) {
            assert!((height - expected).abs() < 1e-3, "{tall:?}");
        }
        assert_eq!(
            flexible(&[Panel::Rack, Panel::PianoRoll]),
            Some(Panel::PianoRoll)
        );
        assert_eq!(
            stack_heights(&[Panel::Rack, Panel::PianoRoll], rack, mixer, area),
            [222.0, 772.0]
        );
        assert_eq!(flexible(&[Panel::Mixer]), None);
        // The make-room rule still holds the least heights: four panels in 504 px close one.
        let open = Panels {
            rack: true,
            playlist: true,
            piano_roll: true,
            mixer: true,
            browser: false,
            settings: false,
        };
        let closed = closed_to_fit(open, Panel::Mixer, &PanelUse::default(), 108.0, 504.0);
        assert_eq!(closed.len(), 1);
    }

    // Settings is a window of its own, no panel of the stack. Open,
    // it is none of the stacked panels; the stack's least heights, its make-room rule and the
    // playlist taking the rest are as without it (the starter rack over the playlist in the
    // 986 px a maximized 1920 x 1080 window has under its toolbar and hint bar); opening it
    // closes no panel however short the stack is, and closing it brings none back.
    #[test]
    fn settings_take_no_room_of_the_stack() {
        let mut open = Panels::FIRST_START;
        let without: Vec<Panel> = open.stacked().collect();
        open.settings = true;
        let with: Vec<Panel> = open.stacked().collect();
        assert_eq!(with, without);
        assert_eq!(with, [Panel::Rack, Panel::Playlist]);
        assert!(!Panel::STACKED.contains(&Panel::Settings));
        assert_eq!(stack_heights(&with, 222.0, &NATURAL, 986.0), [222.0, 764.0]);
        assert_eq!(least_total(&with, 222.0), 70.0 + MIN_HEIGHT);
        let used = PanelUse::default();
        assert!(closed_to_fit(open, Panel::Settings, &used, 222.0, 100.0).is_empty());
        let mut displaced = Displaced::default();
        assert!(displaced.take(Panel::Settings).is_empty());
        let mut touched = used.clone();
        touched.touch(Panel::Settings);
        assert_eq!(touched, used, "using it changes no stacked panel's turn");
    }

    // Resizable dividers, known answers by hand: the playlist, the piano roll and the mixer in
    // a 1000 × 1000 window (936 px under the toolbar, no hint bar), nothing kept. The roll its
    // octave and its band (314), the mixer its 400, the playlist the 222 left. The line
    // between the playlist and the roll dragged 50 px down: the playlist (the flexible panel)
    // grows to 272 and the roll keeps 264; 100 down stops at the roll's least (230, the
    // playlist 306); 200 up at the playlist's (180, the roll 356). The line between the roll
    // and the mixer, neither flexible, dragged 20 px up from there (roll 264): both keep what
    // they show, 244 and 420, the playlist as it was; 400 down stops at the mixer's least. (Pins
    // moved: 928 px under the 72 px toolbar, the playlist 8 px shorter.)
    #[test]
    fn a_divider_moves_the_line_between_two_panels() {
        let stacked = [Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        let area = 1000.0 - crate::ui::layout::TOOLBAR_HEIGHT;
        assert_eq!(area, 936.0);
        assert_eq!(
            stack_heights(&stacked, 0.0, &NATURAL, area),
            [222.0, 314.0, 400.0]
        );
        let drag = |kept: &Kept, lower: usize, moved: f32| {
            let kept = dragged(&stacked, 0.0, kept, area, lower, moved);
            (kept, stack_heights(&stacked, 0.0, &kept, area))
        };
        let (down, heights) = drag(&NATURAL, 1, 50.0);
        assert_eq!(heights, [272.0, 264.0, 400.0]);
        assert_eq!(
            down,
            Kept {
                piano_roll: Some(264.0),
                ..NATURAL
            },
            "the playlist keeps no height: it takes what is left"
        );
        assert_eq!(drag(&NATURAL, 1, 100.0).1, [306.0, 230.0, 400.0]);
        assert_eq!(drag(&NATURAL, 1, -200.0).1, [180.0, 356.0, 400.0]);
        let (up, heights) = drag(&down, 2, -20.0);
        assert_eq!(heights, [272.0, 244.0, 420.0]);
        assert_eq!(
            up,
            Kept {
                piano_roll: Some(244.0),
                mixer: Some(420.0),
                ..NATURAL
            }
        );
        assert_eq!(drag(&down, 2, 400.0).1, [272.0, 484.0, 180.0]);
        // The divider over the top panel, or past the stack, is none: nothing moves.
        assert_eq!(drag(&down, 0, 30.0).0, down);
        assert_eq!(drag(&down, 3, 30.0).0, down);
        // A double-click on the line between the roll and the mixer puts both back; on the
        // line over the roll, the roll alone (the playlist keeps none).
        assert_eq!(reset(&stacked, &up, 2), NATURAL);
        assert_eq!(
            reset(&stacked, &up, 1),
            Kept {
                mixer: Some(420.0),
                ..NATURAL
            }
        );
        assert_eq!(
            stack_heights(&stacked, 0.0, &reset(&stacked, &up, 2), area),
            [222.0, 314.0, 400.0]
        );
    }

    // With the playlist closed the piano roll is the flexible panel: the starter rack (222)
    // over the roll and the mixer, maximized on 1920 × 1080 with the hint bar (986 px): 222,
    // 364, 400. The line between the rack and the roll dragged 40 px down keeps the rack 262,
    // the roll takes the 324 left; the line between the roll and the mixer dragged 100 px up
    // keeps the mixer 500, the roll 264. A double-click on that line puts the mixer back and
    // leaves the roll's own kept height (for when the playlist is open) as it was. With the
    // rack and the mixer alone the rack is the flexible one: a rack of 254 px of content over
    // the mixer in 488 px (the mixer's handle test: the rack shows 88), its
    // line dragged 40 px up keeps the mixer 418 (the rack at its least, 70), 200 down 200, 300
    // down 180 (the mixer's least).
    #[test]
    fn without_the_playlist_the_flexible_panel_absorbs_the_line() {
        let area = 986.0;
        let stacked = [Panel::Rack, Panel::PianoRoll, Panel::Mixer];
        assert_eq!(
            stack_heights(&stacked, 222.0, &NATURAL, area),
            [222.0, 364.0, 400.0]
        );
        let rack = dragged(&stacked, 222.0, &NATURAL, area, 1, 40.0);
        assert_eq!(
            rack,
            Kept {
                rack: Some(262.0),
                ..NATURAL
            }
        );
        assert_eq!(
            stack_heights(&stacked, 222.0, &rack, area),
            [262.0, 324.0, 400.0]
        );
        let roll_kept = Kept {
            piano_roll: Some(300.0),
            ..NATURAL
        };
        let mixer = dragged(&stacked, 222.0, &roll_kept, area, 2, -100.0);
        assert_eq!(
            mixer,
            Kept {
                piano_roll: Some(300.0),
                mixer: Some(500.0),
                ..NATURAL
            },
            "the roll keeps no new height while it is the flexible one"
        );
        assert_eq!(
            stack_heights(&stacked, 222.0, &mixer, area),
            [222.0, 264.0, 500.0]
        );
        assert_eq!(reset(&stacked, &mixer, 2), roll_kept);
        let two = [Panel::Rack, Panel::Mixer];
        let rack = RACK_HEADER_HEIGHT + 46.0 * 5.0;
        assert_eq!(stack_heights(&two, rack, &NATURAL, 488.0), [88.0, 400.0]);
        for (moved, mixer) in [(-40.0, 418.0), (200.0, 200.0), (300.0, 180.0)] {
            let kept = dragged(&two, rack, &NATURAL, 488.0, 1, moved);
            assert_eq!(kept.mixer, Some(mixer), "moved {moved}");
            assert_eq!(kept.rack, None);
        }
    }

    // The window too small for every least height (the four panels in 504 px, 660 px of least
    // heights): the stack is squeezed as before and a divider moves nothing. Made smaller
    // with heights kept (the playlist over the roll kept 406 and the mixer 400, 700 px): the
    // two give up the same share above their least, (520 − 410) / (806 − 410), 278.89 and
    // 241.11, the playlist its 180; grown back, the kept heights come back. A drag in the
    // squeezed stack starts from what it shows: the line between the roll and the mixer 20 px
    // up, the roll 259 (whole pixels) and the mixer the 261 the two had left.
    #[test]
    fn a_window_too_small_squeezes_kept_heights_and_gives_them_back() {
        let four = [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        assert_eq!(dragged(&four, 222.0, &NATURAL, 504.0, 2, -40.0), NATURAL);
        let stacked = [Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        let kept = Kept {
            piano_roll: Some(406.0),
            mixer: Some(400.0),
            ..NATURAL
        };
        assert_eq!(
            stack_heights(&stacked, 0.0, &kept, 986.0),
            [180.0, 406.0, 400.0]
        );
        let share = 110.0 / 396.0;
        let small = stack_heights(&stacked, 0.0, &kept, 700.0);
        let expected = [180.0, 230.0 + 176.0 * share, 180.0 + 220.0 * share];
        assert!(near(&small, &expected), "{small:?}");
        assert_eq!(
            stack_heights(&stacked, 0.0, &kept, 986.0),
            [180.0, 406.0, 400.0]
        );
        let moved = dragged(&stacked, 0.0, &kept, 700.0, 2, -20.0);
        assert_eq!(moved.piano_roll, Some(259.0));
        let heights = stack_heights(&stacked, 0.0, &moved, 700.0);
        assert!(near(&heights, &[180.0, 259.0, 261.0]), "{heights:?}");
    }

    #[test]
    fn function_keys_toggle_one_panel_each() {
        let mut panels = Panels::default();
        assert!(panels.rack && !panels.piano_roll);
        panels.toggle(Panel::PianoRoll);
        assert!(panels.rack && panels.piano_roll);
        panels.toggle(Panel::Rack);
        assert_eq!(
            panels,
            Panels {
                rack: false,
                playlist: false,
                piano_roll: true,
                mixer: false,
                browser: false,
                settings: false,
            }
        );
        assert_eq!(Panel::from_name("piano-roll"), Some(Panel::PianoRoll));
        assert_eq!(Panel::from_name("rack"), Some(Panel::Rack));
        // A name no panel has is nothing.
        assert_eq!(Panel::from_name("mixer"), Some(Panel::Mixer));
        assert_eq!(Panel::from_name("mixers"), None);
    }

    #[test]
    fn f5_toggles_the_playlist_alone() {
        let mut panels = Panels::default();
        panels.toggle(Panel::Playlist);
        assert!(panels.rack && panels.playlist && !panels.piano_roll);
        panels.toggle(Panel::Playlist);
        assert_eq!(panels, Panels::default());
        assert_eq!(Panel::from_name("playlist"), Some(Panel::Playlist));
    }

    #[test]
    fn f8_toggles_the_browser_alone() {
        let mut panels = Panels::default();
        panels.toggle(Panel::Browser);
        assert!(panels.rack && panels.browser && !panels.playlist && !panels.mixer);
        assert_eq!(Panel::from_name("browser"), Some(Panel::Browser));
        panels.toggle(Panel::Browser);
        assert_eq!(panels, Panels::default());
    }

    #[test]
    fn f10_toggles_the_settings_alone() {
        let mut panels = Panels::default();
        panels.toggle(Panel::Settings);
        assert!(panels.rack && panels.settings && !panels.mixer);
        assert_eq!(Panel::from_name("settings"), Some(Panel::Settings));
        panels.toggle(Panel::Settings);
        assert_eq!(panels, Panels::default());
    }

    // Every panel's command-line name reads back as that panel, each name once.
    #[test]
    fn every_panel_reads_back_from_its_name() {
        for panel in Panel::ALL {
            assert_eq!(Panel::from_name(panel.name()), Some(panel));
        }
        let mut names: Vec<&str> = Panel::ALL.iter().map(|panel| panel.name()).collect();
        names.dedup();
        assert_eq!(names.len(), 6);
        let open: Vec<Panel> = Panels::FIRST_START.open().collect();
        assert_eq!(open, [Panel::Browser, Panel::Rack, Panel::Playlist]);
    }

    #[test]
    fn f9_toggles_the_mixer_alone() {
        let mut panels = Panels::default();
        panels.toggle(Panel::Mixer);
        assert!(panels.rack && panels.mixer && !panels.playlist && !panels.piano_roll);
        panels.toggle(Panel::Mixer);
        assert_eq!(panels, Panels::default());
    }
}
