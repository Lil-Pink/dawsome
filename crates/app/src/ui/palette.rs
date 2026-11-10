//! The chrome's colours as one set, swapped whole: the surfaces, the text, the lines, the keys,
//! the accent and the controls' edges ([`Palette`]); the five fixed sets ([`GRAPHITE`], the
//! look the app always had, [`SLATE`], [`GILT`], [`GARNET`] and [`FJORD`]); and the set drawn
//! now ([`palette`], [`set_theme`]), chosen on Settings' Theme page and kept as `ui_theme`.
//!
//! One colour of a set may be the user's own: the accent ([`set_accent`], the Theme page's
//! Accent, kept as `ui_accent`), drawn in every theme over the theme's. Every window reads the
//! accent through [`accent`] and the slider's fill, which follows it, through [`slider_fill`];
//! a set's own two are private to this module, so no draw site can skip the user's (the Theme
//! page's keys show each set's own accent through [`ThemeId::swatch`], by design). Why one
//! colour and not the set: the accent marks what is lit and chosen, the one colour a user has a
//! taste in; the surfaces and the text are tuned to read together, a set's job.
//!
//! Content colours are no part of a set: the channels', the clips', the LEDs', record's, the
//! meters' and the warnings' stay [`crate::ui::style`]'s constants in every theme and whatever
//! the accent. Why: they carry meaning the user has learned (a red dot is armed, a blue block a
//! pattern), and a theme that recoloured them would confuse. One exception, a shade and no
//! hue: the audio clip's body ([`Palette::audio_clip_body`]), which Gilt's lanes all but
//! swallowed while they were green, and which it keeps a shade lighter on its brown ones. Nothing else in a set is editable: the sets are constants, and a fourth is one
//! more of them.

use iced::Color;

/// A colour from its 24-bit hex value, as the sets are written.
const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `color` with each channel scaled by `num/den` and `other` mixed in at `(den − num)/den`,
/// rounded half up on the 8-bit values. Integer arithmetic, so the result is exact.
pub fn mix(color: Color, other: Color, num: u16, den: u16) -> Color {
    let [r, g, b, _] = color.into_rgba8();
    let [r2, g2, b2, _] = other.into_rgba8();
    let blend = |x: u8, y: u8| {
        let mixed = (num * u16::from(x) + (den - num) * u16::from(y) + den / 2) / den;
        // A weighted mean of two 8-bit values stays within 0..=255.
        u8::try_from(mixed).unwrap_or(u8::MAX)
    };
    Color::from_rgb8(blend(r, r2), blend(g, g2), blend(b, b2))
}

/// A colour of the user's own as 8-bit red, green and blue: what settings.json keeps as
/// `"#RRGGBB"`, what the hex box and `--accent` take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    /// `#RRGGBB` in any case, the `#` optional and spaces around it ignored (as typed); none
    /// for anything else, a three-digit `#RGB` among them.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let digits = text.strip_prefix('#').unwrap_or(text);
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        Some(Self([byte(0)?, byte(2)?, byte(4)?]))
    }

    /// `"#RRGGBB"`, capitals, as settings.json keeps it and the hex box shows it.
    pub fn hex(self) -> String {
        let [r, g, b] = self.0;
        format!("#{r:02X}{g:02X}{b:02X}")
    }

    /// The colour `color` is on an 8-bit screen.
    pub fn of(color: Color) -> Self {
        let [r, g, b, _] = color.into_rgba8();
        Self([r, g, b])
    }

    pub fn color(self) -> Color {
        let [r, g, b] = self.0;
        Color::from_rgb8(r, g, b)
    }
}

/// The least contrast (WCAG) an accent has on its theme's panels before Settings notes that it
/// reads poorly there: 3:1, WCAG's floor for the parts of a control. A note, never a refusal:
/// any colour may be the accent.
pub const ACCENT_LEAST_CONTRAST: f32 = 3.0;

/// Every chrome colour the app draws with, one set of them per theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// The recessed ground the rack's rows sit in, the darkest surface.
    pub bg: Color,
    /// The toolbar's, the Library's and the dialogs' ground.
    pub panel: Color,
    /// The panels' title rows, a shade over the toolbar's so the two bands read apart.
    pub raised: Color,
    /// A list row or menu entry under the pointer.
    pub hover: Color,
    /// A lit band: the flat finish's group headers.
    pub active: Color,
    /// The window's ground.
    pub window: Color,
    pub text: Color,
    /// Text that labels live information, a step under [`Palette::text`]: a row's maker, a
    /// group's label ("Bars", "Snap"), a hint, a caption, the icons of a control that acts. At
    /// least 4.5:1 on bg, panel and raised, WCAG's floor for text.
    pub text_secondary: Color,
    /// Text and marks of what is unavailable now: a control that does nothing, a device
    /// unplugged, a file gone. Dimmer than [`Palette::text_secondary`], with no floor: WCAG
    /// exempts what cannot be used.
    pub text_disabled: Color,
    /// The readouts' digits (tempo, position), and their dark glass.
    pub lcd_text: Color,
    pub lcd: Color,
    /// The lit edges: a raised part's top and left, the seam's lip, the roll lane's edges.
    pub line_strong: Color,
    /// The piano roll's finest grid line.
    pub line_soft: Color,
    /// The piano roll's beat lines.
    pub grid_beat: Color,
    /// The bar lines, the strong one starting every four bars among them.
    pub bar_line: Color,
    /// The playlist's lanes: their bar and beat lines, a lane, and a shaded bar of it.
    pub lane_bar_line: Color,
    pub lane_beat_line: Color,
    pub lane: Color,
    pub lane_alt: Color,
    /// The piano roll's grid rows, by the kind of key they belong to.
    pub roll_row_white: Color,
    pub roll_row_black: Color,
    pub button: Color,
    pub button_hover: Color,
    pub button_edge: Color,
    /// The pattern's name box; a console module's face at its top.
    pub name_button: Color,
    pub knob_ring: Color,
    pub knob_face: Color,
    pub fader_track: Color,
    pub fader_cap: Color,
    /// A mixer strip's body, and the selected strip's.
    pub strip: Color,
    pub strip_selected: Color,
    /// An unlit LED (an effect that is off, the master's clip light, a switch of the instrument
    /// window; a mute light off is a ring in [`Palette::line_strong`]), and a route button that
    /// is off.
    pub led_off: Color,
    pub route_off: Color,
    /// The two tones a list's rows alternate between (the plugin picker's), so a row reads as
    /// one across its columns.
    pub row_tones: [Color; 2],
    /// A parameter slider's filled part; drawn through [`slider_fill`].
    slider_fill: Color,
    /// Selection, the playhead, a running transport, a lit choice; drawn through [`accent`].
    accent: Color,
    /// The line along the toolbar's foot and between its groups.
    pub toolbar_line: Color,
    /// The groove down the Library's right edge.
    pub seam: Color,
    /// The piano roll's key column, and the keys of the instrument window's map and of the
    /// Keys page. The white keys are a shade down from each set's design, so they stop
    /// outshining the notes beside them: Graphite's CFD3D8 became B4B9BF, its CIE L* 84.39
    /// to 74.93, 0.888 of it; every other set's white takes the same share of its L*, its a* and
    /// b* kept (`tests/fixtures/gen_ui_contrast.py` works each out). Slate's white was
    /// Graphite's, so it is B4B9BF again.
    pub key_white: Color,
    pub key_black: Color,
    pub key_edge: Color,
    pub key_label: Color,
    /// The Library's previewing row and its selected row.
    pub browser_previewing: Color,
    pub browser_selected: Color,
    /// Unlit step tones, alternating every group of four so beats read at a glance: dark
    /// enough that every channel colour stands 3:1 off both, with a
    /// [`Palette::line_soft`] line between two groups so the beats still read apart.
    pub step_unlit: [Color; 2],
    /// The lanes inside the loop range while it is on.
    pub loop_tint: Color,
    /// A meter's ground.
    pub meter_ground: Color,
    /// An audio clip's body on the playlist's lanes: [`crate::ui::style::AUDIO_BODY`], which every
    /// set drew, in every set whose lanes leave it a hue apart; a shade lighter in Gilt, whose
    /// lanes were its own green and are brown now, so it reads at least 1.5:1 on the lanes (the
    /// raised band).
    pub audio_clip_body: Color,
}

/// Today's look, the default: the constants the app drew with before themes, bit for bit but
/// the white keys, a shade down since (B4B9BF, was CFD3D8), and two changes since: the secondary
/// text 9FA3A9 (the dim text's sites that label live information; the rest, the disabled text,
/// keep its 6E747C), the unlit steps a shade down (34383E and 2A2E34, were 4A4F57 and 3A3E45).
/// The secondary text is the text's CIE L* times Slate's share (`gen_ui_contrast.py`); the
/// picker's rows are hand-set (the panel-to-hover law gives 303439 and 393D43).
pub const GRAPHITE: Palette = Palette {
    bg: hex(0x1F2125),
    panel: hex(0x2B2E33),
    raised: hex(0x34373D),
    hover: hex(0x464B53),
    active: hex(0x3D4148),
    window: hex(0x25282D),
    text: hex(0xC9CDD3),
    text_secondary: hex(0x9FA3A9),
    text_disabled: hex(0x6E747C),
    lcd_text: hex(0xE4E7EB),
    lcd: hex(0x141619),
    line_strong: hex(0x5C626B),
    line_soft: hex(0x3E4248),
    grid_beat: hex(0x4A4F57),
    bar_line: hex(0x626873),
    lane_bar_line: hex(0x484D55),
    lane_beat_line: hex(0x3B3F46),
    lane: hex(0x34383F),
    lane_alt: hex(0x393D44),
    roll_row_white: hex(0x34373D),
    roll_row_black: hex(0x2D3035),
    button: hex(0x3A3E45),
    button_hover: hex(0x464B53),
    button_edge: hex(0x1A1C1F),
    name_button: hex(0x40444B),
    knob_ring: hex(0x4A4F57),
    knob_face: hex(0x17191C),
    fader_track: hex(0x1A1C1F),
    fader_cap: hex(0xB8BEC6),
    strip: hex(0x33373D),
    strip_selected: hex(0x3D4148),
    led_off: hex(0x2E3236),
    route_off: hex(0x4A4F57),
    row_tones: [hex(0x30343A), hex(0x383C43)],
    slider_fill: hex(0x8A5A24),
    accent: hex(0xF29A2E),
    toolbar_line: hex(0x5C626B),
    seam: hex(0x1A1C1F),
    key_white: hex(0xB4B9BF),
    key_black: hex(0x1A1C1F),
    key_edge: hex(0x8A8F96),
    key_label: hex(0x25282D),
    browser_previewing: hex(0x3D4148),
    browser_selected: hex(0x34373D),
    step_unlit: [hex(0x34383E), hex(0x2A2E34)],
    loop_tint: hex(0x383532),
    meter_ground: hex(0x141619),
    audio_clip_body: crate::ui::style::AUDIO_BODY,
};

/// A darker, cooler five-step blue-grey with brighter text, the same orange.
/// Beside the design's table (bg to the disabled text, accent, the two lines, the white keys,
/// the unlit steps) and its
/// sketch (the lanes, the buttons, the readouts, the knob ring, the unlit steps), the rest
/// are Graphite's relations: the window the panel's ground 2 darker a channel, the readouts'
/// glass and the meters' ground bg 3 darker, a shaded lane [`crate::ui::style::shaded`], the
/// beat lines 2/5 of `line_strong` over `line_soft`, the slider's fill the accent at 55 % over
/// bg, the black-key rows 2/9 of the way from the panel's ground to the white-key rows, an
/// unlit LED a third, a route that is off the first unlit step, and the picker's rows 1/5 and
/// 1/2 of the way from the panel's ground to the hover band.
pub const SLATE: Palette = Palette {
    bg: hex(0x15171B),
    panel: hex(0x1C1F24),
    raised: hex(0x262A30),
    hover: hex(0x30353C),
    active: hex(0x3A4048),
    window: hex(0x1A1D22),
    text: hex(0xDCE0E5),
    text_secondary: hex(0xACB2BA),
    text_disabled: hex(0x6B7179),
    lcd_text: hex(0xDCE0E5),
    lcd: hex(0x121418),
    line_strong: hex(0x7A818B),
    line_soft: hex(0x2F343B),
    grid_beat: hex(0x4D535B),
    bar_line: hex(0x7A818B),
    lane_bar_line: hex(0x545A63),
    lane_beat_line: hex(0x2F343B),
    lane: hex(0x262A30),
    lane_alt: hex(0x2B2F35),
    roll_row_white: hex(0x262A30),
    roll_row_black: hex(0x1E2127),
    button: hex(0x262A30),
    button_hover: hex(0x30353C),
    button_edge: hex(0x15171B),
    name_button: hex(0x2C3036),
    knob_ring: hex(0x7A818B),
    knob_face: hex(0x15171B),
    fader_track: hex(0x15171B),
    fader_cap: hex(0xB8BEC6),
    strip: hex(0x262A30),
    strip_selected: hex(0x3A4048),
    led_off: hex(0x1F2328),
    route_off: hex(0x34383E),
    row_tones: [hex(0x202329), hex(0x262A30)],
    slider_fill: hex(0x8F5F25),
    accent: hex(0xF29A2E),
    toolbar_line: hex(0x7A818B),
    seam: hex(0x15171B),
    key_white: hex(0xB4B9BF),
    key_black: hex(0x15171B),
    key_edge: hex(0x8A8F96),
    key_label: hex(0x1A1D22),
    browser_previewing: hex(0x3A4048),
    browser_selected: hex(0x262A30),
    step_unlit: [hex(0x34383E), hex(0x2A2E34)],
    loop_tint: hex(0x2A2723),
    meter_ground: hex(0x121418),
    audio_clip_body: crate::ui::style::AUDIO_BODY,
};

/// Gold leaf on deep chocolate, measured from a photo of glossy deep-brown lacquer flaked with
/// gold leaf beside a cream cable knit (region medians and 10th/90th percentiles; until 0.1.4 it
/// was gold on deep green): the gold is the accent, the lit edges (`line_strong`, the knob ring,
/// the console's lips, the seam's lip) and the fader caps, never a large fill, so it stays clear
/// of the orange record and warning colours and the amber meter; the synth's console reads as
/// chocolate lacquer with gold lips. The design, from the photo's measures in CIE L*a*b* (as
/// `tests/fixtures/gen_ui_contrast.py` works it out):
/// - the tonal ladder: bg 251812, dark chocolate, has the lightness of the lacquer's shadow (the
///   L*a*b* midpoint of its two measured ends 2C100F and 37120C: L* 9.78, a* 16.17, b* 9.28) and
///   a hue turned from its red toward chocolate: a* a third of the shadow's (16.17 / 3 = 5.39), b*
///   the mean of three dark-chocolate references (231512, 2E1D18, 3A2721: b* 4.62, 6.80 and 7.52,
///   mean 6.31), so the hue goes from 29.9° to 49.5° and the chroma from 18.64 to 8.30 (bg in 8
///   bits: L* 9.72, a* 5.52, b* 6.47); the shadow's own a* read as red-brown on every surface. The
///   window 2B1E18, the panel 2E201B, the raised band 382A24, the hover band 473832, the active
///   band 54453F, the readouts' glass 21140D (off Slate's bg-3-darker law, which Garnet and Fjord
///   follow) and `line_soft` 3F312B each keep the L* step from bg it had in the green set (+2.67,
///   +4.02, +8.82, +15.13, +20.92, −1.89 and +11.96), with bg's a* and b*;
/// - the text D7C1B3, the knit's light, and the secondary text B89E8D, its mid, as measured: they
///   read 7.98:1 and 5.45:1 on the raised band, past the 7 and 4.5 the rules ask, so neither is
///   lightened;
/// - the accent F4BC7F, the leaf's bright, as measured (8.10:1 on the raised band, past 4.5); the
///   fader cap E0A265, the leaf's median; `line_strong` 9C7854, the leaf's dim (3.91:1 on the
///   panel, past 2.5);
/// - the white key D7C1B3 as designed, the knit's light (a shade down here as every set's is:
///   BEA99B), and the black key bg.
///
/// The rest follow laws, which [`GARNET`] and [`FJORD`] follow too (8-bit, rounded half up,
/// [`mix`]):
/// - as Slate's: the beat lines 2/5 of `line_strong` over `line_soft`; the slider's fill the
///   accent at 55 % over bg; the black-key rows 2/9 of the way from the panel's ground to the
///   white-key rows; an unlit LED a third of the way; the picker's rows 1/5 and 1/2 of the way
///   from the panel's ground to the hover band; the meters' ground the readouts' glass; the readouts'
///   digits the text; the lanes, the white-key rows, the strips and the Library's selected row
///   the raised band, its previewing row the active band; the bar lines and the knob ring
///   `line_strong`, the lanes' beat lines `line_soft`; the buttons' edge, the knob's face, the
///   fader's slot and the seam bg; a lane's shaded bar [`crate::ui::style::shaded`]; the loop
///   range's lanes the lane with Graphite's warm cast, 4 redder, 3 less green, 13 less blue;
///   the name box 6 lighter a channel than a button (every set's);
/// - after Walnut's, the set Gilt replaces: a button the hover band's face (Walnut's was a step
///   under it) and the active band under the pointer; a route that is off the active band 2
///   lighter a channel (the first unlit step's tone before the steps were darkened); a key's edge
///   5/8 of the white key as designed over the black key;
/// - new here: the lanes' bar lines half way from `line_soft` to `line_strong`.
///
/// The disabled text and the two unlit step tones `tests/fixtures/gen_ui_contrast.py` works out:
/// the disabled text the secondary's CIE L* times Slate's share, the unlit steps the tones they
/// were (the route's, then the hover band) times Graphite's shares from its shade down, a* and b* kept.
pub const GILT: Palette = Palette {
    bg: hex(0x251812),
    panel: hex(0x2E201B),
    raised: hex(0x382A24),
    hover: hex(0x473832),
    active: hex(0x54453F),
    window: hex(0x2B1E18),
    text: hex(0xD7C1B3),
    text_secondary: hex(0xB89E8D),
    text_disabled: hex(0x7A6353),
    lcd_text: hex(0xD7C1B3),
    lcd: hex(0x21140D),
    line_strong: hex(0x9C7854),
    line_soft: hex(0x3F312B),
    grid_beat: hex(0x644D3B),
    bar_line: hex(0x9C7854),
    lane_bar_line: hex(0x6E5540),
    lane_beat_line: hex(0x3F312B),
    lane: hex(0x382A24),
    lane_alt: hex(0x3D2F29),
    roll_row_white: hex(0x382A24),
    roll_row_black: hex(0x30221D),
    button: hex(0x473832),
    button_hover: hex(0x54453F),
    button_edge: hex(0x251812),
    name_button: hex(0x4D3E38),
    knob_ring: hex(0x9C7854),
    knob_face: hex(0x251812),
    fader_track: hex(0x251812),
    fader_cap: hex(0xE0A265),
    strip: hex(0x382A24),
    strip_selected: hex(0x54453F),
    led_off: hex(0x31231E),
    route_off: hex(0x564741),
    row_tones: [hex(0x332520), hex(0x3B2C27)],
    slider_fill: hex(0x97724E),
    accent: hex(0xF4BC7F),
    toolbar_line: hex(0x9C7854),
    seam: hex(0x251812),
    key_white: hex(0xBEA99B),
    key_black: hex(0x251812),
    key_edge: hex(0x948277),
    key_label: hex(0x2B1E18),
    browser_previewing: hex(0x54453F),
    browser_selected: hex(0x382A24),
    step_unlit: [hex(0x40322C), hex(0x372923)],
    loop_tint: hex(0x3C2717),
    meter_ground: hex(0x21140D),
    // AUDIO_BODY (23332C, CIE L* 19.67, a* −8.46, b* 2.32) reads 1.04:1 on these lanes (raised,
    // 382A24): a hue apart from the brown, as on the other sets' lanes, but lifted all the same,
    // the lift Gilt took while its lanes were AUDIO_BODY's own green (1.06:1 there). Lifted by the
    // L* law the sets' text tiers and white keys follow: its L* raised to the least whole L* at
    // which it reads 1.5:1 on raised, a* and b* kept (`gen_ui_contrast.py`'s audio body): L* 30
    // gives 3A4A43, 1.47:1; L* 31 gives 3C4D45, 1.53:1.
    audio_clip_body: hex(0x3C4D45),
};

/// Rose gold on deep plum, garnet set in rose gold: the ground a dark red-violet, the accent a
/// pale rose gold, the lit edges and the knob rings a darker one, the fader caps between the
/// two. Why this violet and no bluer: the automation clips' body is a dark violet; a plum this
/// red keeps the lanes apart from it and from the missing-audio clip's dark red (CIEDE2000
/// about 7 from each, near Graphite's 8 from its nearest clip body). The rose gold is lighter
/// and paler than Record's red, 23 from it (Graphite's orange is 26, Gilt's gold 30), so a lit
/// button reads apart from an armed one by its lightness more than its hue. Designed in CIE LCh
/// (the surfaces' hue 350°, their lightness near Slate's, deeper than Gilt's): bg to
/// the secondary text, the accent, the two lines, the white key as designed (E8D6D2, a blush ivory) and
/// the fader cap; the rest derived as [`GILT`]'s, the readouts' glass bg 3 darker a channel as
/// Slate's.
pub const GARNET: Palette = Palette {
    bg: hex(0x26161D),
    panel: hex(0x301B25),
    raised: hex(0x3B222E),
    hover: hex(0x492E3A),
    active: hex(0x583947),
    window: hex(0x2D1A22),
    text: hex(0xF3DEDB),
    text_secondary: hex(0xBB9FA8),
    text_disabled: hex(0x7D636B),
    lcd_text: hex(0xF3DEDB),
    lcd: hex(0x23131A),
    line_strong: hex(0x9F6A5A),
    line_soft: hex(0x422A35),
    grid_beat: hex(0x674444),
    bar_line: hex(0x9F6A5A),
    lane_bar_line: hex(0x714A48),
    lane_beat_line: hex(0x422A35),
    lane: hex(0x3B222E),
    lane_alt: hex(0x402733),
    roll_row_white: hex(0x3B222E),
    roll_row_black: hex(0x321D27),
    button: hex(0x492E3A),
    button_hover: hex(0x583947),
    button_edge: hex(0x26161D),
    name_button: hex(0x4F3440),
    knob_ring: hex(0x9F6A5A),
    knob_face: hex(0x26161D),
    fader_track: hex(0x26161D),
    fader_cap: hex(0xD9967E),
    strip: hex(0x3B222E),
    strip_selected: hex(0x583947),
    led_off: hex(0x341D28),
    route_off: hex(0x5A3B49),
    row_tones: [hex(0x351F29), hex(0x3D2530)],
    slider_fill: hex(0x956C62),
    accent: hex(0xF0B39B),
    toolbar_line: hex(0x9F6A5A),
    seam: hex(0x26161D),
    key_white: hex(0xCDBBB7),
    key_black: hex(0x26161D),
    key_edge: hex(0x9F8E8E),
    key_label: hex(0x2D1A22),
    browser_previewing: hex(0x583947),
    browser_selected: hex(0x3B222E),
    step_unlit: [hex(0x452835), hex(0x3A202C)],
    loop_tint: hex(0x3F1F21),
    meter_ground: hex(0x23131A),
    audio_clip_body: crate::ui::style::AUDIO_BODY,
};

/// Ice over deep navy water: the ground a saturated navy, the accent a pale ice blue, the lit
/// edges and the knob rings a steel blue, the fader caps frosted steel; the one cool accent of
/// the sets. Why ice and not the coral a navy is often paired with: a coral lies within
/// CIEDE2000 17 of Record's red (Graphite's orange is 26 from it), so a lit button would read
/// as armed; the ice is 59 from it. Why navy and not a charcoal: a charcoal's lanes lie within
/// 3 of Graphite's, the same world; the navy's are 11 from Graphite's and Slate's. The clips'
/// blue keeps its 3:1 on the navy lanes by its lightness. Designed in CIE LCh (the surfaces'
/// hue 265°, their lightness that of Gilt's green set before 0.1.4): bg to the secondary text, the accent, the two lines, the white key
/// as designed (D1DCE2, an ice white) and the fader cap; the rest derived as [`GILT`]'s, the
/// readouts' glass bg 3 darker a channel as Slate's.
pub const FJORD: Palette = Palette {
    bg: hex(0x092237),
    panel: hex(0x0E2B43),
    raised: hex(0x123652),
    hover: hex(0x1E4465),
    active: hex(0x2C5275),
    window: hex(0x0D283E),
    text: hex(0xD9E4EA),
    text_secondary: hex(0x96A8B6),
    text_disabled: hex(0x5A6C78),
    lcd_text: hex(0xD9E4EA),
    lcd: hex(0x061F34),
    line_strong: hex(0x578296),
    line_soft: hex(0x1E3D58),
    grid_beat: hex(0x355971),
    bar_line: hex(0x578296),
    lane_bar_line: hex(0x3B6077),
    lane_beat_line: hex(0x1E3D58),
    lane: hex(0x123652),
    lane_alt: hex(0x173B57),
    roll_row_white: hex(0x123652),
    roll_row_black: hex(0x0F2D46),
    button: hex(0x1E4465),
    button_hover: hex(0x2C5275),
    button_edge: hex(0x092237),
    name_button: hex(0x244A6B),
    knob_ring: hex(0x578296),
    knob_face: hex(0x092237),
    fader_track: hex(0x092237),
    fader_cap: hex(0xA2C0CC),
    strip: hex(0x123652),
    strip_selected: hex(0x2C5275),
    led_off: hex(0x0F2F48),
    route_off: hex(0x2E5477),
    row_tones: [hex(0x11304A), hex(0x163854)],
    slider_fill: hex(0x56899C),
    accent: hex(0x95DDEF),
    toolbar_line: hex(0x578296),
    seam: hex(0x092237),
    key_white: hex(0xB6C1C7),
    key_black: hex(0x092237),
    key_edge: hex(0x8696A2),
    key_label: hex(0x0D283E),
    browser_previewing: hex(0x2C5275),
    browser_selected: hex(0x123652),
    step_unlit: [hex(0x0E3C5D), hex(0x033352)],
    loop_tint: hex(0x163345),
    meter_ground: hex(0x061F34),
    audio_clip_body: crate::ui::style::AUDIO_BODY,
};

/// A theme: which [`Palette`] the app draws with. Slate is the default, the set a new user
/// sees and an absent `"ui_theme"` reads, and it comes first where Settings offers them, then
/// Graphite. Why Slate: it is the set worked in day to day, so a new user starts in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeId {
    Graphite,
    #[default]
    Slate,
    Gilt,
    Garnet,
    Fjord,
}

/// Names a set had in an earlier build, each read as the set it became: Walnut was replaced by
/// Gilt before any release shipped it, so a settings file of a build in between still opens.
const FORMER_NAMES: [(&str, ThemeId); 1] = [("walnut", ThemeId::Gilt)];

impl ThemeId {
    /// In the order Settings offers them: the default first.
    pub const ALL: [ThemeId; 5] = [
        ThemeId::Slate,
        ThemeId::Graphite,
        ThemeId::Gilt,
        ThemeId::Garnet,
        ThemeId::Fjord,
    ];

    /// Its name in settings.json (`"ui_theme"`) and on the command line (`--theme`).
    pub fn name(self) -> &'static str {
        match self {
            ThemeId::Graphite => "graphite",
            ThemeId::Slate => "slate",
            ThemeId::Gilt => "gilt",
            ThemeId::Garnet => "garnet",
            ThemeId::Fjord => "fjord",
        }
    }

    /// Its name as Settings shows it.
    pub fn title(self) -> &'static str {
        match self {
            ThemeId::Graphite => "Graphite",
            ThemeId::Slate => "Slate",
            ThemeId::Gilt => "Gilt",
            ThemeId::Garnet => "Garnet",
            ThemeId::Fjord => "Fjord",
        }
    }

    /// The theme named `name`, in any case, or once named so ([`FORMER_NAMES`]); none for a
    /// name this build does not know.
    pub fn from_name(name: &str) -> Option<Self> {
        let named = |known: &str| known.eq_ignore_ascii_case(name);
        Self::ALL
            .into_iter()
            .find(|theme| named(theme.name()))
            .or_else(|| {
                FORMER_NAMES
                    .into_iter()
                    .find_map(|(former, theme)| named(former).then_some(theme))
            })
    }

    pub fn palette(self) -> &'static Palette {
        match self {
            ThemeId::Graphite => &GRAPHITE,
            ThemeId::Slate => &SLATE,
            ThemeId::Gilt => &GILT,
            ThemeId::Garnet => &GARNET,
            ThemeId::Fjord => &FJORD,
        }
    }

    /// The set's panel and its own accent, as the Theme page's key shows them beside its name.
    /// Why its own accent and never the user's: the key says what the set is, whatever accent
    /// is drawn over it now.
    pub fn swatch(self) -> [Color; 2] {
        let set = self.palette();
        [set.panel, set.accent]
    }
}

/// The palette every window draws with now. Its accent and slider fill are the theme's own:
/// draw with [`accent`] and [`slider_fill`], which give the user's when there is one.
pub fn palette() -> &'static Palette {
    current::get().palette()
}

/// Draws every window with `theme`'s palette from the next frame on. What a canvas keeps of an
/// earlier frame keeps its colours until cleared (the app clears every cache it holds). An
/// accent of the user's own stays over the new theme's.
pub fn set_theme(theme: ThemeId) {
    current::set(theme);
}

/// Draws every window's accent in `accent` from the next frame on, over every theme's own;
/// none: each theme's own again. Caches keep their colours until cleared, as with a theme.
pub fn set_accent(accent: Option<Rgb>) {
    current::set_accent(accent);
}

/// The accent of the user's own drawn now, none when each theme's own is.
pub fn own_accent() -> Option<Rgb> {
    current::accent()
}

/// The accent every window draws with: the user's own when there is one, else the theme's.
/// Selection, the playhead, a running transport, a lit choice.
pub fn accent() -> Color {
    own_accent().map_or(palette().accent, Rgb::color)
}

/// A parameter slider's filled part: the theme's own, or, with an accent of the user's own,
/// that accent at 55 % over the theme's bg, the law every set's fill but Graphite's follows. The
/// theme's own accent typed as the user's keeps the theme's fill (Graphite's is hand-set, off
/// the law), so typing it back changes nothing on the screen.
pub fn slider_fill() -> Color {
    let palette = palette();
    match own_accent().map(Rgb::color) {
        Some(own) if own != palette.accent => mix(own, palette.bg, 55, 100),
        _ => palette.slider_fill,
    }
}

/// The theme and the accent drawn now: one for the whole app. The theme as an index into the
/// fixed sets, the accent as its 24 bits with bit 24 set, 0 for none. Why atomics and no lock:
/// each is one word, read on every draw, and nothing is guarded but which value.
#[cfg(not(test))]
mod current {
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use super::{Rgb, ThemeId};

    /// The theme's place in [`ThemeId::ALL`]; none (past its end) until the app sets one, which
    /// reads as the default.
    static THEME: AtomicUsize = AtomicUsize::new(usize::MAX);
    static ACCENT: AtomicU32 = AtomicU32::new(0);
    const SET: u32 = 1 << 24;

    pub fn get() -> ThemeId {
        let index = THEME.load(Ordering::Relaxed);
        ThemeId::ALL.get(index).copied().unwrap_or_default()
    }

    pub fn set(theme: ThemeId) {
        let index = ThemeId::ALL.iter().position(|&each| each == theme);
        THEME.store(index.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn accent() -> Option<Rgb> {
        let word = ACCENT.load(Ordering::Relaxed);
        let [_, r, g, b] = word.to_be_bytes();
        (word & SET != 0).then_some(Rgb([r, g, b]))
    }

    pub fn set_accent(accent: Option<Rgb>) {
        let word = accent.map_or(0, |Rgb([r, g, b])| SET | u32::from_be_bytes([0, r, g, b]));
        ACCENT.store(word, Ordering::Relaxed);
    }
}

/// Under `cargo test` each test's thread keeps its own theme and accent, starting in Graphite,
/// the set the tests' colours are written in (an app booted in a test sets the theme its
/// script names or its settings keep). Why: the tests run on many threads at once, and one that
/// switches either must not repaint another's colours.
#[cfg(test)]
mod current {
    use std::cell::Cell;

    use super::{Rgb, ThemeId};

    thread_local! {
        static THEME: Cell<ThemeId> = const { Cell::new(ThemeId::Graphite) };
        static ACCENT: Cell<Option<Rgb>> = const { Cell::new(None) };
    }

    pub fn get() -> ThemeId {
        THEME.with(Cell::get)
    }

    pub fn set(theme: ThemeId) {
        THEME.with(|current| current.set(theme));
    }

    pub fn accent() -> Option<Rgb> {
        ACCENT.with(Cell::get)
    }

    pub fn set_accent(accent: Option<Rgb>) {
        ACCENT.with(|current| current.set(accent));
    }
}

#[cfg(test)]
mod tests {
    use iced::Color;

    use super::*;
    use crate::ui::style;

    /// What `tests/fixtures/gen_ui_contrast.py` works out from the design's colours and the WCAG
    /// formula.
    fn known() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/ui_contrast_expected.json"
        )))
        .expect("ui_contrast_expected.json")
    }

    fn hex_of(color: Color) -> String {
        let [r, g, b, _] = color.into_rgba8();
        format!("{r:02X}{g:02X}{b:02X}")
    }

    /// A colour a rule names: one of the palette's, or a content colour by its constant's name
    /// (`channel_1` the first channel colour).
    fn named(palette: &Palette, name: &str) -> Color {
        if let Some(number) = name.strip_prefix("channel_") {
            let number: usize = number.parse().expect("a channel number");
            return style::CHANNEL_COLORS[number - 1];
        }
        match name {
            "bg" => palette.bg,
            "panel" => palette.panel,
            "raised" => palette.raised,
            "hover" => palette.hover,
            "active" => palette.active,
            "text" => palette.text,
            "text_secondary" => palette.text_secondary,
            "text_disabled" => palette.text_disabled,
            "unlit_0" => palette.step_unlit[0],
            "unlit_1" => palette.step_unlit[1],
            "accent" => palette.accent,
            "line_strong" => palette.line_strong,
            "line_soft" => palette.line_soft,
            "audio_clip_body" => palette.audio_clip_body,
            "CLIP_BODY" => style::CLIP_BODY,
            "AUDIO_TITLE" => style::AUDIO_TITLE,
            "AUTOMATION_TITLE" => style::AUTOMATION_TITLE,
            "LED_ON" => style::LED_ON,
            "RECORD_ARMED" => style::RECORD_ARMED,
            other => panic!("no colour named {other}"),
        }
    }

    // The themes' contrast rules, against gen_ui_contrast.py's known answers: each theme's colours are
    // the design's (the text tiers and the unlit steps the generator works out by its laws), every
    // rule's ratio is the generator's to 2 decimals, and the rules each theme misses are the ones
    // it lists. Graphite is the look the app always had and misses four (the clip blue 2.86, the
    // two title strips, and line_strong on panel 2.22); its secondary text, 9FA3A9, passes 4.5 on
    // bg, panel and raised (6.36, 5.38, 4.71), where the dim text it replaced read 3.42, 2.89 and
    // 2.53, and every channel colour stands 3:1 off both its unlit steps (the red 3.16 off
    // 34383E, 2.21 off the 4A4F57 before); Slate, Gilt, Garnet and Fjord miss the audio and
    // automation clips' title strips, which no dark surface can meet (the generator's ceiling:
    // the automation strip's is below 0, the audio strip's darker than every theme's bg). The
    // audio clip's body, 1.5:1 on raised: Gilt's, lifted to 3C4D45, meets it (1.53); the four
    // that keep AUDIO_BODY (23332C) miss it, Graphite 1.11, Slate 1.09, Garnet 1.09, Fjord 1.06,
    // its green a hue apart from their grey, slate, plum and blue lanes (the rule measures
    // luminance alone). Each theme's white key is the generator's, a shade down from the white
    // it was designed with.
    #[test]
    fn each_theme_meets_the_contrast_rules_its_known_answers_say() {
        let known = known();
        let rules = known["rules"].as_array().expect("rules");
        for (content, hex) in known["content"].as_object().expect("content") {
            assert_eq!(
                hex_of(named(&GRAPHITE, content)),
                hex.as_str().expect("a hex colour"),
                "{content}, the same in every theme"
            );
        }
        let unmeetable = ["AUDIO_TITLE/raised", "AUTOMATION_TITLE/raised"];
        let body = "audio_clip_body/raised";
        for theme in ThemeId::ALL {
            let palette = theme.palette();
            let answers = &known["themes"][theme.name()];
            for (token, hex) in answers["colors"].as_object().expect("colors") {
                assert_eq!(
                    hex_of(named(palette, token)),
                    hex.as_str().expect("a hex colour"),
                    "{}'s {token}",
                    theme.name()
                );
            }
            let mut failing = Vec::new();
            for rule in rules {
                let id = rule["id"].as_str().expect("an id");
                let color = named(palette, rule["color"].as_str().expect("a colour"));
                let ground = named(palette, rule["on"].as_str().expect("a ground"));
                let ratio = style::contrast(color, ground);
                let expected = answers["ratios"][id].as_f64().expect("a ratio") as f32;
                assert!(
                    (ratio - expected).abs() <= 0.0051,
                    "{} {id}: {ratio}, known {expected}",
                    theme.name()
                );
                let least = rule["least"].as_f64().expect("a least") as f32;
                let most = rule["most"]
                    .as_f64()
                    .map_or(f32::INFINITY, |most| most as f32);
                if !(least..=most).contains(&ratio) {
                    failing.push(id);
                }
            }
            let listed: Vec<&str> = answers["failing"]
                .as_array()
                .expect("failing")
                .iter()
                .map(|id| id.as_str().expect("an id"))
                .collect();
            assert_eq!(failing, listed, "{}", theme.name());
            if theme == ThemeId::Gilt {
                assert_eq!(failing, unmeetable, "Gilt misses only those");
            } else if theme != ThemeId::Graphite {
                assert_eq!(
                    failing,
                    [unmeetable[0], unmeetable[1], body],
                    "{} misses only those and the audio body",
                    theme.name()
                );
            }
            let lifted = theme == ThemeId::Gilt;
            assert_eq!(
                palette.audio_clip_body == style::AUDIO_BODY,
                !lifted,
                "{}: AUDIO_BODY unless lifted",
                theme.name()
            );
            assert_eq!(
                hex_of(palette.key_white),
                known["white_keys"]["themes"][theme.name()]["shaded"]
                    .as_str()
                    .expect("a hex colour"),
                "{}'s white key",
                theme.name()
            );
        }
    }

    /// `color` with `by` added to its red, green and blue, each held to 0..=255.
    fn lifted(color: Color, by: [i16; 3]) -> Color {
        let [r, g, b, _] = color.into_rgba8();
        let lift = |value: u8, by: i16| {
            u8::try_from(i16::from(value) + by).unwrap_or(if by < 0 { 0 } else { u8::MAX })
        };
        Color::from_rgb8(lift(r, by[0]), lift(g, by[1]), lift(b, by[2]))
    }

    // The tokens the themes' design derives from others (GILT's comment lists them). In every
    // theme: a lane's shaded bar is the lane SHADE_LIFT lighter; the seam is the buttons' edge,
    // the toolbar's line the strong line, the selected strip the active band; the name box a
    // button 6 lighter a channel; the loop range's lanes the lane 4 redder, 3 less green and 13
    // less blue. In every set after Graphite, whose are today's hand-set values, Slate's laws;
    // in Gilt, Garnet and Fjord also those after Walnut's and the lanes' bar lines half way
    // between the two lines. Worked by hand (rounded half up): the readouts' glass bg 3 darker
    // a channel, Slate 121418, Garnet 23131A, Fjord 061F34, and Gilt's the design's 21140D; the
    // slider's fill the accent at 55 % over bg, Slate 0.55 · F29A2E + 0.45 · 15171B = (142.55,
    // 95.05, 37.45) → 8F5F25, Gilt 0.55 · F4BC7F + 0.45 · 251812 = (150.85, 114.2, 77.95) →
    // 97724E, Garnet 0.55 · F0B39B + 0.45 · 26161D = (149.1, 108.35, 98.3) → 956C62, Fjord
    // 0.55 · 95DDEF + 0.45 · 092237 = (86, 136.85, 156.2) → 56899C; the beat lines 2/5 of the
    // strong line over the soft one, Slate 0.4 · 7A818B + 0.6 · 2F343B = (77, 82.8, 91) → 4D535B,
    // Gilt 0.4 · 9C7854 + 0.6 · 3F312B = (100.2, 77.4, 59.4) → 644D3B, Garnet 0.4 · 9F6A5A + 0.6 ·
    // 422A35 = (103.2, 67.6, 67.8) → 674444, Fjord 0.4 · 578296 + 0.6 · 1E3D58 = (52.8, 88.6,
    // 112.8) → 355971; Slate's window its panel's ground 2 darker, 1A1D22. A key's edge is 5/8
    // of the white key as the generator lists it designed over the black key.
    #[test]
    fn the_derived_tokens_follow_the_design() {
        for theme in ThemeId::ALL {
            let palette = theme.palette();
            let name = theme.name();
            assert_eq!(style::shaded(palette.lane), palette.lane_alt, "{name}");
            assert_eq!(palette.seam, palette.button_edge, "{name}");
            assert_eq!(palette.toolbar_line, palette.line_strong, "{name}");
            assert_eq!(palette.strip_selected, palette.active, "{name}");
            assert_eq!(
                lifted(palette.button, [6; 3]),
                palette.name_button,
                "{name}"
            );
            assert_eq!(
                lifted(palette.lane, [4, -3, -13]),
                palette.loop_tint,
                "{name}"
            );
        }
        for theme in [
            ThemeId::Slate,
            ThemeId::Gilt,
            ThemeId::Garnet,
            ThemeId::Fjord,
        ] {
            let palette = theme.palette();
            let name = theme.name();
            let raised = [
                palette.lane,
                palette.roll_row_white,
                palette.strip,
                palette.browser_selected,
            ];
            assert_eq!(raised, [palette.raised; 4], "{name}");
            let bg = [
                palette.button_edge,
                palette.knob_face,
                palette.fader_track,
                palette.key_black,
            ];
            assert_eq!(bg, [palette.bg; 4], "{name}");
            assert_eq!(palette.browser_previewing, palette.active, "{name}");
            assert_eq!(
                [palette.bar_line, palette.knob_ring],
                [palette.line_strong; 2],
                "{name}"
            );
            assert_eq!(palette.lane_beat_line, palette.line_soft, "{name}");
            assert_eq!(palette.key_label, palette.window, "{name}");
            assert_eq!(palette.meter_ground, palette.lcd, "{name}");
            assert_eq!(palette.lcd_text, palette.text, "{name}");
            assert_eq!(
                palette.slider_fill,
                mix(palette.accent, palette.bg, 55, 100),
                "{name}"
            );
            assert_eq!(
                palette.grid_beat,
                mix(palette.line_strong, palette.line_soft, 2, 5),
                "{name}"
            );
            assert_eq!(
                palette.roll_row_black,
                mix(palette.raised, palette.panel, 2, 9),
                "{name}"
            );
            assert_eq!(
                palette.led_off,
                mix(palette.raised, palette.panel, 1, 3),
                "{name}"
            );
            assert_eq!(
                palette.row_tones,
                [
                    mix(palette.hover, palette.panel, 1, 5),
                    mix(palette.hover, palette.panel, 1, 2)
                ],
                "{name}"
            );
        }
        assert_eq!(SLATE.route_off, SLATE.step_unlit[0]);
        let designed = known()["white_keys"]["themes"].clone();
        for theme in [ThemeId::Gilt, ThemeId::Garnet, ThemeId::Fjord] {
            let palette = theme.palette();
            let name = theme.name();
            assert_eq!(
                [palette.button, palette.button_hover],
                [palette.hover, palette.active],
                "{name}"
            );
            assert_eq!(palette.route_off, lifted(palette.active, [2; 3]), "{name}");
            assert_eq!(
                palette.lane_bar_line,
                mix(palette.line_strong, palette.line_soft, 1, 2),
                "{name}"
            );
            let white = designed[name]["designed"].as_str().expect("a hex colour");
            let white = Rgb::parse(white).expect("a colour").color();
            assert_eq!(
                palette.key_edge,
                mix(white, palette.key_black, 5, 8),
                "{name}"
            );
        }
        let derived =
            |palette: &Palette| [palette.lcd, palette.slider_fill, palette.grid_beat].map(hex_of);
        assert_eq!(derived(&SLATE), ["121418", "8F5F25", "4D535B"]);
        assert_eq!(derived(&GILT), ["21140D", "97724E", "644D3B"]);
        assert_eq!(derived(&GARNET), ["23131A", "956C62", "674444"]);
        assert_eq!(derived(&FJORD), ["061F34", "56899C", "355971"]);
        assert_eq!(hex_of(SLATE.window), "1A1D22");
    }

    /// What `tests/fixtures/gen_accent_expected.py` works out for the accent of the user's own.
    fn accent_known() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/accent_expected.json"
        )))
        .expect("accent_expected.json")
    }

    fn rgb(value: &serde_json::Value) -> Rgb {
        let hex = value.as_str().expect("a hex colour");
        Rgb::parse(hex).unwrap_or_else(|| panic!("{hex} is a colour"))
    }

    // An accent of the user's own is drawn over every theme's, through a theme switch, until
    // none puts each theme's own back; the slider's fill follows it by the law the fixed sets
    // follow, against the generator's: the probes' blue #3FA7D6 at 55 % over Graphite's bg is
    // #316B86 and over Gilt's #33677E, and every other set's own accent typed as the user's gives
    // its own fill back (Slate #8F5F25, Gilt #97724E, Garnet #956C62, Fjord #56899C). Graphite's
    // orange typed as the user's keeps
    // Graphite's hand-set fill, #8A5A24, so typing it changes nothing on the screen. The theme's
    // own two stay its own.
    #[test]
    fn an_accent_of_ones_own_is_drawn_over_every_themes() {
        let known = accent_known();
        let fills = &known["slider_fill"];
        let blue = rgb(&known["probe"]["hex"]);
        assert_eq!(
            (accent(), slider_fill()),
            (GRAPHITE.accent, GRAPHITE.slider_fill)
        );
        set_accent(Some(blue));
        assert_eq!(own_accent(), Some(blue));
        assert_eq!(accent(), blue.color());
        assert_eq!(Rgb::of(slider_fill()), rgb(&fills["probe"]["graphite"]));
        set_theme(ThemeId::Gilt);
        assert_eq!(accent(), blue.color(), "kept through a theme switch");
        assert_eq!(Rgb::of(slider_fill()), rgb(&fills["probe"]["gilt"]));
        assert_eq!(
            palette().accent,
            GILT.accent,
            "the theme's own stays its own"
        );
        for theme in [
            ThemeId::Slate,
            ThemeId::Gilt,
            ThemeId::Garnet,
            ThemeId::Fjord,
        ] {
            let name = theme.name();
            set_theme(theme);
            set_accent(Some(Rgb::of(theme.palette().accent)));
            let own = rgb(&fills["own"][name]);
            assert_eq!(Rgb::of(slider_fill()), own, "{name}");
            assert_eq!(Rgb::of(theme.palette().slider_fill), own, "{name}'s set");
        }
        set_theme(ThemeId::Graphite);
        set_accent(Some(Rgb::of(GRAPHITE.accent)));
        assert_eq!(Rgb::of(slider_fill()).hex(), "#8A5A24");
        set_accent(None);
        assert_eq!(
            (accent(), slider_fill()),
            (GRAPHITE.accent, GRAPHITE.slider_fill)
        );
    }

    // The note under the accent's row shows below 3:1 on the theme's panel: the generator's
    // greys either side of it on Graphite's panel #2B2E33, #767676 (2.9996) and #777777
    // (3.0425), worked out by WCAG's formula to 4 decimals and held to 0.0005 here.
    #[test]
    fn the_contrast_note_starts_under_three_to_one() {
        let known = accent_known();
        let contrast = &known["contrast"];
        let panel = rgb(&contrast["panel"]).color();
        assert_eq!(panel, GRAPHITE.panel);
        assert_eq!(
            contrast["least"].as_f64(),
            Some(f64::from(ACCENT_LEAST_CONTRAST))
        );
        for (side, low) in [("below", true), ("above", false)] {
            let color = rgb(&contrast[side]["hex"]).color();
            let ratio = style::contrast(color, panel);
            let expected = contrast[side]["ratio"].as_f64().expect("a ratio") as f32;
            assert!((ratio - expected).abs() <= 0.0005, "{side}: {ratio}");
            assert_eq!(ratio < ACCENT_LEAST_CONTRAST, low, "{side}");
        }
    }

    // A colour is "#RRGGBB" in settings.json, in capitals; read in any case, the # optional and
    // spaces around ignored, as typed; three digits, seven, a letter past F or a sign are none.
    #[test]
    fn a_colour_of_ones_own_is_written_as_hex() {
        let blue = Rgb([0x3F, 0xA7, 0xD6]);
        assert_eq!(blue.hex(), "#3FA7D6");
        for typed in ["#3FA7D6", "#3fa7d6", "3FA7D6", " #3FA7D6 "] {
            assert_eq!(Rgb::parse(typed), Some(blue), "{typed:?}");
        }
        for refused in [
            "", "#", "#36D", "#3FA7D6F", "#3FA7DG", "+3FA7D6", "#+3FA7D", "##3FA7D",
        ] {
            assert_eq!(Rgb::parse(refused), None, "{refused:?}");
        }
        assert_eq!(Rgb::of(GRAPHITE.accent).hex(), "#F29A2E");
        assert_eq!(Rgb::parse("#F29A2E").map(Rgb::color), Some(GRAPHITE.accent));
    }

    // A theme's name is what settings.json and --theme hold, read in any case; "walnut", the name
    // Gilt had in a build before, is read as Gilt, and no theme has a former name as its own; a
    // name this build does not know is none. Switching draws from the theme's palette, every
    // set its own; Graphite is the default.
    #[test]
    fn a_theme_is_named_and_switched_whole() {
        for theme in ThemeId::ALL {
            assert_eq!(ThemeId::from_name(theme.name()), Some(theme));
            assert!(
                FORMER_NAMES
                    .iter()
                    .all(|&(former, _)| former != theme.name()),
                "{}",
                theme.name()
            );
        }
        assert_eq!(ThemeId::from_name("FJORD"), Some(ThemeId::Fjord));
        assert_eq!(ThemeId::from_name("Walnut"), Some(ThemeId::Gilt));
        assert_eq!(ThemeId::from_name("oak"), None);
        let sets: Vec<&Palette> = ThemeId::ALL.into_iter().map(ThemeId::palette).collect();
        for (index, set) in sets.iter().enumerate() {
            assert!(!sets[..index].contains(set), "set {index} is its own");
        }
        assert_eq!(palette(), &GRAPHITE, "the default");
        set_theme(ThemeId::Gilt);
        assert_eq!(palette(), &GILT);
        set_theme(ThemeId::Graphite);
        assert_eq!(palette(), &GRAPHITE);
    }

    // The Theme page's key of each set shows its panel and its own accent as the design gives
    // them (the generator's colours: Gilt 2E201B and F4BC7F), whatever theme or accent is drawn
    // now: an accent of the user's own and another theme leave every key's swatch as it was.
    #[test]
    fn a_themes_swatch_is_its_own_panel_and_accent() {
        let known = known();
        let swatches = || ThemeId::ALL.map(ThemeId::swatch);
        let before = swatches();
        for (theme, [panel, accent]) in ThemeId::ALL.into_iter().zip(before) {
            let colors = &known["themes"][theme.name()]["colors"];
            let design = |name: &str| colors[name].as_str().expect("a hex colour").to_owned();
            assert_eq!(
                (hex_of(panel), hex_of(accent)),
                (design("panel"), design("accent")),
                "{}",
                theme.name()
            );
        }
        assert_eq!(
            ThemeId::Gilt.swatch().map(hex_of),
            ["2E201B".to_owned(), "F4BC7F".to_owned()]
        );
        set_theme(ThemeId::Fjord);
        set_accent(Some(Rgb([0x3F, 0xA7, 0xD6])));
        assert_eq!(swatches(), before);
        set_accent(None);
        set_theme(ThemeId::Graphite);
    }
}
