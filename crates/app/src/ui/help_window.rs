//! The Help window's drawing: the guide's contents down the left, the section chosen on a page
//! that scrolls on the right (its title, its words and pictures, then the way to the sections
//! before and after it), and a line at the foot saying how to close the window.
//! `app/help_window.rs` runs it; `help.rs` reads the guide.

use std::collections::BTreeMap;

use iced::font::Weight;
use iced::widget::text::{Span, Wrapping};
use iced::widget::{
    button, column, container, image, rich_text, row, scrollable, space, text, Column, Id,
};
use iced::{Background, Border, Color, ContentFit, Element, Fill, Font, Size, Theme};

use crate::app::Message;
use crate::help::{Block, Guide, Text};
use crate::ui::layout::{HINT_BAR_HEIGHT, TOOLBAR_SIDE_PADDING};
use crate::ui::style;

/// The window's size at its first opening: the contents and a page as wide as the pictures.
pub const WINDOW_SIZE: Size = Size::new(960.0, 700.0);

/// The least the window is made: the contents and a page still wide enough to read.
pub const WINDOW_MIN: Size = Size::new(560.0, 360.0);

/// The page's scrollable, which a section chosen scrolls back to its top.
pub const PAGE: &str = "help-page";

/// The contents' width.
pub const CONTENTS_WIDTH: f32 = 236.0;

/// The page's text at its widest: lines longer than this are hard to follow.
const PAGE_MOST: f32 = 760.0;

const TITLE_SIZE: f32 = 22.0;
const HEADING_SIZE: f32 = 16.0;
const BODY_SIZE: f32 = 14.0;
const CAPTION_SIZE: f32 = 12.0;
const ENTRY_SIZE: f32 = 13.0;
const FOOT_SIZE: f32 = 12.0;
const PAGE_PADDING: f32 = 20.0;

/// Something the Help window reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpMessage {
    /// A section chosen: in the contents, or the way on at a page's foot.
    Show(usize),
}

/// What the Help window shows.
pub struct HelpPage<'a> {
    pub guide: &'a Guide,
    /// The section on the page, an index into the guide's.
    pub section: usize,
    /// The pictures read so far, by file: none for one missing or unreadable.
    pub pictures: &'a BTreeMap<String, Option<image::Handle>>,
    /// The foot line's words: how to close the window.
    pub foot: String,
}

impl<'a> HelpPage<'a> {
    pub fn view(self) -> Element<'a, Message> {
        let contents = self.contents();
        let page = self.page();
        let foot = container(
            text(self.foot)
                .size(FOOT_SIZE)
                .color(style::TEXT_DIM)
                .wrapping(Wrapping::None),
        )
        .padding([0.0, TOOLBAR_SIDE_PADDING])
        .width(Fill)
        .height(HINT_BAR_HEIGHT)
        .align_y(iced::Center)
        .style(style::panel);
        container(column![row![contents, page].height(Fill), foot])
            .width(Fill)
            .height(Fill)
            .style(style::rack_well)
            .into()
    }

    /// The contents: a row a section, numbered, the one shown lit.
    fn contents(&self) -> Element<'a, Message> {
        let mut entries = Column::new().spacing(2).padding([12.0, 8.0]);
        entries = entries.push(
            container(text("Contents").size(ENTRY_SIZE).color(style::TEXT_DIM)).padding([4.0, 8.0]),
        );
        for (index, section) in self.guide.sections.iter().enumerate() {
            let label = text(entry_label(index, &section.title))
                .size(ENTRY_SIZE)
                .width(Fill);
            entries = entries.push(
                button(label)
                    .width(Fill)
                    .padding([5.0, 8.0])
                    .style(entry_style(index == self.section))
                    .on_press(Message::Help(HelpMessage::Show(index))),
            );
        }
        container(scrollable(entries).height(Fill))
            .width(CONTENTS_WIDTH)
            .height(Fill)
            .style(style::panel)
            .into()
    }

    /// The section shown: its title, its blocks, the way to the sections either side.
    fn page(&self) -> Element<'a, Message> {
        let mut page = Column::new().spacing(10).max_width(PAGE_MOST);
        if let Some(section) = self.guide.sections.get(self.section) {
            page = page.push(
                text(section.title.clone())
                    .size(TITLE_SIZE)
                    .color(style::TEXT)
                    .font(bold()),
            );
            for block in &section.blocks {
                page = page.push(self.block(block));
            }
        }
        page = page.push(space().height(8)).push(self.way_on());
        scrollable(container(page).padding(PAGE_PADDING).width(Fill))
            .id(Id::new(PAGE))
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn block(&self, block: &Block) -> Element<'a, Message> {
        match block {
            Block::Heading(words) => container(words_of(words, HEADING_SIZE, true))
                .padding(iced::Padding::ZERO.top(6.0))
                .into(),
            Block::Paragraph(words) => words_of(words, BODY_SIZE, false),
            Block::Point(words) => row![
                text("\u{2022}").size(BODY_SIZE).color(style::ACCENT),
                words_of(words, BODY_SIZE, false),
            ]
            .spacing(8)
            .padding(iced::Padding::ZERO.left(6.0))
            .into(),
            Block::Picture { file, caption } => {
                let caption = text(caption.plain())
                    .size(CAPTION_SIZE)
                    .color(style::TEXT_DIM);
                // A picture missing or unreadable leaves its caption alone.
                match self.pictures.get(file).and_then(Option::as_ref) {
                    Some(handle) => column![
                        container(image(handle.clone()).content_fit(ContentFit::ScaleDown))
                            .padding(1)
                            .style(frame),
                        caption,
                    ]
                    .spacing(4)
                    .into(),
                    None => caption.into(),
                }
            }
        }
    }

    /// "← <the section before>" and "Next: <the one after> →", each where there is one.
    fn way_on(&self) -> Element<'a, Message> {
        let sections = &self.guide.sections;
        let back = self.section.checked_sub(1).and_then(|before| {
            let section = sections.get(before)?;
            Some(way_button(format!("\u{2190} {}", section.title), before))
        });
        let next = sections.get(self.section + 1).map(|section| {
            way_button(
                format!("Next: {} \u{2192}", section.title),
                self.section + 1,
            )
        });
        let mut way = row![].spacing(8);
        if let Some(back) = back {
            way = way.push(back);
        }
        way = way.push(space().width(Fill));
        if let Some(next) = next {
            way = way.push(next);
        }
        way.into()
    }
}

/// A contents entry's words: its number and the section's title.
pub fn entry_label(index: usize, title: &str) -> String {
    format!("{}. {title}", index + 1)
}

fn way_button<'a>(label: String, section: usize) -> Element<'a, Message> {
    button(text(label).size(ENTRY_SIZE))
        .padding([6.0, 10.0])
        .style(style::tool_button(false))
        .on_press(Message::Help(HelpMessage::Show(section)))
        .into()
}

/// `words` at `size`, their bold runs bold (all of them when `bold`).
fn words_of<'a>(words: &Text, size: f32, bold_all: bool) -> Element<'a, Message> {
    let spans: Vec<Span<'a, (), Font>> = words
        .0
        .iter()
        .map(|run| {
            let span = Span::new(run.words.clone());
            if run.bold || bold_all {
                span.font(bold())
            } else {
                span
            }
        })
        .collect();
    rich_text(spans)
        .size(size)
        .color(style::TEXT)
        .width(Fill)
        .into()
}

fn bold() -> Font {
    Font {
        weight: Weight::Bold,
        ..Font::DEFAULT
    }
}

/// A contents entry: flat on the contents' ground, lit in the accent while its section shows.
fn entry_style(shown: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let background = match (shown, status) {
            (true, _) => Some(Background::Color(style::BUTTON)),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                Some(Background::Color(style::BUTTON_HOVER))
            }
            (false, _) => None,
        };
        button::Style {
            background,
            text_color: if shown { style::ACCENT } else { style::TEXT },
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }
    }
}

/// The line around a picture, so a dark screenshot stands apart from the page's ground.
fn frame(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: style::TOOLBAR_LINE,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}
