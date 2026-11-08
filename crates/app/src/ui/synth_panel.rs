//! A synth channel's part of the instrument window: its parameters as the mixer's knob blocks
//! ([`crate::ui::knob_block`]), a block for each heading its factory names, placed as a DJ's
//! booth ([`booth`]): a deck either side (Osc 1 over Unison, Osc 2 over the LFO) and the mixer
//! between them (Filter, Filter env, Amp env and Play, one under another), always so, however
//! wide the window. Under the left deck its LEVEL (the strip's, which the window draws), under
//! the right deck its presets: the list (the factory's, then the user's) and Save preset…, whose
//! name box and question whether to replace a preset show in their module, as the window has no
//! dialogs of its own.
//!
//! The window is drawn as a little sound board ([`FINISH`]): the blocks are a console's raised
//! modules, laid out and hit as the mixer's flat ones are.
//!
//! The knobs answer as the mixer's do: a drag sideways moves one (Shift: fine), a double-click
//! sets its default, a choice's button steps to its next value, a right-click opens the
//! automation menu (an automation clip, a controller link). An automated knob shows the value
//! its automation set last and a dot, a linked one a dot of its own. The canvas only says which
//! knob a press found; the app makes the edit or runs the drag
//! ([`crate::ui::mixer_state::MixerDrag::Instrument`]).

use std::fmt;
use std::ops::Range;
use std::time::Instant;

use dawsome_engine::processor::ProcessorFactory;
use dawsome_engine::project::ChannelId;
use iced::widget::canvas::{self, Event, Frame, Geometry};
use iced::widget::{button, column, pick_list, pin, row, stack, text, text_input, Id};
use iced::{mouse, Center, Element, Fill, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::ui::console::Finish;
use crate::ui::fonts::size;
use crate::ui::hint::{hover_area, hover_change, Hover, SoundPart};
use crate::ui::instrument_layout::STRIP_HEIGHT;
use crate::ui::instrument_window::InstrumentMessage;
use crate::ui::knob_block::{
    choice_rect, draw_ground, draw_param, grid_cell, grid_knob_at, grid_size, BLOCK_GAP,
    BLOCK_HEAD, BLOCK_INSET,
};
use crate::ui::mixer::{label, DOUBLE_PRESS_DISTANCE, DOUBLE_PRESS_TIME};
use crate::ui::mixer_layout::{LED_OFFSET, LED_RADIUS};
use crate::ui::mixer_state::is_choice;
use crate::ui::style;
use crate::ui::text_fit::fit_width;

/// How the synth's window is drawn: as a console seen from slightly above, its strip and
/// presets on the same chassis as its knobs.
pub const FINISH: Finish = Finish::Console;

/// The headings on the booth's decks, top to bottom: the sound's two makers, each over what
/// thickens or moves it. Every other heading is the mixer's, in the factory's order.
const LEFT_DECK: [&str; 2] = ["Osc 1", "Unison"];
const RIGHT_DECK: [&str; 2] = ["Osc 2", "LFO"];
/// How many knobs a row of a deck's block holds (an oscillator's six or seven in two rows, the
/// two decks' rows knob for knob alike), and of the mixer's: one fewer, so it stands narrower,
/// an envelope's four and Play's still in one row.
pub const DECK_COLUMNS: usize = 5;
pub const MIXER_COLUMNS: usize = 4;
/// The space between a deck and the mixer, wider than between two blocks of one column so the
/// three read as three units.
pub const DECK_GAP: f32 = 12.0;

/// The text box a preset's name is typed in.
pub const NAME_INPUT: &str = "synth-preset-name";

/// The presets' module's heading, a block's as the factory's headings are.
pub const PRESETS_HEADING: &str = "Presets";

/// The space between the presets' lines, and between the controls of one.
const PRESET_SPACING: f32 = 6.0;

/// A heading's name starts where an effect's light does: a heading has no light, nothing to
/// switch on or off.
const HEADING_INSET: f32 = LED_OFFSET - LED_RADIUS;

/// A heading's block on the booth: its place, the run of parameters it shows, and how many
/// knobs a row of it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub rect: Rectangle,
    pub run: Range<usize>,
    pub columns: usize,
}

/// One of the booth's three columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    LeftDeck,
    Mixer,
    RightDeck,
}

impl Column {
    fn of(heading: &str) -> Self {
        if LEFT_DECK.contains(&heading) {
            Column::LeftDeck
        } else if RIGHT_DECK.contains(&heading) {
            Column::RightDeck
        } else {
            Column::Mixer
        }
    }

    fn columns(self) -> usize {
        match self {
            Column::LeftDeck | Column::RightDeck => DECK_COLUMNS,
            Column::Mixer => MIXER_COLUMNS,
        }
    }
}

/// The booth's places: the headings' blocks, the two modules under the decks, and its size.
#[derive(Debug, Clone, PartialEq)]
pub struct Booth {
    /// One per heading of the factory's, in its order.
    pub blocks: Vec<Block>,
    /// The strip's place under the left deck: its LEVEL, one module as wide as the deck.
    pub level: Rectangle,
    /// The presets' module under the right deck.
    pub presets: Rectangle,
    pub size: Size,
}

/// The booth: the blocks of `factory`'s headings in its order, each where [`Column::of`] puts
/// it, from the booth's top left: the left deck, then the mixer [`DECK_GAP`] right of it, then
/// the right deck as far right again, each column's blocks one under another from the top,
/// [`BLOCK_GAP`] apart; under each deck one more module as wide as it, the strip's LEVEL on the
/// left (as tall as the strip) and the presets on the right. The booth's foot is its tallest
/// column's (the left deck's: Dawsome Synth's mixer stands 2 px shorter than a deck with the
/// strip under it), and the mixer's last block and the presets reach down to it, so no chassis
/// shows under a column. Why: a booth whose columns ended at three heights showed bare chassis
/// under both decks. Fixed whatever room it is shown in: a window narrower than it scrolls it.
pub fn booth(factory: &dyn ProcessorFactory) -> Booth {
    let deck = grid_size(DECK_COLUMNS, 1).width;
    let mixer = grid_size(MIXER_COLUMNS, 1).width;
    let left = |column| match column {
        Column::LeftDeck => 0.0,
        Column::Mixer => deck + DECK_GAP,
        Column::RightDeck => deck + DECK_GAP + mixer + DECK_GAP,
    };
    // Where each column's next module starts, and the mixer's last block.
    let mut filled = [0.0f32; 3];
    let mut mixer_last = None;
    let mut blocks = Vec::with_capacity(factory.groups().len());
    for (name, run) in factory.groups() {
        let column = Column::of(name);
        let columns = column.columns();
        let size = grid_size(columns, run.len().div_ceil(columns).max(1));
        let top = &mut filled[column as usize];
        let rect = Rectangle::new(Point::new(left(column), *top), size);
        *top += size.height + BLOCK_GAP;
        if column == Column::Mixer {
            mixer_last = Some(blocks.len());
        }
        blocks.push(Block {
            rect,
            run: run.clone(),
            columns,
        });
    }
    let under = |column: Column| {
        Rectangle::new(
            Point::new(left(column), filled[column as usize]),
            Size::new(deck, STRIP_HEIGHT),
        )
    };
    let (level, mut presets) = (under(Column::LeftDeck), under(Column::RightDeck));
    let bottom = |rect: Rectangle| rect.y + rect.height;
    let mixer_foot = mixer_last.map_or(0.0, |index: usize| bottom(blocks[index].rect));
    let foot = bottom(level).max(bottom(presets)).max(mixer_foot);
    if let Some(index) = mixer_last {
        blocks[index].rect.height = foot - blocks[index].rect.y;
    }
    presets.height = foot - presets.y;
    Booth {
        blocks,
        level,
        presets,
        size: Size::new(left(Column::RightDeck) + deck, foot),
    }
}

/// The parameter whose knob (or choice's button) a press at `at` on the booth finds.
pub fn hit(factory: &dyn ProcessorFactory, at: Point) -> Option<usize> {
    booth(factory).blocks.into_iter().find_map(|block| {
        block
            .rect
            .contains(at)
            .then(|| grid_knob_at(block.rect, block.columns, block.run.len(), at))
            .flatten()
            .map(|knob| block.run.start + knob)
    })
}

/// What a knob shows: its value (its automation's last, while one governs it), and whether it
/// is automated and linked to a controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knob {
    pub value: f32,
    pub automated: bool,
    pub linked: bool,
}

/// A preset the list offers: a factory one or one the user saved, by its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetChoice {
    pub name: String,
    pub factory: bool,
}

impl fmt::Display for PresetChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// Save preset… under way: the name box, or the question whether to replace a preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Naming {
    /// The name typed so far, and why the last Save was refused.
    Typing {
        name: String,
        problem: Option<String>,
    },
    /// A preset of this name (as it is saved) is there already.
    Replacing { name: String },
}

/// What the presets' controls report.
#[derive(Debug, Clone, PartialEq)]
pub enum PresetMessage {
    /// A preset picked from the list: its sound, one undo step.
    Load(PresetChoice),
    /// Save preset…: the name box opens.
    Save,
    /// The name box's text changed.
    Named(String),
    /// The name box's Save, or Enter in it.
    Submit,
    /// Replace: the preset of that name is written over.
    Replace,
    /// Cancel: the name box, or the question, closes.
    Cancel,
}

/// What the window shows of a synth channel beyond its strip.
pub struct SynthView<'a> {
    pub channel: ChannelId,
    pub factory: &'a dyn ProcessorFactory,
    /// One per parameter, in the factory's order.
    pub knobs: Vec<Knob>,
    pub sample_rate: u32,
    /// The presets the list offers: the factory's, then the user's.
    pub presets: Vec<PresetChoice>,
    /// The preset the synth sounds as now, shown picked; none once it is changed from all.
    pub current: Option<PresetChoice>,
    /// Whether Save preset… is offered: there is a folder to save in.
    pub saves: bool,
    pub naming: Option<&'a Naming>,
    /// The automation menu, while it is open over the knobs: pinned where the press was, in
    /// the knobs' coordinates.
    pub menu: Option<Element<'a, Message>>,
    pub cache: &'a canvas::Cache,
}

/// The booth at its size: the knobs' canvas, `level` (the strip's LEVEL, the window's to draw)
/// in its place under the left deck, the presets' controls in their module under the right
/// deck, and the automation menu over them while it is open there.
pub fn board<'a>(
    synth: &mut SynthView<'a>,
    booth: &Booth,
    level: Element<'a, Message>,
) -> Element<'a, Message> {
    let canvas = canvas::Canvas::new(Knobs {
        channel: synth.channel,
        factory: synth.factory,
        knobs: synth.knobs.clone(),
        sample_rate: synth.sample_rate,
        cache: synth.cache,
    })
    .width(booth.size.width)
    .height(booth.size.height);
    let place = booth.presets;
    let at = presets_origin(place);
    let presets = presets(synth, place.width - 2.0 * (at.x - place.x));
    let mut layers: Vec<Element<'a, Message>> = vec![
        canvas.into(),
        pin(level).x(booth.level.x).y(booth.level.y).into(),
        pin(presets).x(at.x).y(at.y).into(),
    ];
    layers.extend(synth.menu.take());
    stack(layers)
        .width(booth.size.width)
        .height(booth.size.height)
        .into()
}

/// Where the presets' controls start in their module at `place`: in line with a block's first
/// choice key across (Osc 2's wave key stands over them), [`BLOCK_INSET`] under the heading.
/// They end as far in from the module's right edge.
pub fn presets_origin(place: Rectangle) -> Point {
    let key = choice_rect(grid_cell(place, DECK_COLUMNS, 0));
    Point::new(key.x, place.y + BLOCK_HEAD + BLOCK_INSET)
}

/// The presets' controls, `width` across, a line under another: the list and Save preset…,
/// or the name box, then Save and Cancel, then why the last Save was refused, or the question
/// whether to replace a preset, then Replace and Cancel.
fn presets<'a>(synth: &SynthView<'a>, width: f32) -> Element<'a, Message> {
    let send = |message| Message::Instrument(InstrumentMessage::Preset(message));
    let tool = |words: &'a str, message: PresetMessage| {
        button(text(words).size(size::LABEL))
            .padding([2, 8])
            .style(style::tool_button(false))
            .on_press(send(message))
    };
    let label = |words: &'a str| {
        text(words)
            .size(size::LABEL)
            .color(style::palette().text_secondary)
    };
    let warning = |words: String| text(words).size(size::LABEL).color(style::WARNING);
    let lines = match synth.naming {
        None => {
            let list = pick_list(
                synth.presets.clone(),
                synth.current.clone(),
                move |choice| send(PresetMessage::Load(choice)),
            )
            .placeholder("Choose a preset")
            .text_size(size::LABEL)
            .padding([2, 4])
            .width(Fill)
            .style(style::chooser)
            .menu_style(style::chooser_menu);
            let list = row![label("Preset"), list]
                .spacing(PRESET_SPACING)
                .align_y(Center);
            let mut lines = column![hover_area(list, Hover::Sound(SoundPart::Presets))];
            if synth.saves {
                let save = tool("Save preset\u{2026}", PresetMessage::Save);
                lines = lines.push(hover_area(save, Hover::Sound(SoundPart::SavePreset)));
            }
            lines
        }
        Some(Naming::Typing { name, problem }) => {
            let input = text_input("Preset name", name)
                .id(Id::new(NAME_INPUT))
                .on_input(move |typed| send(PresetMessage::Named(typed)))
                .on_submit(send(PresetMessage::Submit))
                .size(size::LABEL)
                .padding([2, 6])
                .width(Fill);
            let name = row![
                label("Name"),
                hover_area(input, Hover::Sound(SoundPart::PresetName))
            ]
            .spacing(PRESET_SPACING)
            .align_y(Center);
            let answers = row![
                tool("Save", PresetMessage::Submit),
                tool("Cancel", PresetMessage::Cancel)
            ]
            .spacing(PRESET_SPACING);
            let mut lines = column![name, answers];
            if let Some(problem) = problem {
                lines = lines.push(warning(problem.clone()));
            }
            lines
        }
        Some(Naming::Replacing { name }) => column![
            warning(format!(
                "\u{201c}{name}\u{201d} is saved already. Replace it?"
            )),
            row![
                tool("Replace", PresetMessage::Replace),
                tool("Cancel", PresetMessage::Cancel)
            ]
            .spacing(PRESET_SPACING),
        ],
    };
    lines.spacing(PRESET_SPACING).width(width).into()
}

/// The knobs' canvas.
struct Knobs<'a> {
    channel: ChannelId,
    factory: &'a dyn ProcessorFactory,
    knobs: Vec<Knob>,
    sample_rate: u32,
    cache: &'a canvas::Cache,
}

/// The canvas's memory between events.
#[derive(Debug, Default)]
struct KnobsState {
    /// The last left press, to spot a double press.
    previous_press: Option<(Point, Instant)>,
    /// What the cursor was over last.
    hover: Option<Hover>,
}

impl Knobs<'_> {
    /// The booth's modules: each heading's block with its knobs, and the presets' module (its
    /// controls are widgets over it). The LEVEL's place is left to the strip, a module of its own.
    fn draw_blocks(&self, frame: &mut Frame) {
        let view = Rectangle::new(Point::ORIGIN, frame.size());
        let booth = booth(self.factory);
        for (
            (name, _),
            Block {
                rect: block,
                run,
                columns,
            },
        ) in self.factory.groups().iter().zip(booth.blocks)
        {
            draw_module(frame, block, name, view);
            for (knob, index) in run.enumerate() {
                let Some(shown) = self.knobs.get(index) else {
                    continue;
                };
                let cell = grid_cell(block, columns, knob);
                draw_param(
                    frame,
                    cell,
                    self.factory,
                    (index, shown.value),
                    self.sample_rate,
                    (shown.automated, shown.linked),
                    FINISH,
                );
            }
        }
        draw_module(frame, booth.presets, PRESETS_HEADING, view);
    }
}

/// A block's module at `block` in the [`FINISH`], its well and `heading` engraved over it, cut
/// with an ellipsis to its room.
fn draw_module(frame: &mut Frame, block: Rectangle, heading: &str, view: Rectangle) {
    draw_ground(frame, block, view, FINISH);
    let room = block.width - 2.0 * HEADING_INSET;
    if let Some(shown) = fit_width(heading, room, size::LABEL) {
        let at = Point::new(block.x + HEADING_INSET, block.y + BLOCK_HEAD / 2.0);
        FINISH.write_heading(at, style::palette().text, |at, color| {
            label(frame, &shown, at, color, size::LABEL);
        });
    }
}

impl canvas::Program<Message> for Knobs<'_> {
    type State = KnobsState;

    fn update(
        &self,
        state: &mut KnobsState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(
                button @ (mouse::Button::Left | mouse::Button::Right),
            )) => {
                let at = cursor.position_in(bounds)?;
                let index = hit(self.factory, at)?;
                let left = *button == mouse::Button::Left;
                let now = crate::ui::clock::now();
                let double = left
                    && state.previous_press.is_some_and(|(previous, time)| {
                        now.duration_since(time) <= DOUBLE_PRESS_TIME
                            && previous.distance(at) <= DOUBLE_PRESS_DISTANCE
                    });
                // A double press is not the first of another one.
                state.previous_press = (left && !double).then_some((at, now));
                let message = InstrumentMessage::SynthKnob {
                    index,
                    at,
                    right: !left,
                    double,
                };
                Some(canvas::Action::publish(Message::Instrument(message)).and_capture())
            }
            // The hint line names the knob under the cursor.
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let channel = self.channel;
                let now = cursor
                    .position_in(bounds)
                    .and_then(|at| hit(self.factory, at))
                    .map(|index| Hover::Sound(SoundPart::SynthParam { channel, index }));
                hover_change(&mut state.hover, now)
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &KnobsState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let blocks = self
            .cache
            .draw(renderer, bounds.size(), |frame| self.draw_blocks(frame));
        vec![blocks]
    }

    fn mouse_interaction(
        &self,
        _state: &KnobsState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let index = cursor
            .position_in(bounds)
            .and_then(|at| hit(self.factory, at));
        match index {
            Some(index) if is_choice(self.factory, index) => mouse::Interaction::Pointer,
            Some(_) => mouse::Interaction::ResizingHorizontally,
            None => mouse::Interaction::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use dawsome_engine::dsp::synth;
    use dawsome_engine::processor::Registry;

    use super::*;
    use crate::ui::instrument_window::{synth_window_size, WINDOW_MIN};
    use crate::ui::knob_block::{knob_rect, KNOB_TEXT_ROOM};

    fn synth(registry: &Registry) -> &dyn ProcessorFactory {
        registry.get(synth::ID).expect("registered")
    }

    // The booth, by hand from the block sizes (knob_block: 60 px a knob, 6 px margins, 22 px
    // head, 58 px a row, 4 px foot). A deck's blocks hold five knobs a row, 5 × 60 + 12 = 312
    // across: Osc 1's six and Osc 2's seven in two rows, 22 + 2 × 58 + 4 = 142 tall; Unison's
    // and the LFO's four in one, 84 tall. The mixer's hold four, 252 across: the Filter's five
    // in two rows (142), each envelope's and Play's four in one (84). The left deck at x 0, the
    // mixer 12 right of it at 312 + 12 = 324, the right deck at 324 + 252 + 12 = 588, 900 across
    // in all; down each column 6 apart: Unison and the LFO at 142 + 6 = 148, the envelopes at 148
    // and 148 + 84 + 6 = 238, Play at 238 + 90 = 328. Under each deck, from 148 + 84 + 6 = 238
    // (Amp env's line), one module 312 across: the LEVEL, as tall as the strip (176), its foot
    // 238 + 176 = 414, the booth's, the tallest column's (the mixer's ends at 328 + 84 = 412);
    // the presets' as tall, and Play stretched to it, 414 − 328 = 86. The presets' controls
    // start where a choice's key does in a block's first place, 588 + 6 + 4 = 598 (Osc 2's wave
    // key over them), 238 + 22 + 6 = 266 down. The window: 900 + 2 × 12 = 924 wide; 12 + title
    // 20 + 8 + 414 + 8 + footer 20 + 12 + foot line 22 = 516 tall. It holds the window's least
    // size (420 × 300).
    #[test]
    fn the_synths_headings_stand_as_a_booth() {
        let registry = Registry::native();
        let factory = synth(&registry);
        let booth = booth(factory);
        /// A heading, its block's corner and size, and how many knobs a row of it holds.
        type Placed<'a> = (&'a str, (f32, f32, f32, f32), usize);
        let placed: Vec<Placed> = factory
            .groups()
            .iter()
            .zip(&booth.blocks)
            .map(|((name, _), block)| {
                let rect = block.rect;
                (
                    *name,
                    (rect.x, rect.y, rect.width, rect.height),
                    block.columns,
                )
            })
            .collect();
        assert_eq!(
            placed,
            [
                ("Osc 1", (0.0, 0.0, 312.0, 142.0), 5),
                ("Osc 2", (588.0, 0.0, 312.0, 142.0), 5),
                ("Unison", (0.0, 148.0, 312.0, 84.0), 5),
                ("Filter", (324.0, 0.0, 252.0, 142.0), 4),
                ("Filter env", (324.0, 148.0, 252.0, 84.0), 4),
                ("Amp env", (324.0, 238.0, 252.0, 84.0), 4),
                ("LFO", (588.0, 148.0, 312.0, 84.0), 5),
                ("Play", (324.0, 328.0, 252.0, 86.0), 4),
            ]
        );
        let corner = |rect: Rectangle| (rect.x, rect.y, rect.width, rect.height);
        assert_eq!(corner(booth.level), (0.0, 238.0, 312.0, 176.0), "the LEVEL");
        assert_eq!(
            corner(booth.presets),
            (588.0, 238.0, 312.0, 176.0),
            "the presets"
        );
        assert_eq!(
            presets_origin(booth.presets),
            Point::new(598.0, 266.0),
            "the presets' controls in line with Osc 2's wave key, 6 under the heading"
        );
        assert_eq!((booth.size.width, booth.size.height), (900.0, 414.0));
        let feet =
            [booth.level, booth.blocks[7].rect, booth.presets].map(|rect| rect.y + rect.height);
        assert_eq!(feet, [414.0; 3], "every column reaches the booth's foot");
        let runs: Vec<Range<usize>> = booth.blocks.iter().map(|block| block.run.clone()).collect();
        let groups: Vec<Range<usize>> = factory
            .groups()
            .iter()
            .map(|(_, run)| run.clone())
            .collect();
        assert_eq!(runs, groups, "every parameter on the booth, once");
        let size = synth_window_size(factory);
        assert_eq!((size.width, size.height), (924.0, 516.0));
        assert!(size.width >= WINDOW_MIN.width && size.height >= WINDOW_MIN.height);
    }

    // A press finds the knob drawn there, by hand from the places above: the Filter at
    // (324, 0), its first place (330, 22), the knob centred at (360, 50): the cutoff, parameter
    // 17; its fifth knob starts its second row, the place (330, 80), the knob at (360, 108): the
    // Drive (21). Osc 2 at (588, 0), its seventh knob the second of its second row, from x 588 +
    // 6 + 60 = 654: at (684, 108) the oscillators' mix (12). The LFO at (588, 148), its fourth
    // from x 588 + 6 + 180 = 774, y 170: at (804, 198) the LFO to the cutoff (33). Play at
    // (324, 328), its fourth from x 510, y 350: at (540, 378) velocity → cutoff, the last (37).
    // Osc 1's wave button, (6 + 30, 22 + 29) → 0. A heading, the space between a deck and the
    // mixer, Unison's fifth place (it has four knobs), Play's 2 px stretched foot (413), the
    // LEVEL's place (the strip takes its own presses) and the presets' module find none.
    #[test]
    fn a_press_finds_the_parameter_whose_knob_is_drawn_there() {
        let registry = Registry::native();
        let factory = synth(&registry);
        let at = |x, y| hit(factory, Point::new(x, y));
        let key = |index: Option<usize>| index.map(|index| factory.params()[index].key);
        assert_eq!(at(360.0, 50.0), Some(17));
        assert_eq!(key(at(360.0, 50.0)), Some("cutoff"));
        assert_eq!(key(at(360.0, 108.0)), Some("drive"));
        assert_eq!(at(360.0, 108.0), Some(21));
        assert_eq!(key(at(684.0, 108.0)), Some("osc_mix"));
        assert_eq!(at(684.0, 108.0), Some(12));
        assert_eq!(key(at(804.0, 198.0)), Some("lfo_cutoff"));
        assert_eq!(at(804.0, 198.0), Some(33));
        assert_eq!(key(at(540.0, 378.0)), Some("vel_cutoff"));
        assert_eq!(at(540.0, 378.0), Some(37));
        assert_eq!(at(36.0, 51.0), Some(0));
        assert_eq!(at(450.0, 10.0), None, "the Filter heading");
        assert_eq!(at(318.0, 50.0), None, "between the left deck and the mixer");
        assert_eq!(at(276.0, 198.0), None, "Unison's place past its last knob");
        assert_eq!(at(540.0, 413.0), None, "Play's stretched foot");
        assert_eq!(at(150.0, 300.0), None, "the LEVEL's place");
        assert_eq!(at(744.0, 300.0), None, "the presets' module");
    }

    // Every knob's name and every heading fit their places whole (the knob's 58 px of text,
    // the heading its block's width less the insets; the presets' module's too), measured in
    // the window's font: none is cut with an ellipsis.
    #[test]
    fn every_knob_name_and_heading_fits_whole() {
        let registry = Registry::native();
        let factory = synth(&registry);
        for index in 0..factory.params().len() {
            let name = factory.knob_label(index);
            assert!(!name.is_empty(), "parameter {index} has a knob name");
            assert_eq!(
                fit_width(name, KNOB_TEXT_ROOM, size::DENSE).as_deref(),
                Some(name),
                "{name}"
            );
        }
        let booth = booth(factory);
        for ((name, _), block) in factory.groups().iter().zip(booth.blocks) {
            let room = block.rect.width - 2.0 * HEADING_INSET;
            assert_eq!(fit_width(name, room, size::LABEL).as_deref(), Some(*name));
        }
        let room = booth.presets.width - 2.0 * HEADING_INSET;
        assert_eq!(
            fit_width(PRESETS_HEADING, room, size::LABEL).as_deref(),
            Some(PRESETS_HEADING)
        );
    }

    /// The knobs' canvas, the booth's size, taking a press of `button` at `at`: the message it
    /// publishes.
    fn press(
        knobs: &Knobs<'_>,
        state: &mut KnobsState,
        button: mouse::Button,
        at: Point,
    ) -> Option<InstrumentMessage> {
        use canvas::Program;
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(900.0, 414.0));
        let event = Event::Mouse(mouse::Event::ButtonPressed(button));
        let action = knobs.update(state, &event, bounds, mouse::Cursor::Available(at))?;
        match action.into_inner().0? {
            Message::Instrument(message) => Some(message),
            other => panic!("not the window's: {other:?}"),
        }
    }

    // The canvas says which knob a press found and how: a left press on the cutoff's knob, the
    // same again at once (a double press, which the next does not continue), a right press, and
    // nothing for a press on a heading.
    #[test]
    fn the_canvas_reports_the_knob_a_press_found() {
        let cache = canvas::Cache::new();
        let registry = Registry::native();
        let factory = synth(&registry);
        let knobs = Knobs {
            channel: ChannelId(1),
            factory,
            knobs: Vec::new(),
            sample_rate: 44_100,
            cache: &cache,
        };
        let mut state = KnobsState::default();
        let booth = booth(factory);
        let filter = &booth.blocks[3];
        let cutoff = knob_rect(grid_cell(filter.rect, filter.columns, 0)).center();
        let left = mouse::Button::Left;
        let knob = |double, right| InstrumentMessage::SynthKnob {
            index: 17,
            at: cutoff,
            right,
            double,
        };
        assert_eq!(
            press(&knobs, &mut state, left, cutoff),
            Some(knob(false, false))
        );
        assert_eq!(
            press(&knobs, &mut state, left, cutoff),
            Some(knob(true, false))
        );
        assert_eq!(
            press(&knobs, &mut state, left, cutoff),
            Some(knob(false, false)),
            "a double press is not the first of another"
        );
        let right = mouse::Button::Right;
        assert_eq!(
            press(&knobs, &mut state, right, cutoff),
            Some(knob(false, true))
        );
        assert_eq!(
            press(&knobs, &mut state, left, Point::new(450.0, 10.0)),
            None
        );
    }
}
