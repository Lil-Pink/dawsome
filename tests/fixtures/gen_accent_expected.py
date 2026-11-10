"""Known answers for the accent of the user's own: HSV and RGB, the colour wheel's geometry, the
contrast note's threshold and the slider fill's law.

Expected values come from these formulas, written here from their definitions, never from app
code:
  rgb -> hsv    each 8-bit channel over 255; v = max; s = (max - min) / max (0 when max is 0);
                h in degrees from the channel that is the max: red 60 * ((g - b) / d mod 6),
                green 60 * ((b - r) / d + 2), blue 60 * ((r - g) / d + 4), d = max - min,
                0 when d is 0 (a grey has no hue: 0 by convention). Checked against the standard
                library's colorsys.rgb_to_hsv, which gives the hue as a fraction of a turn.
  hsv -> rgb    c = v * s, x = c * (1 - |(h / 60) mod 2 - 1|), m = v - c; the sixth of the
                circle h lies in picks (c, x, 0), (x, c, 0), (0, c, x), (0, x, c), (x, 0, c) or
                (c, 0, x); each channel (that + m) * 255 rounded half up
  the wheel     160 px across, its ring 16 px thick, so the ring runs from radius 64 to 80
                and its middle, where the hue's dot sits, is radius 72; hue 0 (red) at the
                right, growing counter-clockwise as on a colour wheel drawn on paper:
                x = cx + 72 cos h, y = cy - 72 sin h, screen y growing down; the
                saturation/value square inscribed in the ring with 2 px to spare at the least,
                the largest side in steps of 4 that is, 84 px (its corners 42 * sqrt 2 = 59.40 px
                from the centre, under the ring's 64 less 2; 88 would reach 62.23),
                saturation along x from its left edge, value up from its foot:
                x = left + s * 84, y = top + (1 - v) * 84, a point outside held to the edge
  contrast      WCAG 2.2's ratio, as tests/fixtures/gen_ui_contrast.py works it out; the note
                shows under 3:1 on the theme's panel
  slider fill   an accent of the user's own at 55 % over the theme's bg, each 8-bit channel
                (55 a + 45 b + 50) // 100, rounded half up, the law every set's fill but
                Graphite's follows (their accents give their own fills back)
  the menu      over the wheel, what it sets: its rows "Accent" then "Piano roll notes"
  notes         the piano roll's notes' colour of the user's own: the note under the wheel shows
                when it reads under 3:1 on either of the roll's row tones, the white keys' and
                the black keys' (the lower ratio counts); every channel's own colour reads 3:1
                or more on both in every theme, so "Channel's own" never shows it; the roll's
                probe draws its notes in a yellow-green no channel has, #E8F04A

  accent_expected.json  the colours, the picks, the wheel, the probe's blue, the contrast pair,
                        the slider fills, the menu's rows and the notes' pair, probe and
                        channels

Run:
  python3 -I gen_accent_expected.py              print the known answers
  python3 -I gen_accent_expected.py --write DIR  also write accent_expected.json into DIR
  python3 -I gen_accent_expected.py --check DIR  exit 1 unless DIR's accent_expected.json is
                                                 what --write writes
"""
import colorsys, json, math, os, sys

FILE = "accent_expected.json"

# Hand-chosen: the three primaries, Graphite's own accent, a mid grey and black.
COLORS = ["FF0000", "00FF00", "0000FF", "F29A2E", "808080", "000000"]
# Picks made on the wheel, not from an 8-bit colour: the hue, saturation and value.
PICKS = [[199.0, 0.5, 0.5], [33.0, 1.0, 1.0], [300.0, 0.25, 0.9], [359.9, 1.0, 1.0],
         [120.0, 0.0, 0.73]]

WHEEL_SIZE = 160
RING = 16
SQUARE = 84
# The wheel's own corner is the origin of every position here.
CENTER = (WHEEL_SIZE / 2, WHEEL_SIZE / 2)
DOT_RADIUS = WHEEL_SIZE / 2 - RING / 2
HUES = [0.0, 45.0, 90.0, 180.0, 270.0]
# Points pressed on the wheel and the hue each gives: the ring's four quarters, half way out,
# and 51 px right of the centre and 51 up, 72.12 px out on the 45 degree line.
HUE_POINTS = [[152.0, 80.0], [80.0, 8.0], [8.0, 80.0], [80.0, 152.0], [131.0, 29.0]]
# Points in and beside the square (38..122 both ways) and the saturation and value each gives:
# its corners, its centre, a quarter in from its left and its foot, and the wheel's corners.
SQUARE_POINTS = [[38.0, 38.0], [122.0, 122.0], [80.0, 80.0], [59.0, 101.0], [0.0, 160.0],
                 [160.0, 0.0]]

# The main window's and the Settings window's probe draw with this blue (scripts/ui-check.sh
# passes it as --accent).
PROBE = "3FA7D6"

PANEL = {"graphite": "2B2E33"}
BG = {"graphite": "1F2125", "slate": "15171B", "gilt": "251812", "garnet": "26161D",
      "fjord": "092237"}
THEME_ACCENT = {"slate": "F29A2E", "gilt": "F4BC7F", "garnet": "F0B39B", "fjord": "95DDEF"}
LEAST = 3.0
# Hand-picked greys either side of 3:1 on Graphite's panel.
CONTRAST_PAIR = {"below": "767676", "above": "777777"}

# The menu over the wheel: what it sets, in its order.
TARGETS = ["Accent", "Piano roll notes"]
# The piano roll's row tones as the design gives them (white keys', black keys'), each theme.
ROLL_ROWS = {"graphite": ("34373D", "2D3035"), "slate": ("262A30", "1E2127"),
             "gilt": ("382A24", "30221D"), "garnet": ("3B222E", "321D27"),
             "fjord": ("123652", "0F2D46")}
# The channels' own colours, the same in every theme.
CHANNELS = ["E5533D", "4DA3E0", "6CC24A", "D9B23A", "B06AD4", "3FC1B0", "E07AA8", "9AA0A8"]
# Hand-picked greys either side of 3:1 on Graphite's lighter row (the white keys').
NOTE_PAIR = {"below": "7F7F7F", "above": "808080"}
# The roll's probe draws its notes in this (scripts/ui-check.sh passes it as --note-color).
NOTE_PROBE = "E8F04A"


def channels(hex_colour):
    return [int(hex_colour[i:i + 2], 16) for i in (0, 2, 4)]


def to_hsv(hex_colour):
    r, g, b = (value / 255 for value in channels(hex_colour))
    high, low = max(r, g, b), min(r, g, b)
    d = high - low
    if d == 0:
        hue = 0.0
    elif high == r:
        hue = 60 * (((g - b) / d) % 6)
    elif high == g:
        hue = 60 * ((b - r) / d + 2)
    else:
        hue = 60 * ((r - g) / d + 4)
    saturation = 0.0 if high == 0 else d / high
    # The standard library's answer, the hue there a fraction of a turn.
    h, s, v = colorsys.rgb_to_hsv(r, g, b)
    assert abs(h * 360 - hue) < 1e-9 and abs(s - saturation) < 1e-12 and abs(v - high) < 1e-12
    return [hue, saturation, high]


def to_rgb(hue, saturation, value):
    c = value * saturation
    x = c * (1 - abs((hue / 60) % 2 - 1))
    m = value - c
    sixth = int(hue // 60) % 6
    r, g, b = [(c, x, 0), (x, c, 0), (0, c, x), (0, x, c), (x, 0, c), (c, 0, x)][sixth]
    return "".join(f"{math.floor((part + m) * 255 + 0.5):02X}" for part in (r, g, b))


def dot(hue):
    angle = math.radians(hue)
    x = CENTER[0] + DOT_RADIUS * math.cos(angle)
    y = CENTER[1] - DOT_RADIUS * math.sin(angle)
    return {"hue": hue, "x": round(x, 6), "y": round(y, 6)}


def hue_at(point):
    x, y = point
    return math.degrees(math.atan2(CENTER[1] - y, x - CENTER[0])) % 360


def square():
    left = CENTER[0] - SQUARE / 2
    return left, CENTER[1] - SQUARE / 2


def sv_dot(saturation, value):
    left, top = square()
    return [left + saturation * SQUARE, top + (1 - value) * SQUARE]


def sv_at(point):
    left, top = square()
    clamp = lambda part: min(max(part, 0.0), 1.0)
    return [clamp((point[0] - left) / SQUARE), clamp(1 - (point[1] - top) / SQUARE)]


def luminance(hex_colour):
    def linear(value):
        c = value / 255
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
    r, g, b = channels(hex_colour)
    return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)


def ratio(a, b):
    la, lb = luminance(a), luminance(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


def on_rows(hex_colour, theme):
    """The lower ratio of `hex_colour` on `theme`'s two row tones."""
    return min(ratio(hex_colour, row) for row in ROLL_ROWS[theme])


def slider_fill(accent, bg):
    return "".join(f"{(55 * a + 45 * b + 50) // 100:02X}"
                   for a, b in zip(channels(accent), channels(bg)))


def answers():
    colors = []
    for hex_colour in COLORS + [PROBE]:
        hsv = to_hsv(hex_colour)
        back = to_rgb(*hsv)
        assert back == hex_colour, (hex_colour, back)
        colors.append({"hex": hex_colour, "hsv": [round(part, 9) for part in hsv],
                       "rgb": back, "square": [round(part, 6) for part in sv_dot(*hsv[1:])]})
    picks = [{"hsv": pick, "rgb": to_rgb(*pick)} for pick in PICKS]
    probe_hue = to_hsv(PROBE)[0]
    hues = HUES + [color["hsv"][0] for color in colors]
    dots = [dot(hue) for index, hue in enumerate(hues) if hue not in hues[:index]]
    contrast = {side: {"hex": hex_colour,
                       "ratio": round(ratio(hex_colour, PANEL["graphite"]), 4)}
                for side, hex_colour in CONTRAST_PAIR.items()}
    assert contrast["below"]["ratio"] < LEAST < contrast["above"]["ratio"]
    fills = {
        "probe": {theme: slider_fill(PROBE, BG[theme]) for theme in ("graphite", "gilt")},
        "own": {theme: slider_fill(accent, BG[theme]) for theme, accent in THEME_ACCENT.items()},
    }
    note_pair = {side: {"hex": hex_colour, "ratio": round(on_rows(hex_colour, "graphite"), 4)}
                 for side, hex_colour in NOTE_PAIR.items()}
    assert note_pair["below"]["ratio"] < LEAST < note_pair["above"]["ratio"]
    channels_least = {theme: round(min(on_rows(channel, theme) for channel in CHANNELS), 4)
                      for theme in ROLL_ROWS}
    assert all(least >= LEAST for least in channels_least.values())
    note = {
        "rows": {theme: {"white": white, "black": black}
                 for theme, (white, black) in ROLL_ROWS.items()},
        "contrast": {"least": LEAST, **note_pair},
        "channels_least": channels_least,
        "probe": {"hex": NOTE_PROBE,
                  "ratio": round(on_rows(NOTE_PROBE, "graphite"), 4)},
    }
    assert note["probe"]["ratio"] >= LEAST and NOTE_PROBE not in CHANNELS
    return {
        "targets": TARGETS,
        "note": note,
        "colors": colors,
        "picks": picks,
        "wheel": {
            "size": WHEEL_SIZE, "ring": RING, "square": SQUARE, "center": list(CENTER),
            "dot_radius": DOT_RADIUS, "dots": dots,
            "hue_at": [{"at": point, "hue": round(hue_at(point), 6)} for point in HUE_POINTS],
            "sv_at": [{"at": point, "sv": [round(part, 6) for part in sv_at(point)]}
                      for point in SQUARE_POINTS],
        },
        "probe": {"hex": PROBE, "hue": round(probe_hue, 6),
                  "offset": [round(DOT_RADIUS * math.cos(math.radians(probe_hue)), 6),
                             round(-DOT_RADIUS * math.sin(math.radians(probe_hue)), 6)]},
        "contrast": {"panel": PANEL["graphite"], "least": LEAST, **contrast},
        "slider_fill": fills,
    }


def main():
    args = sys.argv[1:]
    text = json.dumps(answers(), indent=1) + "\n"
    if len(args) == 2 and args[0] in ("--write", "--check"):
        path = os.path.join(args[1], FILE)
        if args[0] == "--write":
            with open(path, "w", encoding="utf-8") as out:
                out.write(text)
            print(f"wrote {path}")
            return
        try:
            with open(path, encoding="utf-8") as committed:
                same = committed.read() == text
        except FileNotFoundError:
            same = False
        if not same:
            sys.exit(f"gen_accent_expected: {path} is not what --write writes")
        print(f"gen_accent_expected: {FILE} matches")
        return
    if args:
        sys.exit(__doc__)
    print(text, end="")


if __name__ == "__main__":
    main()
