//! Colors and widget styles: a dark workspace with an orange accent. The chrome's colours
//! (surfaces, text, lines, keys, the controls' edges) come from the theme's
//! [`Palette`](crate::ui::palette::Palette), read through [`palette`], the accent (the user's
//! own or the theme's) through [`accent`] and [`slider_fill`]; the content colours
//! below (the channels', the clips', the LEDs', record's, the meters', the warnings') are the
//! same in every theme.

use iced::overlay::menu;
use iced::widget::{button, container, pick_list, text_input};
use iced::{Background, Border, Color, Theme};

use crate::ui::layout::STEPS_PER_GROUP;
use crate::ui::palette::mix;
pub use crate::ui::palette::{accent, palette, slider_fill};

/// Notices about problems, such as audio being off.
pub const WARNING: Color = Color::from_rgb8(0xE8, 0xB0, 0x4A);

pub const LED_ON: Color = Color::from_rgb8(0x6C, 0xD8, 0x4A);

/// Lit step color per channel, by rack order, repeating after the last.
pub const CHANNEL_COLORS: [Color; 8] = [
    Color::from_rgb8(0xE5, 0x53, 0x3D),
    Color::from_rgb8(0x4D, 0xA3, 0xE0),
    Color::from_rgb8(0x6C, 0xC2, 0x4A),
    Color::from_rgb8(0xD9, 0xB2, 0x3A),
    Color::from_rgb8(0xB0, 0x6A, 0xD4),
    Color::from_rgb8(0x3F, 0xC1, 0xB0),
    Color::from_rgb8(0xE0, 0x7A, 0xA8),
    Color::from_rgb8(0x9A, 0xA0, 0xA8),
];

pub fn channel_color(row: usize) -> Color {
    CHANNEL_COLORS[row % CHANNEL_COLORS.len()]
}

/// A key chord two shortcuts share, on the Keys page.
pub const CONFLICT: Color = Color::from_rgb8(0xE5, 0x53, 0x3D);

/// How much lighter a shaded bar is than what it lies on, per channel: every other bar of the
/// playlist's lanes and of the piano roll's rows (every other four bars far out), so bars read
/// as blocks however faint their lines are ([`shaded`]; the palette's `lane_alt` is its
/// `lane` shaded).
pub const SHADE_LIFT: u8 = 5;
/// The loop range in the ruler's strip, on and off.
pub const LOOP_ON: Color = Color::from_rgb8(0xC4, 0x7A, 0x2C);
pub const LOOP_OFF: Color = Color::from_rgb8(0x5E, 0x51, 0x43);
/// Pattern clips: the title strip, its text, the body, a muted body, and the note marks.
pub const CLIP_TITLE: Color = Color::from_rgb8(0x3F, 0x5D, 0x80);
pub const CLIP_TITLE_TEXT: Color = Color::from_rgb8(0xE8, 0xEC, 0xF1);
pub const CLIP_BODY: Color = Color::from_rgb8(0x5B, 0x7F, 0xA6);
pub const CLIP_MUTED: Color = Color::from_rgb8(0x4A, 0x50, 0x58);
pub const CLIP_NOTE: Color = Color::from_rgb8(0xDD, 0xE6, 0xF0);
/// Audio clips: the title strip, the body, the waveform and its center line, and the body of a
/// clip whose file is missing.
pub const AUDIO_TITLE: Color = Color::from_rgb8(0x3E, 0x6B, 0x57);
pub const AUDIO_BODY: Color = Color::from_rgb8(0x23, 0x33, 0x2C);
pub const AUDIO_WAVE: Color = Color::from_rgb8(0x9F, 0xD8, 0xBE);
pub const AUDIO_CENTER: Color = Color::from_rgb8(0x3E, 0x5E, 0x52);
pub const AUDIO_MISSING: Color = Color::from_rgb8(0x4A, 0x2E, 0x2E);
/// Automation clips: the title strip, the body, and the value line with its points.
pub const AUTOMATION_TITLE: Color = Color::from_rgb8(0x6B, 0x4A, 0x7A);
pub const AUTOMATION_BODY: Color = Color::from_rgb8(0x2E, 0x25, 0x33);
pub const AUTOMATION_LINE: Color = Color::from_rgb8(0xE0, 0xB8, 0xF0);
/// The sample browser's root names, which are brighter than the rest of the tree.
pub const BROWSER_ROOT: Color = Color::from_rgb8(0xE8, 0xEC, 0xF1);
/// The Record button: off, and armed (it blinks between the two during the count-in).
pub const RECORD_OFF: Color = Color::from_rgb8(0x6E, 0x30, 0x30);
pub const RECORD_ARMED: Color = Color::from_rgb8(0xE5, 0x53, 0x3D);
/// A track's mute light while audible.
pub const TRACK_LED_ON: Color = Color::from_rgb8(0x6C, 0xC2, 0x4A);

/// The meter's three bands (to −6 dB, to 0 dB, above), and a route button flashing for a
/// refused send.
pub const METER_GREEN: Color = Color::from_rgb8(0x6C, 0xC2, 0x4A);
pub const METER_YELLOW: Color = Color::from_rgb8(0xD9, 0xB2, 0x3A);
pub const METER_RED: Color = Color::from_rgb8(0xE5, 0x53, 0x3D);
pub const REFUSED: Color = Color::from_rgb8(0xE5, 0x53, 0x3D);

/// `color` in a shaded bar: each channel [`SHADE_LIFT`] lighter (of 255), held at white.
pub fn shaded(color: Color) -> Color {
    let [r, g, b, _] = color.into_rgba8();
    let lift = |x: u8| x.saturating_add(SHADE_LIFT);
    Color::from_rgb8(lift(r), lift(g), lift(b))
}

/// A grid row past the pattern's end: each channel times 0.6.
pub fn dimmed(row: Color) -> Color {
    mix(row, Color::BLACK, 6, 10)
}

/// A piano roll row or key outside the chosen scale: 0.7 of its color over black, darker
/// than the rows in it and lighter than a row past the pattern's end.
pub fn out_of_scale(row: Color) -> Color {
    mix(row, Color::BLACK, 7, 10)
}

/// Another channel's note shown behind the edited one: 0.3 of its color over 0.7 of the row.
pub fn ghost(channel: Color, row: Color) -> Color {
    mix(channel, row, 3, 10)
}

/// A note the Quantize dialog moves, drawn where it was: 0.15 of the channel's color over 0.85
/// of the row, half a [`ghost`], so the old place shows through faintly behind where it goes.
pub fn faint_ghost(channel: Color, row: Color) -> Color {
    mix(channel, row, 3, 20)
}

/// The rim round a [`faint_ghost`]: 0.3 of the channel's color over 0.7 of the row, so it reads
/// as the note's own old place rather than another channel's note. Why no firmer: at 0.6, with
/// a 0.3 fill, the old places read as solid as the notes.
pub fn ghost_rim(channel: Color, row: Color) -> Color {
    mix(channel, row, 3, 10)
}

pub fn unlit_step(step: u32) -> Color {
    palette().step_unlit[(step / STEPS_PER_GROUP % 2) as usize]
}

pub fn window(_theme: &Theme) -> container::Style {
    filled(palette().window)
}

pub fn panel(_theme: &Theme) -> container::Style {
    filled(palette().panel)
}

pub fn rack_well(_theme: &Theme) -> container::Style {
    filled(palette().bg)
}

/// The piano roll's strip and the playlist's header: panel title rows.
pub fn strip(_theme: &Theme) -> container::Style {
    filled(palette().raised)
}

/// The frame round a piece of the toolbar made of parts (the pattern's arrows and its name, Pat
/// over Song, Notes over Audio): the buttons' edge, its corners rounded as a button's, and the
/// same color showing between the parts, so they read as one control. Why: each part drew its
/// own edge, 2 px seams between three boxes of two grays, and the pattern's piece looked the
/// worst of the bar.
pub fn piece_frame(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: palette().button_edge,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..filled(palette().button_edge)
    }
}

/// A button inside a piece's frame (the pattern's arrows): [`tool_button`]'s face with no edge of
/// its own, `radius` rounding the piece's outer corners inside the frame's.
pub fn piece_part(
    radius: iced::border::Radius,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    let face = tool_button(false);
    move |theme, status| {
        let mut style = face(theme, status);
        style.border = Border {
            radius,
            ..Border::default()
        };
        style
    }
}

/// A side of a two-row switch inside its frame (Pat over Song, Notes over Audio): the side lit,
/// the one in use, on the readouts' dark glass in the accent, so the switch reads as set to it;
/// the other a button's face, lighter under the pointer. `radius` rounds the switch's outer
/// corners inside the frame's.
pub fn switch_side(
    lit: bool,
    radius: iced::border::Radius,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let background = match (lit, status) {
            (true, _) => palette().lcd,
            (false, button::Status::Hovered | button::Status::Pressed) => palette().button_hover,
            (false, _) => palette().button,
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: if lit { accent() } else { palette().text },
            border: Border {
                radius,
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

/// The pattern's name between its arrows, inside the piece's frame: the readouts' dark glass
/// (the tempo's and the position's either side of it), no edge of its own.
pub fn name_display(_theme: &Theme) -> container::Style {
    filled(palette().lcd)
}

/// A recessed display box.
pub fn lcd(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: palette().button_edge,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..filled(palette().lcd)
    }
}

/// The pattern name between its arrows.
pub fn name_box(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: palette().button_edge,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..filled(palette().name_button)
    }
}

/// The dimmed window behind a dialog.
pub fn scrim(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba8(0x0B, 0x0C, 0x0E, 0.6))),
        ..container::Style::default()
    }
}

/// The thin line between two of the toolbar's groups, and along its foot.
pub fn divider(_theme: &Theme) -> container::Style {
    filled(palette().toolbar_line)
}

/// A first-run note's bubble: the panels' ground in an accent border.
pub fn callout(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: accent(),
            width: 2.0,
            radius: 6.0.into(),
        },
        ..filled(palette().panel)
    }
}

/// The outline of the window a panel pops out into, following the cursor while a divider is
/// dragged sideways: the dialogs' dimming inside an accent frame.
pub fn pop_out_outline(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba8(0x0B, 0x0C, 0x0E, 0.6))),
        border: Border {
            color: accent(),
            width: 2.0,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    }
}

/// A dialog box, and the File menu's list.
pub fn dialog(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: palette().button_edge,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..filled(palette().panel)
    }
}

/// The instrument window's output chooser: a tool button's face, its list the File menu's
/// ground with the hovered row lit.
pub fn chooser(_theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let background = match status {
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => palette().button_hover,
        pick_list::Status::Active => palette().button,
    };
    pick_list::Style {
        text_color: palette().text,
        placeholder_color: palette().text_secondary,
        handle_color: palette().text_secondary,
        background: Background::Color(background),
        border: Border {
            color: palette().button_edge,
            width: 1.0,
            radius: 2.0.into(),
        },
    }
}

/// The output chooser's list.
pub fn chooser_menu(_theme: &Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(palette().panel),
        border: Border {
            color: palette().button_edge,
            width: 1.0,
            radius: 2.0.into(),
        },
        text_color: palette().text,
        selected_text_color: palette().text,
        selected_background: Background::Color(palette().hover),
        shadow: iced::Shadow::default(),
    }
}

/// A File menu entry: flat until hovered; `dimmed` for a document that is gone.
pub fn menu_item(dimmed: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => Some(palette().hover),
            button::Status::Active | button::Status::Disabled => None,
        };
        let text_color = match status {
            button::Status::Disabled => palette().text_disabled,
            _ if dimmed => palette().text_disabled,
            _ => palette().text,
        };
        button::Style {
            background: background.map(Background::Color),
            text_color,
            ..button::Style::default()
        }
    }
}

fn filled(color: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(color)),
        text_color: Some(palette().text),
        ..container::Style::default()
    }
}

/// The browser's search box: no ground or border of its own, so the well it sits in shows.
pub fn search_box(_theme: &Theme, _status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: palette().text_secondary,
        placeholder: palette().text_secondary,
        value: palette().text,
        selection: Color { a: 0.4, ..accent() },
    }
}

/// The accent's hex box in Settings: the readouts' dark glass in the buttons' edge, the edge in
/// the accent while the box has the keys; its selection the accent at 0.4, as the search box's.
pub fn hex_box(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let edge = match status {
        text_input::Status::Focused { .. } => accent(),
        _ => palette().button_edge,
    };
    text_input::Style {
        background: Background::Color(palette().lcd),
        border: Border {
            color: edge,
            width: 1.0,
            radius: 2.0.into(),
        },
        icon: palette().text_secondary,
        placeholder: palette().text_secondary,
        value: palette().text,
        selection: Color { a: 0.4, ..accent() },
    }
}

/// A toolbar button; `lit` marks the active choice with accent text.
pub fn tool_button(lit: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => palette().button_hover,
            button::Status::Active | button::Status::Disabled => palette().button,
        };
        let text_color = match (status, lit) {
            (button::Status::Disabled, _) => palette().text_disabled,
            (_, true) => accent(),
            (_, false) => palette().text,
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: Border {
                color: palette().button_edge,
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }
    }
}

/// The WCAG contrast ratio of `a` on `b`: (L1 + 0.05) / (L2 + 0.05), the lighter's relative
/// luminance over the darker's, each from its linearized sRGB channels.
pub fn contrast(a: Color, b: Color) -> f32 {
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = |c: Color| 0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b);
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::palette::GRAPHITE;

    // Hand-computed, round half up: 0x34·0.6 = 31.2 → 0x1F, 0x37·0.6 = 33 → 0x21,
    // 0x3D·0.6 = 36.6 → 0x25; the kick color #4DA3E0 at 0.3 over #34373D is
    // (23.1 + 36.4, 48.9 + 38.5, 67.2 + 42.7) → #3C576E.
    #[test]
    fn dimmed_rows_and_ghosts_round_half_up() {
        assert_eq!(
            dimmed(GRAPHITE.roll_row_white).into_rgba8(),
            [0x1F, 0x21, 0x25, 0xFF]
        );
        assert_eq!(
            dimmed(GRAPHITE.roll_row_black).into_rgba8(),
            [0x1B, 0x1D, 0x20, 0xFF]
        );
        let kick = channel_color(1);
        assert_eq!(
            ghost(kick, GRAPHITE.roll_row_white).into_rgba8(),
            [0x3C, 0x57, 0x6E, 0xFF]
        );
    }

    // A shaded bar, by hand, 5 lighter a channel: Graphite's lane #34383F is its lane_alt
    // #393D44; the roll's white-key row #34373D is #393C42 and its black-key row #2D3035 is
    // #32353A; past the end (#1F2125 for the white row) #24262A; white stays white.
    #[test]
    fn a_shaded_bar_is_five_lighter_on_the_lanes_and_the_rows() {
        assert_eq!(shaded(GRAPHITE.lane), GRAPHITE.lane_alt);
        assert_eq!(
            shaded(GRAPHITE.roll_row_white).into_rgba8(),
            [0x39, 0x3C, 0x42, 0xFF]
        );
        assert_eq!(
            shaded(GRAPHITE.roll_row_black).into_rgba8(),
            [0x32, 0x35, 0x3A, 0xFF]
        );
        assert_eq!(
            shaded(dimmed(GRAPHITE.roll_row_white)).into_rgba8(),
            [0x24, 0x26, 0x2A, 0xFF]
        );
        assert_eq!(shaded(Color::WHITE), Color::WHITE);
    }

    // The Quantize preview's old places, fill and rim at half a ghost's weight and a ghost's,
    // on a white-key and a black-key row (the kick, rounded half up, from Python).
    #[test]
    fn quantize_ghosts_are_faint_on_both_rows() {
        let kick = channel_color(1);
        let drawn = |row| {
            (
                faint_ghost(kick, row).into_rgba8(),
                ghost_rim(kick, row).into_rgba8(),
            )
        };
        assert_eq!(
            drawn(GRAPHITE.roll_row_white),
            ([0x38, 0x47, 0x55, 0xFF], [0x3C, 0x57, 0x6E, 0xFF])
        );
        assert_eq!(
            drawn(GRAPHITE.roll_row_black),
            ([0x32, 0x41, 0x4F, 0xFF], [0x37, 0x53, 0x68, 0xFF])
        );
    }
}
