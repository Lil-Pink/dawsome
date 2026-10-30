//! Colors and widget styles: a dark workspace with an orange accent.

use iced::overlay::menu;
use iced::widget::{button, container, pick_list, text_input};
use iced::{Background, Border, Color, Theme};

use crate::ui::layout::STEPS_PER_GROUP;

/// Window ground.
pub const WINDOW: Color = Color::from_rgb8(0x25, 0x28, 0x2D);
/// Toolbar and other raised panels.
pub const PANEL: Color = Color::from_rgb8(0x2B, 0x2E, 0x33);
/// The panels' title rows ("Library", "Channel rack", the playlist's and the piano roll's
/// strips, "Mixer", "Settings"): a shade over the toolbar's, so the two bands read apart.
/// They were the toolbar's own gray, which read as one band. Why lighter
/// and not darker: a darker row (0x212327 was tried) met the rack's well (0x1F2125) above the
/// playlist's strip, and those two blended instead.
pub const HEADER: Color = Color::from_rgb8(0x34, 0x37, 0x3D);
/// The line along the toolbar's foot and between its groups: lighter than the toolbar and the
/// title rows both (was the knob ring's 0x4A4F57 between the groups, and nothing at the foot).
pub const TOOLBAR_LINE: Color = Color::from_rgb8(0x5C, 0x62, 0x6B);
/// The recessed area the rack rows sit in.
pub const RACK_WELL: Color = Color::from_rgb8(0x1F, 0x21, 0x25);
pub const TEXT: Color = Color::from_rgb8(0xC9, 0xCD, 0xD3);
pub const TEXT_DIM: Color = Color::from_rgb8(0x6E, 0x74, 0x7C);
/// Selection, the playhead, and a running transport.
pub const ACCENT: Color = Color::from_rgb8(0xF2, 0x9A, 0x2E);
/// Notices about problems, such as audio being off.
pub const WARNING: Color = Color::from_rgb8(0xE8, 0xB0, 0x4A);

pub const BUTTON: Color = Color::from_rgb8(0x3A, 0x3E, 0x45);
pub const BUTTON_HOVER: Color = Color::from_rgb8(0x46, 0x4B, 0x53);
pub const BUTTON_EDGE: Color = Color::from_rgb8(0x1A, 0x1C, 0x1F);
pub const NAME_BUTTON: Color = Color::from_rgb8(0x40, 0x44, 0x4B);

/// Display boxes (tempo, position): dark glass with bright digits.
pub const LCD: Color = Color::from_rgb8(0x14, 0x16, 0x19);
pub const LCD_TEXT: Color = Color::from_rgb8(0xE4, 0xE7, 0xEB);

pub const KNOB_RING: Color = Color::from_rgb8(0x4A, 0x4F, 0x57);
pub const KNOB_FACE: Color = Color::from_rgb8(0x17, 0x19, 0x1C);
pub const LED_ON: Color = Color::from_rgb8(0x6C, 0xD8, 0x4A);
pub const LED_OFF: Color = Color::from_rgb8(0x2E, 0x32, 0x36);

/// Unlit step tones, alternating every group of four so beats read at a glance.
pub const STEP_UNLIT: [Color; 2] = [
    Color::from_rgb8(0x4A, 0x4F, 0x57),
    Color::from_rgb8(0x3A, 0x3E, 0x45),
];

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

/// Piano roll grid rows, by the kind of key they belong to.
pub const ROLL_ROW_WHITE: Color = Color::from_rgb8(0x34, 0x37, 0x3D);
pub const ROLL_ROW_BLACK: Color = Color::from_rgb8(0x2D, 0x30, 0x35);
/// Grid lines: the finest line unit, beats and bars.
pub const GRID_LINE: Color = Color::from_rgb8(0x3E, 0x42, 0x48);
pub const BEAT_LINE: Color = Color::from_rgb8(0x4A, 0x4F, 0x57);
pub const BAR_LINE: Color = Color::from_rgb8(0x62, 0x68, 0x73);
/// The piano roll's key column.
pub const KEY_WHITE: Color = Color::from_rgb8(0xCF, 0xD3, 0xD8);
pub const KEY_BLACK: Color = Color::from_rgb8(0x1A, 0x1C, 0x1F);
pub const KEY_EDGE: Color = Color::from_rgb8(0x8A, 0x8F, 0x96);
pub const KEY_LABEL: Color = Color::from_rgb8(0x25, 0x28, 0x2D);
/// A key chord two shortcuts share, on the Keys page.
pub const CONFLICT: Color = Color::from_rgb8(0xE5, 0x53, 0x3D);

/// Playlist lanes, the gap between them, and their grid lines.
pub const LANE: Color = Color::from_rgb8(0x34, 0x38, 0x3F);
pub const LANE_BAR_LINE: Color = Color::from_rgb8(0x48, 0x4D, 0x55);
pub const LANE_BEAT_LINE: Color = Color::from_rgb8(0x3B, 0x3F, 0x46);
/// The loop range in the ruler's strip, on and off, and the lanes inside it while it is on.
pub const LOOP_ON: Color = Color::from_rgb8(0xC4, 0x7A, 0x2C);
pub const LOOP_OFF: Color = Color::from_rgb8(0x5E, 0x51, 0x43);
pub const LOOP_TINT: Color = Color::from_rgb8(0x38, 0x35, 0x32);
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
/// The sample browser: its ground (the panels' own), the previewing row, and root names, which
/// are brighter than the rest of the tree.
pub const BROWSER_PREVIEWING: Color = Color::from_rgb8(0x3D, 0x41, 0x48);
pub const BROWSER_SELECTED: Color = Color::from_rgb8(0x34, 0x37, 0x3D);
pub const BROWSER_ROOT: Color = Color::from_rgb8(0xE8, 0xEC, 0xF1);
/// The Record button: off, and armed (it blinks between the two during the count-in).
pub const RECORD_OFF: Color = Color::from_rgb8(0x6E, 0x30, 0x30);
pub const RECORD_ARMED: Color = Color::from_rgb8(0xE5, 0x53, 0x3D);
/// A track's mute light: audible, muted.
pub const TRACK_LED_ON: Color = Color::from_rgb8(0x6C, 0xC2, 0x4A);
pub const TRACK_LED_OFF: Color = Color::from_rgb8(0x2A, 0x2D, 0x31);

/// Mixer strips: the body, the selected strip's body, the fader track and cap, the meter's
/// ground and its three bands (to −6 dB, to 0 dB, above), a route button that is off, one
/// flashing for a refused send, and a parameter slider's filled part.
pub const STRIP: Color = Color::from_rgb8(0x33, 0x37, 0x3D);
pub const STRIP_SELECTED: Color = Color::from_rgb8(0x3D, 0x41, 0x48);
pub const FADER_TRACK: Color = Color::from_rgb8(0x1A, 0x1C, 0x1F);
pub const FADER_CAP: Color = Color::from_rgb8(0xB8, 0xBE, 0xC6);
pub const METER_GROUND: Color = Color::from_rgb8(0x14, 0x16, 0x19);
pub const METER_GREEN: Color = Color::from_rgb8(0x6C, 0xC2, 0x4A);
pub const METER_YELLOW: Color = Color::from_rgb8(0xD9, 0xB2, 0x3A);
pub const METER_RED: Color = Color::from_rgb8(0xE5, 0x53, 0x3D);
pub const ROUTE_OFF: Color = Color::from_rgb8(0x4A, 0x4F, 0x57);
pub const REFUSED: Color = Color::from_rgb8(0xE5, 0x53, 0x3D);
pub const SLIDER_FILL: Color = Color::from_rgb8(0x8A, 0x5A, 0x24);

/// `color` with each channel scaled by `num/den` and `other` mixed in at `(den − num)/den`,
/// rounded half up on the 8-bit values. Integer arithmetic, so the result is exact.
fn mix(color: Color, other: Color, num: u16, den: u16) -> Color {
    let [r, g, b, _] = color.into_rgba8();
    let [r2, g2, b2, _] = other.into_rgba8();
    let blend = |x: u8, y: u8| {
        let mixed = (num * u16::from(x) + (den - num) * u16::from(y) + den / 2) / den;
        // A weighted mean of two 8-bit values stays within 0..=255.
        u8::try_from(mixed).unwrap_or(u8::MAX)
    };
    Color::from_rgb8(blend(r, r2), blend(g, g2), blend(b, b2))
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

pub fn unlit_step(step: u32) -> Color {
    STEP_UNLIT[(step / STEPS_PER_GROUP % 2) as usize]
}

pub fn window(_theme: &Theme) -> container::Style {
    filled(WINDOW)
}

pub fn panel(_theme: &Theme) -> container::Style {
    filled(PANEL)
}

pub fn rack_well(_theme: &Theme) -> container::Style {
    filled(RACK_WELL)
}

/// The piano roll's strip and the playlist's header: panel title rows.
pub fn strip(_theme: &Theme) -> container::Style {
    filled(HEADER)
}

/// The frame round a piece of the toolbar made of parts (the pattern's arrows and its name, Pat
/// over Song, Notes over Audio): the buttons' edge, its corners rounded as a button's, and the
/// same color showing between the parts, so they read as one control. Why: each part drew its
/// own edge, 2 px seams between three boxes of two grays (2026-10-27: the pattern's piece was
/// called "the ugliest part of the UI").
pub fn piece_frame(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BUTTON_EDGE,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..filled(BUTTON_EDGE)
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
            (true, _) => LCD,
            (false, button::Status::Hovered | button::Status::Pressed) => BUTTON_HOVER,
            (false, _) => BUTTON,
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: if lit { ACCENT } else { TEXT },
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
    filled(LCD)
}

/// A recessed display box.
pub fn lcd(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BUTTON_EDGE,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..filled(LCD)
    }
}

/// The pattern name between its arrows.
pub fn name_box(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BUTTON_EDGE,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..filled(NAME_BUTTON)
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
    filled(TOOLBAR_LINE)
}

/// A first-run note's bubble: the panels' ground in an accent border.
pub fn callout(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: ACCENT,
            width: 2.0,
            radius: 6.0.into(),
        },
        ..filled(PANEL)
    }
}

/// A dialog box, and the File menu's list.
pub fn dialog(_theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: BUTTON_EDGE,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..filled(PANEL)
    }
}

/// The instrument window's output chooser: a tool button's face, its list the File menu's
/// ground with the hovered row lit.
pub fn chooser(_theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let background = match status {
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => BUTTON_HOVER,
        pick_list::Status::Active => BUTTON,
    };
    pick_list::Style {
        text_color: TEXT,
        placeholder_color: TEXT_DIM,
        handle_color: TEXT_DIM,
        background: Background::Color(background),
        border: Border {
            color: BUTTON_EDGE,
            width: 1.0,
            radius: 2.0.into(),
        },
    }
}

/// The output chooser's list.
pub fn chooser_menu(_theme: &Theme) -> menu::Style {
    menu::Style {
        background: Background::Color(PANEL),
        border: Border {
            color: BUTTON_EDGE,
            width: 1.0,
            radius: 2.0.into(),
        },
        text_color: TEXT,
        selected_text_color: TEXT,
        selected_background: Background::Color(BUTTON_HOVER),
        shadow: iced::Shadow::default(),
    }
}

/// A File menu entry: flat until hovered; `dimmed` for a document that is gone.
pub fn menu_item(dimmed: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => Some(BUTTON_HOVER),
            button::Status::Active | button::Status::Disabled => None,
        };
        let text_color = match status {
            button::Status::Disabled => TEXT_DIM,
            _ if dimmed => TEXT_DIM,
            _ => TEXT,
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
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

/// A toolbar button; `lit` marks the active choice with accent text.
/// The browser's search box: no ground or border of its own, so the well it sits in shows.
pub fn search_box(_theme: &Theme, _status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: TEXT_DIM,
        placeholder: TEXT_DIM,
        value: TEXT,
        selection: Color { a: 0.4, ..ACCENT },
    }
}

pub fn tool_button(lit: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => BUTTON_HOVER,
            button::Status::Active | button::Status::Disabled => BUTTON,
        };
        let text_color = match (status, lit) {
            (button::Status::Disabled, _) => TEXT_DIM,
            (_, true) => ACCENT,
            (_, false) => TEXT,
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: Border {
                color: BUTTON_EDGE,
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }
    }
}

/// The WCAG contrast ratio of `a` on `b`: (L1 + 0.05) / (L2 + 0.05), the lighter's relative
/// luminance over the darker's, each from its linearized sRGB channels.
#[cfg(test)]
pub(crate) fn contrast(a: Color, b: Color) -> f32 {
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

    // Hand-computed, round half up: 0x34·0.6 = 31.2 → 0x1F, 0x37·0.6 = 33 → 0x21,
    // 0x3D·0.6 = 36.6 → 0x25; the kick color #4DA3E0 at 0.3 over #34373D is
    // (23.1 + 36.4, 48.9 + 38.5, 67.2 + 42.7) → #3C576E.
    #[test]
    fn dimmed_rows_and_ghosts_round_half_up() {
        assert_eq!(
            dimmed(ROLL_ROW_WHITE).into_rgba8(),
            [0x1F, 0x21, 0x25, 0xFF]
        );
        assert_eq!(
            dimmed(ROLL_ROW_BLACK).into_rgba8(),
            [0x1B, 0x1D, 0x20, 0xFF]
        );
        let kick = channel_color(1);
        assert_eq!(
            ghost(kick, ROLL_ROW_WHITE).into_rgba8(),
            [0x3C, 0x57, 0x6E, 0xFF]
        );
    }
}
