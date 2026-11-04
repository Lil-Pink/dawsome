//! The main window's panels. The Library is a column down the left; the other open panels stack
//! under the toolbar to its right in a fixed order (rack, playlist, piano roll, mixer). Each
//! function key toggles one. Settings is no panel of the main window's: its key, digit and
//! button toggle the Settings window (it once stacked under the mixer, where its
//! lower options could not be scrolled to and nothing closed it but its key).
//!
//! Stacked panels keep a least height each ([`min_height`]; the mixer's is every strip whole
//! and a line of its effects' blocks): when a stacked panel just opened leaves the stack no
//! room for every open panel's, the most recently used of the others stay while they fit and
//! the rest close ([`closed_to_fit`]); a stack that is too short all the same (a window made
//! smaller, a saved layout or a scripted run's panels in a short window) shows every panel at
//! its least height and scrolls ([`stack_heights`], [`fits`]). Why scroll, not squeeze: a
//! squeezed mixer scrolled its strips' lower controls and its add menu out of the window.
//! With room, the least heights come first; one panel takes what the others
//! leave ([`flexible`]: the playlist, else the piano roll, else the rack), the others their
//! kept heights, else their natural ones ([`natural_height`]), each giving up the same share
//! of what it has above its least when the flexible one would get less than its own. Beside the
//! mixer the playlist and the piano roll share what the mixer's default gave up
//! ([`given_up_to`]): in the roll's natural height, and while the stack keeps its default
//! heights, as the two's floors before the others share what is left.
//!
//! Every stacked panel under another starts with a divider ([`DIVIDER_BAND`], inside its
//! height): dragged, the line between the two moves ([`dragged`]) and the heights it leaves are
//! kept ([`Kept`], in the settings); a double-click puts the two back ([`reset`]). The mixer's
//! top edge is that band even alone, where nothing drags it (its desk is laid out under it).
//! Dragged sideways instead, a divider pops the panel under it out of the stack into a window
//! of its own ([`Popped`], `App::divider_message`): the panel stays open, the stack lays out
//! the others as though it were closed, and closing its window puts it back.

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

    /// Its place in [`Panel::STACKED`]; none for the Library and Settings.
    pub fn stacked_index(self) -> Option<usize> {
        Panel::STACKED.iter().position(|&stacked| stacked == self)
    }
}

/// The stacked panels popped out of the main window, each shown in a window of its own: open,
/// but out of the stack. The Library and Settings are never popped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Popped([bool; 4]);

impl Popped {
    /// No panel popped.
    pub const NONE: Self = Self([false; 4]);

    /// Whether `panel` is popped out.
    pub fn has(self, panel: Panel) -> bool {
        panel.stacked_index().is_some_and(|at| self.0[at])
    }

    /// Pops `panel` out, or puts it back; the Library and Settings stay as they are.
    pub fn set(&mut self, panel: Panel, popped: bool) {
        if let Some(at) = panel.stacked_index() {
            self.0[at] = popped;
        }
    }

    /// The panels popped out, top to bottom as they stack.
    pub fn panels(self) -> impl Iterator<Item = Panel> {
        Panel::STACKED
            .into_iter()
            .filter(move |&panel| self.has(panel))
    }
}

/// Which panels are open, and which of them are popped out of the stack. The window opens with
/// the rack alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panels {
    pub rack: bool,
    pub playlist: bool,
    pub piano_roll: bool,
    pub mixer: bool,
    pub browser: bool,
    pub settings: bool,
    /// The open stacked panels shown in windows of their own instead of the stack.
    pub popped: Popped,
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
            popped: Popped::NONE,
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
        popped: Popped::NONE,
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

    /// Opens or closes `panel`; a panel closed is popped out no more.
    pub fn set(&mut self, panel: Panel, open: bool) {
        if !open {
            self.popped.set(panel, false);
        }
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

    /// The stacked panels open in the stack, top to bottom ([`Panel::STACKED`]'s order): the
    /// popped ones are left out.
    pub fn stacked(self) -> impl Iterator<Item = Panel> {
        Panel::STACKED
            .into_iter()
            .filter(move |&panel| self.in_stack(panel))
    }

    /// Whether `panel` is open in the stack: open and not popped out.
    pub fn in_stack(self, panel: Panel) -> bool {
        self.is_open(panel) && !self.popped.has(panel)
    }

    /// Pops the open stacked panel `panel` out of the stack; any other stays as it is.
    pub fn pop_out(&mut self, panel: Panel) {
        if self.in_stack(panel) {
            self.popped.set(panel, true);
        }
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
        popped: Popped::NONE,
    };
}

/// The least height of the playlist: room for its header and a few rows of what it shows.
/// The rack, the piano roll and the mixer have their own ([`min_height`]).
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
/// header), or its content when that is shorter; the piano roll [`MIN_ROLL_HEIGHT`]; the mixer
/// [`mixer_layout::LEAST_HEIGHT`] (225: its top edge, the header, every compact strip whole
/// with a 50 px fader, and beside the strips a line of the tallest effect block); every other panel
/// [`MIN_HEIGHT`]. Under another panel the piano roll needs its divider's band too
/// ([`least_heights`]).
pub fn min_height(panel: Panel, rack_content: f32) -> f32 {
    match panel {
        Panel::Rack => rack_content.min(RACK_HEADER_HEIGHT + layout::row_top(1)),
        Panel::PianoRoll => MIN_ROLL_HEIGHT,
        Panel::Mixer => mixer_layout::LEAST_HEIGHT,
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

/// What the mixer's default height gave up: 310 px (its least before) less its
/// compact strip's [`mixer_layout::DEFAULT_HEIGHT`], 225: 85 px.
const MIXER_GAVE_UP: f32 = 310.0 - mixer_layout::DEFAULT_HEIGHT;

/// The piano roll's share of [`MIXER_GAVE_UP`] beside the playlist: half, rounded down to a
/// whole pixel, 42 px; the playlist gets the other half and the odd pixel, 43
/// ([`given_up_to`]).
pub const ROLL_SHARE: f32 = (MIXER_GAVE_UP as u32 / 2) as f32;

/// What `panel` of `stacked` gets of [`MIXER_GAVE_UP`] while the mixer is stacked too: the
/// piano roll [`ROLL_SHARE`] and the playlist the rest while both are, either of them alone all
/// of it; nothing without the mixer, and nothing for the rack or the mixer. Why: the room the
/// mixer gave up is meant to make the piano roll and the playlist open bigger, half each, the
/// playlist the bigger half when it does not split evenly. Why
/// only beside the mixer: without it nothing was given up, and a roll taller there would take
/// its pixels from the playlist.
fn given_up_to(panel: Panel, stacked: &[Panel]) -> f32 {
    let has = |other: Panel| stacked.contains(&other);
    if !has(Panel::Mixer) {
        return 0.0;
    }
    match panel {
        Panel::PianoRoll if has(Panel::Playlist) => ROLL_SHARE,
        Panel::Playlist if has(Panel::PianoRoll) => MIXER_GAVE_UP - ROLL_SHARE,
        Panel::PianoRoll | Panel::Playlist => MIXER_GAVE_UP,
        Panel::Rack | Panel::Mixer | Panel::Browser | Panel::Settings => 0.0,
    }
}

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

    /// The mixer's height: as a divider left it, else [`mixer_layout::DEFAULT_HEIGHT`]. The
    /// stack shows it this tall while it has room ([`natural_height`]); its desk is laid out
    /// from the height the stack shows ([`stack_heights`]).
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

/// The height `panel` takes among `stacked` (in any order) in an area `area` tall when another
/// panel is the flexible one, a divider over it when another panel stacks above it: the height
/// kept for it ([`Kept`]), else the rack its content up to half the area, the piano roll its
/// strip, an octave of rows and the room under its grid ([`UNDER_GRID`]) (and its band under a
/// divider), and its part of what the mixer gave up beside it ([`given_up_to`]), the mixer
/// [`mixer_layout::DEFAULT_HEIGHT`]; never less than its least height.
pub fn natural_height(
    panel: Panel,
    stacked: &[Panel],
    rack_content: f32,
    kept: &Kept,
    area: f32,
) -> f32 {
    let band = divider_room(panel, under_another(panel, stacked));
    let beside_mixer = given_up_to(panel, stacked);
    let natural = kept
        .of(panel)
        .filter(|height| height.is_finite())
        .unwrap_or_else(|| match panel {
            Panel::Rack => rack_height_above_roll(rack_content, area),
            Panel::PianoRoll => {
                STRIP_HEIGHT + NATURAL_ROLL_ROWS * ROW_HEIGHT + UNDER_GRID + beside_mixer + band
            }
            Panel::Mixer => mixer_layout::DEFAULT_HEIGHT,
            Panel::Playlist | Panel::Browser | Panel::Settings => 0.0,
        });
    natural.max(min_height(panel, rack_content) + band)
}

/// Whether the stacked panels `stacked` all get their least height in an area `area` tall;
/// when not, the stack scrolls ([`stack_heights`]).
pub fn fits(stacked: &[Panel], rack_content: f32, area: f32) -> bool {
    least_total(stacked, rack_content) <= area
}

fn least_total(stacked: &[Panel], rack_content: f32) -> f32 {
    least_heights(stacked, rack_content).iter().sum()
}

/// The height of each of the stacked panels `stacked` (top to bottom) in an area `area` tall,
/// the rack's content being `rack_content` tall and the dividers having left `kept`, in their
/// order; they add up to `area` when they [`fits`].
///
/// When they fit, the [`flexible`] panel takes what the others leave, and they their kept or
/// natural heights ([`natural_height`]); when those leave it less than its floor, each of the
/// others gives up the same share of what it has above its own floor, so the flexible one keeps
/// its floor. A panel's floor is its least height, and while no panel of the stack (but the
/// flexible one) keeps a height, the playlist's and the piano roll's floors hold their parts of
/// what the mixer gave up beside them ([`given_up_to`]), as much of it as the area holds over
/// the least heights, both by the same fraction. Why the floors: squeezed (four panels at
/// 1600 × 900), the others' shares took the mixer's room to the rack, and the playlist stayed
/// at its least; with them the playlist and the roll get 43 and 42 px more than they had beside
/// the 310 px mixer, the rack the same. Why only with no height kept: a divider dragged sets
/// heights by hand, down to the least heights. A rack alone takes the whole area, and so does
/// the mixer alone.
///
/// When they do not fit, each takes its least height: they add up to more than `area`, and the
/// stack scrolls (`App::panels_view`). Why not squeeze them into the area: under its least the
/// mixer scrolled its strips' lower controls out of view, and its add menu with them.
pub fn stack_heights(stacked: &[Panel], rack_content: f32, kept: &Kept, area: f32) -> Vec<f32> {
    let least = least_heights(stacked, rack_content);
    let total: f32 = least.iter().sum();
    if total > area {
        return least;
    }
    let Some(flex) = flexible(stacked) else {
        let mut heights = least.clone();
        share(&mut heights, &least, area);
        return heights;
    };
    let others = |panel: Panel| panel != flex;
    let defaults = stacked
        .iter()
        .all(|&panel| !others(panel) || kept.of(panel).is_none());
    let given: Vec<f32> = stacked
        .iter()
        .map(|&panel| {
            if defaults {
                given_up_to(panel, stacked)
            } else {
                0.0
            }
        })
        .collect();
    let extra: f32 = given.iter().sum();
    let part = if extra > 0.0 {
        ((area - total) / extra).min(1.0)
    } else {
        0.0
    };
    let floor: Vec<f32> = least
        .iter()
        .zip(&given)
        .map(|(least, given)| least + given * part)
        .collect();
    let natural: Vec<f32> = stacked
        .iter()
        .map(|&panel| {
            if others(panel) {
                natural_height(panel, stacked, rack_content, kept, area)
            } else {
                0.0
            }
        })
        .collect();
    let flex_floor = stacked
        .iter()
        .zip(&floor)
        .find(|(&panel, _)| panel == flex)
        .map_or(0.0, |(_, &floor)| floor);
    let room = area - flex_floor;
    let natural_total: f32 = natural.iter().sum();
    let floor_total = floor.iter().sum::<f32>() - flex_floor;
    // What each of the others keeps of its height above its floor: all of it with room, else
    // the same share each, so the flexible panel keeps its floor.
    let kept_share = if natural_total <= room || natural_total <= floor_total {
        1.0
    } else {
        (room - floor_total) / (natural_total - floor_total)
    };
    let mut heights: Vec<f32> = stacked
        .iter()
        .zip(natural.iter().zip(&floor))
        .map(|(&panel, (&natural, &floor))| {
            if others(panel) {
                floor + (natural - floor) * kept_share
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
        let natural = natural_height(panel, stacked, rack_content, kept, area);
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
/// the stack, and neither do the panels popped out of it.
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
        if panel == opened || !open.in_stack(panel) {
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
    // a 1000 × 560 window (504 px under the toolbar): their least heights (70, 180, 230, 225)
    // add up to 705 px, so each shows at its least and the stack scrolls: the roll keeps its
    // 230 px and its grid rows (nine over the 10 px scroll bar, the lane's 6 px handle and the
    // lane's default 64 px), the mixer its 225, every strip whole. At the top of the stack the
    // roll has no divider over it: its least is 224, so with the mixer under it 449 fit and
    // 448 do not. The mixer alone in less than its 225 shows its 225 and scrolls too. (Pins
    // moved: the mixer's least was 180, and a stack too short was squeezed into the
    // area, the roll keeping its 230 and the rest's 274 px going 70 : 180 : 180 to the others,
    // 44.60, 114.70 and 114.70 px, which scrolled the mixer's strips' foot out of view; then
    // 310 until its compact strip: 790 px, 534 and 533, alone in 300.)
    #[test]
    fn a_stack_too_short_shows_every_panel_at_its_least_and_scrolls() {
        assert_eq!(MIN_ROLL_HEIGHT, 224.0);
        let stacked = [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        assert!(!fits(&stacked, 222.0, 504.0));
        let heights = stack_heights(&stacked, 222.0, &MIXER, 504.0);
        assert_eq!(heights, [70.0, 180.0, 230.0, 225.0]);
        assert_eq!(roll_rows(heights[2]), 9);
        assert_eq!(
            heights.iter().sum::<f32>(),
            705.0,
            "past the area: it scrolls"
        );
        assert_eq!(
            stack_heights(&[Panel::PianoRoll, Panel::Mixer], 0.0, &MIXER, 200.0),
            [224.0, 225.0]
        );
        assert!(fits(&[Panel::PianoRoll, Panel::Mixer], 0.0, 449.0));
        assert!(!fits(&[Panel::PianoRoll, Panel::Mixer], 0.0, 448.0));
        assert!(!fits(&[Panel::Playlist, Panel::PianoRoll], 0.0, 409.0));
        assert_eq!(
            stack_heights(&[Panel::Mixer], 0.0, &NATURAL, 200.0),
            [225.0]
        );
    }

    // By hand, the four panels in a 1600 × 900 window with the hint bar: 900 − 64 − 22 = 814 px
    // under the toolbar, the starter rack (222) and nothing kept. Their least heights, 70 + 180
    // + 230 + 225 = 705, fit with 109 px to spare, more than the 85 the mixer gave up: the
    // playlist's floor is its 180 and its 43, 223, the roll's its 230 and its 42, 272. The
    // playlist keeps its 223; the rack (222), the roll (356: its octave, its 42 and its band)
    // and the mixer (225, its default and its least) would take 803 of the 591 left, so each
    // keeps the same share, 24 / 236, of what it has above its floor (152, 84 and 0): the rack
    // 85.458, the roll 280.542, the mixer 225. Laid out in whole pixels (85, 223, 280 and the
    // 226 left) the mixer starts at 64 + 588 = 652, its desk under its 6 px edge at 658, and
    // its strips end 2 px above the least desk's foot, 658 + 217 = 875, inside the 878 above
    // the hint bar. With the bar hidden, 836 px: the share 46 / 236, the rack 99.627, the roll
    // 288.373. (Before, the mixer's least was 180 and its default 400: it showed
    // 254.3 px of a 394 px desk from y 629, its strips' foot at 1019, cut at the hint bar's
    // 878; then 310 until its compact strip: the rack 85.458, the playlist 180,
    // the roll 238.542, the strips' foot at 873.)
    #[test]
    fn four_panels_at_900_px_keep_their_least_and_the_mixers_strips_whole() {
        use crate::ui::layout::{HINT_BAR_HEIGHT, TOOLBAR_HEIGHT};
        let four = [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        let area = 900.0 - TOOLBAR_HEIGHT - HINT_BAR_HEIGHT;
        assert_eq!(area, 814.0);
        assert_eq!(least_total(&four, 222.0), 705.0);
        assert!(fits(&four, 222.0, area));
        let heights = stack_heights(&four, 222.0, &NATURAL, area);
        let share = 24.0 / 236.0;
        let expected = [70.0 + 152.0 * share, 223.0, 272.0 + 84.0 * share, 225.0];
        assert!(near(&heights, &expected), "{heights:?}");
        assert!(
            near(&heights, &[85.458, 223.0, 280.542, 225.0]),
            "{heights:?}"
        );
        for ((&panel, height), least) in four.iter().zip(&heights).zip(least_heights(&four, 222.0))
        {
            assert!(
                *height >= least - 1e-3,
                "{panel:?} {height} under its least {least}"
            );
        }
        let above: f32 = heights[..3].iter().map(|height| height.floor()).sum();
        let desk_top = TOOLBAR_HEIGHT + above + DIVIDER_BAND;
        assert_eq!(desk_top, 658.0);
        let desk = mixer_layout::desk_height(heights[3]);
        assert_eq!(desk, mixer_layout::LEAST_DESK);
        assert_eq!(desk_top + mixer_layout::body_bottom(desk), 875.0);
        assert!(desk_top + mixer_layout::body_bottom(desk) <= 900.0 - HINT_BAR_HEIGHT);
        let hidden = stack_heights(&four, 222.0, &NATURAL, 836.0);
        let share = 46.0 / 236.0;
        let expected = [70.0 + 152.0 * share, 223.0, 272.0 + 84.0 * share, 225.0];
        assert!(near(&hidden, &expected), "{hidden:?}");
        assert!(
            near(&hidden, &[99.627, 223.0, 288.373, 225.0]),
            "{hidden:?}"
        );
    }

    // The mixer opens 50% shorter, and the room it gives up makes
    // the piano roll and the playlist open bigger, half each, the playlist the odd pixel. 50% of
    // 310 is 155, which holds no whole strip, so the mixer opens at its compact strip's least,
    // 225: 85 px given up, 42 to the roll (ROLL_SHARE) and 43 to the playlist. By hand, the four
    // panels with nothing kept in a window with room for every natural height, before and now:
    // 1920 × 1200 with the hint bar (1200 − 64 − 22 = 1114 px) and the starter rack (222):
    // before the rack 222, the roll 314 (its strip, an octave of rows, the 136 px under its grid
    // and its band), the mixer 310 and the playlist the 268 left; now the rack 222, the roll
    // 356, the mixer 225 and the playlist 311. The four fill the same 1114 px; the mixer's 85
    // are the playlist's 43 and the roll's 42, the playlist's gain less the roll's 1. Squeezed,
    // 1600 × 900 with the hint bar (814 px): before the playlist at its least, 180, and the
    // rack, the roll and the mixer each keeping 24 / 236 of what it had above its least, 85.458,
    // 238.542, 310; now the playlist's and the roll's floors hold their 43 and 42 (223, 272) and
    // the three keep the same 24 / 236 above them: 85.458, 223, 280.542, 225, the same gains.
    // The same with the playlist, the roll and the mixer in 1000 × 1000 (936 px, no hint bar):
    // 312, 314, 310 before, 355, 356, 225 now. Where only the least heights fit now (the four
    // need 705 px, 790 before), the two take what the area holds over them, 43 : 42: in 1280 ×
    // 800 with the hint bar (714 px) 9 px of it, the playlist 180 + 43 × 9 / 85 = 184.553 and
    // the roll 230 + 42 × 9 / 85 = 234.447, the rack and the mixer at their least. Without the
    // mixer nothing was given up: the playlist over the roll in 714 px keeps 400 and 314, as
    // before.
    #[test]
    fn the_room_the_mixer_gave_up_goes_half_to_the_roll_and_half_to_the_playlist() {
        assert_eq!(ROLL_SHARE, 42.0);
        assert_eq!(mixer_layout::DEFAULT_HEIGHT, 225.0);
        let four = [Panel::Rack, Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        let before = [222.0, 268.0, 314.0, 310.0];
        let now = stack_heights(&four, 222.0, &NATURAL, 1114.0);
        assert_eq!(now, [222.0, 311.0, 356.0, 225.0]);
        let gain = |panel: usize| now[panel] - before[panel];
        assert_eq!(now.iter().sum::<f32>(), before.iter().sum::<f32>());
        assert_eq!(gain(3), -85.0, "the mixer");
        assert_eq!(gain(0), 0.0, "the rack");
        assert_eq!(gain(1) + gain(2), 85.0);
        let more = gain(1) - gain(2);
        assert!(
            more == 0.0 || more == 1.0,
            "playlist {} roll {}",
            gain(1),
            gain(2)
        );
        let before = [85.458, 180.0, 238.542, 310.0];
        let now = stack_heights(&four, 222.0, &NATURAL, 814.0);
        assert!(near(&now, &[85.458, 223.0, 280.542, 225.0]), "{now:?}");
        let gains: Vec<f32> = now
            .iter()
            .zip(before)
            .map(|(now, before)| now - before)
            .collect();
        assert!(near(&gains, &[0.0, 43.0, 42.0, -85.0]), "{gains:?}");
        let blend = stack_heights(&four, 222.0, &NATURAL, 714.0);
        assert!(near(&blend, &[70.0, 184.553, 234.447, 225.0]), "{blend:?}");
        let three = [Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        let before = [312.0, 314.0, 310.0];
        let now = stack_heights(&three, 0.0, &NATURAL, 936.0);
        assert_eq!(now, [355.0, 356.0, 225.0]);
        assert_eq!((now[0] - before[0], now[1] - before[1]), (43.0, 42.0));
        let two = [Panel::Playlist, Panel::PianoRoll];
        assert_eq!(stack_heights(&two, 0.0, &NATURAL, 714.0), [400.0, 314.0]);
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

    // Opening the mixer over the rack, the playlist and the piano roll in 549 px: 705 px of
    // least heights do not fit, so the most recently used of the others stay while they fit
    // beside it. Used last: the roll, then the mixer; the rack and the playlist least. The roll
    // (449 px with the mixer) and then the rack (525: the roll has a divider over it now, 6 px)
    // fit, the playlist (635 with the roll and the mixer) does not, so it alone closes; with
    // the rack pressed after the roll, the same. (Resizable dividers: 474 and 654 before the
    // roll's band; same answers. The mixer's least grew 130 px, 180 to 310, and the
    // area with it, 504 to 634; then fell 85 px to 225 with its compact strip, and the area with
    // it, to 549, so the answers stand; with room is 705 now, was 790 and 744.)
    #[test]
    fn a_panel_opened_without_room_closes_the_least_recently_used() {
        let mut open = Panels {
            rack: true,
            playlist: true,
            piano_roll: true,
            mixer: true,
            browser: true,
            settings: false,
            popped: Popped::NONE,
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
        let closed = closed_to_fit(open, Panel::Mixer, &used, 108.0, 549.0);
        assert_eq!(closed, [Panel::Playlist]);
        used.touch(Panel::Rack);
        let closed = closed_to_fit(open, Panel::Mixer, &used, 108.0, 549.0);
        assert_eq!(closed, [Panel::Playlist]);
        // With room, nothing closes; and the panel opened never does, even alone too tall.
        assert!(closed_to_fit(open, Panel::Mixer, &used, 108.0, 705.0).is_empty());
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
            popped: Popped::NONE,
        };
        let stacked: Vec<Panel> = open.stacked().collect();
        assert!(!fits(&stacked, 108.0, 364.0));
        let used = PanelUse::default();
        assert!(closed_to_fit(open, Panel::Browser, &used, 108.0, 364.0).is_empty());
    }

    // Beside the panel opened, the most recently used of the others stay
    // open while they fit, so a small panel used long ago stays when it fits. The stacked
    // panels opened in turn in 549 px (least heights: rack 70, playlist 180, piano roll 224,
    // and 230 under another panel with its divider, mixer 225): the playlist and the roll fit
    // (480); the mixer keeps the roll (449) and the rack (525) but not the playlist (635).
    // Settings then closes nothing (it kept the mixer and the rack and closed
    // the roll while it was stacked, 180 px). (504 px with the mixer's old least of
    // 180, then 634 with 310; the area follows it, so the answers stand.)
    #[test]
    fn the_most_recently_used_panels_that_fit_stay_open() {
        let mut open = Panels::default();
        let mut used = PanelUse::default();
        let mut open_in_turn = |panel: Panel| {
            open.set(panel, true);
            let closed = closed_to_fit(open, panel, &used, 108.0, 549.0);
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
    // rack (222) over the playlist leaves it 772; with the mixer under them it keeps its 225 px
    // and the playlist 547 (no roll to share the mixer's 85 px with); with the piano roll
    // instead of the playlist, the roll takes what the
    // rack leaves; the mixer and the settings alone share as before. (Pins moved:
    // 1002 px under the 56 px toolbar gave 780, and the 504 px mixer left 276; 986 under the
    // 72 px one gave 764 and 364; the 400 px default mixer left 372, the 310 px
    // one 462.) The mixer's divider dragged to 600 px leaves the playlist 172, under its least,
    // so the rack and the mixer give up the same share of their height above their least (70
    // and 225): (994 − 180 − 295) / (822 − 295) of it, the rack 219.693 and the mixer 594.307
    // (with the mixer's least of 180, (994 − 180 − 250) / (822 − 250); of 310, of 380). The
    // make-room rule's area follows the mixer's least: 549 px (504 with 180, 634 with 310).
    #[test]
    fn the_playlist_takes_the_height_the_others_leave() {
        let area = 1080.0 - crate::ui::layout::TOOLBAR_HEIGHT - crate::ui::layout::HINT_BAR_HEIGHT;
        assert_eq!(area, 994.0);
        let rack = 222.0;
        let mixer = &NATURAL;
        assert_eq!(mixer.mixer_height(), 225.0);
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
            [222.0, 547.0, 225.0]
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
        let kept = (994.0 - 180.0 - 295.0) / (822.0 - 295.0);
        let expected = [70.0 + 152.0 * kept, 180.0, 225.0 + 375.0 * kept];
        for (height, expected) in tall.iter().zip(expected) {
            assert!((height - expected).abs() < 1e-3, "{tall:?}");
        }
        assert!(near(&tall, &[219.693, 180.0, 594.307]), "{tall:?}");
        assert_eq!(
            flexible(&[Panel::Rack, Panel::PianoRoll]),
            Some(Panel::PianoRoll)
        );
        assert_eq!(
            stack_heights(&[Panel::Rack, Panel::PianoRoll], rack, mixer, area),
            [222.0, 772.0]
        );
        assert_eq!(flexible(&[Panel::Mixer]), None);
        // The make-room rule still holds the least heights: four panels in 549 px close one
        // (504 px with the mixer's least of 180, 634 with 310).
        let open = Panels {
            rack: true,
            playlist: true,
            piano_roll: true,
            mixer: true,
            browser: false,
            settings: false,
            popped: Popped::NONE,
        };
        let closed = closed_to_fit(open, Panel::Mixer, &PanelUse::default(), 108.0, 549.0);
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
    // octave, its 42 px share beside the mixer and its band (356), the mixer its 225, the
    // playlist the 355 left. The line between the playlist and the roll dragged 50 px down: the
    // playlist (the flexible panel) grows to 405 and the roll keeps 306; 150 down stops at the
    // roll's least (230, the playlist 481); 200 up at the playlist's (180, the roll 531). The
    // line between the roll and the mixer, neither flexible, dragged 20 px up from there (roll
    // 306): both keep what they show, 286 and 245, the playlist as it was; 400 down stops at the
    // mixer's least, 225, the roll back at 306. (Pins moved: 928 px under the 72 px toolbar, the
    // playlist 8 px shorter; the mixer's default 400 → 310, the playlist 90 px
    // taller, and its least 180 → 310; then both 310 → 225 with its compact strip, the playlist
    // 43 px taller and the roll 42: were 312, 314, 310, 362, 264, 396, 446, 244, 330.)
    #[test]
    fn a_divider_moves_the_line_between_two_panels() {
        let stacked = [Panel::Playlist, Panel::PianoRoll, Panel::Mixer];
        let area = 1000.0 - crate::ui::layout::TOOLBAR_HEIGHT;
        assert_eq!(area, 936.0);
        assert_eq!(
            stack_heights(&stacked, 0.0, &NATURAL, area),
            [355.0, 356.0, 225.0]
        );
        let drag = |kept: &Kept, lower: usize, moved: f32| {
            let kept = dragged(&stacked, 0.0, kept, area, lower, moved);
            (kept, stack_heights(&stacked, 0.0, &kept, area))
        };
        let (down, heights) = drag(&NATURAL, 1, 50.0);
        assert_eq!(heights, [405.0, 306.0, 225.0]);
        assert_eq!(
            down,
            Kept {
                piano_roll: Some(306.0),
                ..NATURAL
            },
            "the playlist keeps no height: it takes what is left"
        );
        assert_eq!(drag(&NATURAL, 1, 150.0).1, [481.0, 230.0, 225.0]);
        assert_eq!(drag(&NATURAL, 1, -200.0).1, [180.0, 531.0, 225.0]);
        let (up, heights) = drag(&down, 2, -20.0);
        assert_eq!(heights, [405.0, 286.0, 245.0]);
        assert_eq!(
            up,
            Kept {
                piano_roll: Some(286.0),
                mixer: Some(245.0),
                ..NATURAL
            }
        );
        assert_eq!(drag(&down, 2, 400.0).1, [405.0, 306.0, 225.0]);
        // The divider over the top panel, or past the stack, is none: nothing moves.
        assert_eq!(drag(&down, 0, 30.0).0, down);
        assert_eq!(drag(&down, 3, 30.0).0, down);
        // A double-click on the line between the roll and the mixer puts both back; on the
        // line over the roll, the roll alone (the playlist keeps none).
        assert_eq!(reset(&stacked, &up, 2), NATURAL);
        assert_eq!(
            reset(&stacked, &up, 1),
            Kept {
                mixer: Some(245.0),
                ..NATURAL
            }
        );
        assert_eq!(
            stack_heights(&stacked, 0.0, &reset(&stacked, &up, 2), area),
            [355.0, 356.0, 225.0]
        );
    }

    // With the playlist closed the piano roll is the flexible panel: the starter rack (222)
    // over the roll and the mixer, maximized on 1920 × 1080 with the hint bar (986 px): 222,
    // 539, 225. The line between the rack and the roll dragged 40 px down keeps the rack 262,
    // the roll takes the 499 left; the line between the roll and the mixer dragged 100 px up
    // keeps the mixer 325, the roll 439. A double-click on that line puts the mixer back and
    // leaves the roll's own kept height (for when the playlist is open) as it was. With the
    // rack and the mixer alone the rack is the flexible one: a rack of 254 px of content over
    // the mixer in 488 px (the mixer's divider test: the rack shows 263), its line dragged
    // 40 px up keeps the mixer 265, 120 up 345, 200 up 418 (the rack at its least, 70), 200
    // down 225 (the mixer's least). (Pins moved: the mixer's default 400 → 310, its
    // least 180 → 310; were 364, 324, 500 and 264, the rack 88, and 418, 200 and 180 for 40 up,
    // 200 and 300 down; then 310 → 225 with its compact strip: were 454, 414, 410, 354, the
    // rack 178, and 350, 418 and 310 for 40 up, 120 up and 200 down.)
    #[test]
    fn without_the_playlist_the_flexible_panel_absorbs_the_line() {
        let area = 986.0;
        let stacked = [Panel::Rack, Panel::PianoRoll, Panel::Mixer];
        assert_eq!(
            stack_heights(&stacked, 222.0, &NATURAL, area),
            [222.0, 539.0, 225.0]
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
            [262.0, 499.0, 225.0]
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
                mixer: Some(325.0),
                ..NATURAL
            },
            "the roll keeps no new height while it is the flexible one"
        );
        assert_eq!(
            stack_heights(&stacked, 222.0, &mixer, area),
            [222.0, 439.0, 325.0]
        );
        assert_eq!(reset(&stacked, &mixer, 2), roll_kept);
        let two = [Panel::Rack, Panel::Mixer];
        let rack = RACK_HEADER_HEIGHT + 46.0 * 5.0;
        assert_eq!(stack_heights(&two, rack, &NATURAL, 488.0), [263.0, 225.0]);
        for (moved, mixer) in [
            (-40.0, 265.0),
            (-120.0, 345.0),
            (-200.0, 418.0),
            (200.0, 225.0),
        ] {
            let kept = dragged(&two, rack, &NATURAL, 488.0, 1, moved);
            assert_eq!(kept.mixer, Some(mixer), "moved {moved}");
            assert_eq!(kept.rack, None);
        }
    }

    // The window too small for every least height (the four panels in 504 px, 705 px of least
    // heights): the stack scrolls and a divider moves nothing. Made smaller with heights kept
    // (the playlist over the roll kept 406 and the mixer 400, to 800 px, room for the 635 px of
    // least heights): the two give up the same share above their least, (620 − 455) / (806 −
    // 455), 312.735 and 307.265, the playlist its 180; grown back, the kept heights come back. A
    // drag in the squeezed stack starts from what it shows: the line between the roll and the
    // mixer 20 px up, the roll 293 (whole pixels) and the mixer the 327 the two had left.
    // (Pins moved: 700 px with the mixer's old least of 180 gave 278.89 and 241.11,
    // the drag 259 and 261; with its least of 310, 282.93 and 337.07, the drag 263 and 357.)
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
        let share = 165.0 / 351.0;
        let small = stack_heights(&stacked, 0.0, &kept, 800.0);
        let expected = [180.0, 230.0 + 176.0 * share, 225.0 + 175.0 * share];
        assert!(near(&small, &expected), "{small:?}");
        assert!(near(&small, &[180.0, 312.735, 307.265]), "{small:?}");
        assert_eq!(
            stack_heights(&stacked, 0.0, &kept, 986.0),
            [180.0, 406.0, 400.0]
        );
        let moved = dragged(&stacked, 0.0, &kept, 800.0, 2, -20.0);
        assert_eq!(moved.piano_roll, Some(293.0));
        let heights = stack_heights(&stacked, 0.0, &moved, 800.0);
        assert!(near(&heights, &[180.0, 293.0, 327.0]), "{heights:?}");
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
                popped: Popped::NONE,
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
