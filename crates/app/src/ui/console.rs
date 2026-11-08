//! The console finish: the synth's instrument window drawn as a little sound board seen from
//! slightly above, lit from the top left. A chassis under the whole window, darker towards its
//! foot as a tilted surface is; each heading a raised module on it (its top and left edges lit,
//! its bottom and right ones shaded, its shadow falling right and down, its upper corners
//! screwed down) with its knobs in a sunken well and its name engraved; each knob a domed cap
//! (lit at its upper left, a glint along it) on a darker skirt, casting its own shadow; a
//! choice's button a raised key; a fader's track a slot and its cap raised.
//!
//! Every shade is one of the theme's palette colors in a new role ([`Shades`]), or black or
//! the text's color at some opacity, so the board stays the app's own in every theme. The
//! mixer draws the same blocks [`Finish::Flat`] through the same functions
//! ([`crate::ui::knob_block`]), so the two still answer alike: nothing here moves a control,
//! and a raised part's shadow falls inside the place it was given, so every press finds what
//! the flat look's would.

use std::f32::consts::{FRAC_1_SQRT_2, PI};

use iced::widget::canvas::gradient::Linear;
use iced::widget::canvas::path::Arc;
use iced::widget::canvas::{self, Fill, Frame, Geometry, LineCap, Path, Stroke};
use iced::{mouse, Color, Element, Point, Radians, Rectangle, Renderer, Size, Theme, Vector};

use crate::ui::palette::Palette;
use crate::ui::rack;
use crate::ui::style;

/// How a surface of knob blocks is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// The mixer's: flat grounds, flat dials and buttons.
    Flat,
    /// The synth window's: the board this module draws.
    Console,
}

/// The board's shades, each one of the theme's palette colors in a new role, so the board stays
/// the theme's own: Gilt's lit edges and lips are its gold `line_strong` and `knob_ring` on its
/// green lacquer, Garnet's rose gold on plum, Fjord's steel on navy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shades {
    /// The chassis's surface at its top and at its foot: darker than the window's ground round
    /// it and than every module on it, so the board reads as one slab.
    pub chassis_top: Color,
    pub chassis_foot: Color,
    /// The chassis's rim: its lit top and left edges (its shaded bottom and right ones are
    /// [`RIM_SHADED`]).
    pub rim_lit: Color,
    /// A module's face at its top and at its foot.
    pub face_top: Color,
    pub face_foot: Color,
    /// A raised part's lit edges, and its shaded ones (also an engraving's shaded wall and a
    /// screw's head).
    pub edge_lit: Color,
    pub edge_shaded: Color,
    /// The sunken well a module's controls sit in, and its lit lower and right lips.
    pub well: Color,
    pub well_lip: Color,
    /// A raised key's face.
    pub key: Color,
    /// A knob's dome across its diagonal: lit at its upper left, the flat knob's face at its
    /// lower right; and the skirt round it.
    pub dome_lit: Color,
    pub dome_shaded: Color,
    pub skirt: Color,
    /// The light a knob's dome or a screw's head catches at its upper left.
    pub glint: Color,
}

impl Shades {
    pub fn of(palette: &Palette) -> Self {
        Self {
            chassis_top: palette.window,
            chassis_foot: palette.lcd,
            rim_lit: palette.knob_ring,
            face_top: palette.name_button,
            face_foot: palette.strip,
            edge_lit: palette.line_strong,
            edge_shaded: palette.button_edge,
            well: palette.panel,
            well_lip: palette.knob_ring,
            key: palette.button_hover,
            dome_lit: palette.line_strong,
            dome_shaded: palette.knob_face,
            skirt: palette.button_edge,
            glint: Color {
                a: 0.35,
                ..palette.text
            },
        }
    }

    /// The shades of the palette drawn now.
    pub fn now() -> Self {
        Self::of(style::palette())
    }
}

/// The chassis rim's shaded bottom and right edges.
const RIM_SHADED: Color = Color::BLACK;

/// How far right and down a raised part's shadow falls; a module is drawn that much smaller
/// than its place so its shadow stays inside it.
pub const DROP: f32 = 2.0;
/// The shadow's two steps: how far each falls, and how dark it is.
const SHADOW_STEPS: [(f32, f32); 2] = [(DROP, 0.25), (DROP / 2.0, 0.4)];
/// The chassis's margin inside its place (the window's padding), which its shadow falls in.
const CHASSIS_MARGIN: f32 = 3.0;
const CHASSIS_RADIUS: f32 = 6.0;
const MODULE_RADIUS: f32 = 3.0;
const KEY_RADIUS: f32 = 2.0;
const WELL_RADIUS: f32 = 2.0;
const FADER_CAP_RADIUS: f32 = 1.0;
/// A screw's middle, in from its module's corner, and its radius.
const SCREW_INSET: f32 = 4.0;
const SCREW_RADIUS: f32 = 1.5;
/// The skirt's width round a knob's dome, as the flat knob's ring.
const SKIRT_WIDTH: f32 = 2.0;
/// The glint along a knob's dome: in from the dome's edge, its width, and the arc it spans
/// (clockwise from the right, so from left of the middle to above it).
const GLINT_INSET: f32 = 1.5;
const GLINT_WIDTH: f32 = 1.5;
const GLINT_ARC: (f32, f32) = (1.05 * PI, 1.45 * PI);

impl Finish {
    /// A knob `size` px across at `center` pointing at `fraction` of its sweep, as
    /// [`rack::draw_knob`] points.
    pub fn draw_knob(self, frame: &mut Frame, center: Point, size: f32, fraction: f32) {
        match self {
            Finish::Flat => rack::draw_knob(frame, center, size, fraction),
            Finish::Console => draw_domed_knob(frame, center, size, fraction),
        }
    }

    /// A choice's button at `rect`, under its words.
    pub fn draw_key(self, frame: &mut Frame, rect: Rectangle) {
        match self {
            Finish::Flat => {
                frame.fill_rectangle(rect.position(), rect.size(), style::palette().button)
            }
            Finish::Console => {
                let shades = Shades::now();
                draw_shadow(frame, rect, KEY_RADIUS);
                let edges = (shades.edge_lit, shades.edge_shaded);
                bevelled(frame, rect, KEY_RADIUS, edges, shades.key);
            }
        }
    }

    /// A fader's track or a readout's well at `rect`: sunk into the surface.
    pub fn draw_slot(self, frame: &mut Frame, rect: Rectangle) {
        match self {
            Finish::Flat => {
                frame.fill_rectangle(rect.position(), rect.size(), style::palette().fader_track)
            }
            Finish::Console => {
                let shades = Shades::now();
                let edges = (shades.edge_shaded, shades.well_lip);
                bevelled(frame, rect, 0.0, edges, style::palette().fader_track);
            }
        }
    }

    /// A fader's cap at `rect`: raised over its track.
    pub fn draw_cap(self, frame: &mut Frame, rect: Rectangle) {
        match self {
            Finish::Flat => {
                frame.fill_rectangle(rect.position(), rect.size(), style::palette().fader_cap)
            }
            Finish::Console => {
                draw_shadow(frame, rect, FADER_CAP_RADIUS);
                let edges = (style::palette().lcd_text, style::palette().knob_ring);
                bevelled(
                    frame,
                    rect,
                    FADER_CAP_RADIUS,
                    edges,
                    style::palette().fader_cap,
                );
            }
        }
    }

    /// A heading in `color` at `at`, which `write` writes: as it is, or engraved (the cut's
    /// shaded upper wall a pixel above it, under it).
    pub fn write_heading(self, at: Point, color: Color, mut write: impl FnMut(Point, Color)) {
        if self == Finish::Console {
            write(at - Vector::new(0.0, 1.0), Shades::now().edge_shaded);
        }
        write(at, color);
    }
}

/// A raised module in `place`: its shadow, then its face (lighter at its top, as a raised
/// surface lit from above is) with its edges bevelled, [`DROP`] smaller than the place so the
/// shadow stays in it, and a screw in each upper corner. Returns the face.
pub fn draw_module(frame: &mut Frame, place: Rectangle) -> Rectangle {
    let face = Rectangle::new(
        place.position(),
        Size::new(
            (place.width - DROP).max(0.0),
            (place.height - DROP).max(0.0),
        ),
    );
    let shades = Shades::now();
    draw_shadow(frame, face, MODULE_RADIUS);
    let lit = downwards(face, shades.face_top, shades.face_foot);
    bevelled(
        frame,
        face,
        MODULE_RADIUS,
        (shades.edge_lit, shades.edge_shaded),
        lit,
    );
    let top = face.y + SCREW_INSET;
    for x in [face.x + SCREW_INSET, face.x + face.width - SCREW_INSET] {
        draw_screw(frame, &shades, Point::new(x, top));
    }
    face
}

/// A sunken well at `rect`: its upper and left walls in shade, its lower and right lips lit.
pub fn draw_well(frame: &mut Frame, rect: Rectangle) {
    let shades = Shades::now();
    let edges = (shades.edge_shaded, shades.well_lip);
    bevelled(frame, rect, WELL_RADIUS, edges, shades.well);
}

/// Where the chassis lies in its place `size` across and down: [`CHASSIS_MARGIN`] inside it.
pub fn chassis_rect(size: Size) -> Rectangle {
    Rectangle::new(
        Point::new(CHASSIS_MARGIN, CHASSIS_MARGIN),
        Size::new(
            (size.width - 2.0 * CHASSIS_MARGIN).max(0.0),
            (size.height - 2.0 * CHASSIS_MARGIN).max(0.0),
        ),
    )
}

/// The chassis, filling its place: a canvas under the window's content, its drawing kept in
/// `cache`, which only a new size draws again (nothing else on it changes). Why a layer of its
/// own: a knob dragged redraws the knobs every frame, and the chassis then costs nothing.
pub fn chassis<'a, Message: 'a>(cache: &'a canvas::Cache) -> Element<'a, Message> {
    canvas::Canvas::new(Chassis { cache })
        .width(iced::Fill)
        .height(iced::Fill)
        .into()
}

struct Chassis<'a> {
    cache: &'a canvas::Cache,
}

impl<Message> canvas::Program<Message> for Chassis<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        vec![self.cache.draw(renderer, bounds.size(), draw_chassis)]
    }
}

/// The chassis in its frame: its shadow, then its surface (darker towards its foot, as a
/// surface tilted away from the light is) inside its rim.
fn draw_chassis(frame: &mut Frame) {
    let shades = Shades::now();
    let chassis = chassis_rect(frame.size());
    draw_shadow(frame, chassis, CHASSIS_RADIUS);
    let tilt = downwards(chassis, shades.chassis_top, shades.chassis_foot);
    let edges = (shades.rim_lit, RIM_SHADED);
    bevelled(frame, chassis, CHASSIS_RADIUS, edges, tilt);
}

/// A domed knob: its shadow, the darker skirt, the dome shaded across its diagonal from lit at
/// its upper left to dark at its lower right with a glint along its upper left, and the flat
/// knob's pointer.
fn draw_domed_knob(frame: &mut Frame, center: Point, size: f32, fraction: f32) {
    let shades = Shades::now();
    let radius = size / 2.0;
    for (step, opacity) in SHADOW_STEPS {
        let shadow = Path::circle(center + Vector::new(step, step), radius);
        frame.fill(&shadow, black(opacity));
    }
    frame.fill(&Path::circle(center, radius), shades.skirt);
    let dome = radius - SKIRT_WIDTH;
    let reach = Vector::new(dome, dome) * FRAC_1_SQRT_2;
    let lit = Linear::new(center - reach, center + reach)
        .add_stop(0.0, shades.dome_lit)
        .add_stop(1.0, shades.dome_shaded);
    frame.fill(&Path::circle(center, dome), lit);
    let glint = Path::new(|path| {
        path.arc(Arc {
            center,
            radius: dome - GLINT_INSET,
            start_angle: Radians(GLINT_ARC.0),
            end_angle: Radians(GLINT_ARC.1),
        });
    });
    let stroke = Stroke::default()
        .with_color(shades.glint)
        .with_width(GLINT_WIDTH)
        .with_line_cap(LineCap::Round);
    frame.stroke(&glint, stroke);
    rack::draw_pointer(frame, center, radius, fraction);
}

/// A screw's head at `center`: a dark dot with a glint at its upper left.
fn draw_screw(frame: &mut Frame, shades: &Shades, center: Point) {
    frame.fill(&Path::circle(center, SCREW_RADIUS), shades.edge_shaded);
    let glint = center - Vector::new(SCREW_RADIUS / 3.0, SCREW_RADIUS / 3.0);
    frame.fill(&Path::circle(glint, SCREW_RADIUS / 2.0), shades.glint);
}

/// The shadow a part at `rect` casts on what it stands on: two soft steps right and down,
/// which the part itself covers but for the band it does not.
fn draw_shadow(frame: &mut Frame, rect: Rectangle, radius: f32) {
    for (step, opacity) in SHADOW_STEPS {
        frame.fill(
            &rounded(rect + Vector::new(step, step), radius),
            black(opacity),
        );
    }
}

/// `rect` filled with `face`, its top and left edges a pixel of `upper` and its bottom and
/// right ones a pixel of `lower`: raised when `upper` is the lighter, sunken when the darker.
/// Three shapes stacked, so rounded corners need no stroking.
fn bevelled(
    frame: &mut Frame,
    rect: Rectangle,
    radius: f32,
    (upper, lower): (Color, Color),
    face: impl Into<Fill>,
) {
    frame.fill(&rounded(rect, radius), lower);
    let upper_left = Size::new((rect.width - 1.0).max(0.0), (rect.height - 1.0).max(0.0));
    frame.fill(
        &rounded(Rectangle::new(rect.position(), upper_left), radius),
        upper,
    );
    let inner = Rectangle::new(
        Point::new(rect.x + 1.0, rect.y + 1.0),
        Size::new((rect.width - 2.0).max(0.0), (rect.height - 2.0).max(0.0)),
    );
    frame.fill(&rounded(inner, (radius - 1.0).max(0.0)), face);
}

/// A gradient from `top` at the top of `rect` to `foot` at its foot.
fn downwards(rect: Rectangle, top: Color, foot: Color) -> Linear {
    Linear::new(rect.position(), Point::new(rect.x, rect.y + rect.height))
        .add_stop(0.0, top)
        .add_stop(1.0, foot)
}

fn rounded(rect: Rectangle, radius: f32) -> Path {
    Path::rounded_rectangle(rect.position(), rect.size(), radius.into())
}

fn black(opacity: f32) -> Color {
    Color {
        a: opacity,
        ..Color::BLACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The chassis sits 3 px inside its place, so the window's 12 px padding keeps 9 px of it
    // round the content and its shadow, 2 px right and down, stays in the place: in a place
    // 1050 × 524, from (3, 3) to (1047, 521).
    #[test]
    fn the_chassis_and_its_shadow_stay_in_its_place() {
        let place = Size::new(1050.0, 524.0);
        let chassis = chassis_rect(place);
        assert_eq!(
            (chassis.x, chassis.y, chassis.width, chassis.height),
            (3.0, 3.0, 1044.0, 518.0)
        );
        let shadow = chassis + Vector::new(DROP, DROP);
        assert!(shadow.x + shadow.width <= place.width);
        assert!(shadow.y + shadow.height <= place.height);
        let none = chassis_rect(Size::new(4.0, 0.0));
        assert_eq!((none.width, none.height), (0.0, 0.0), "no negative size");
    }
}
