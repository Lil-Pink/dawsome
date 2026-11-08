"""Known answers for the colour themes: the contrast of each theme's colours, rule by rule.

Expected values come from these inputs and the WCAG formula, never from app code:
  the colours   each theme's colours the rules name, as the design's table gives them (Graphite's
                being the constants the app drew with before themes), and the content colours,
                which are the same in every theme; the app's palettes are held to these values
  luminance     WCAG 2.2's relative luminance: each sRGB channel c = v / 255 linearised as
                c / 12.92 when c <= 0.04045, else ((c + 0.055) / 1.055) ** 2.4, then
                L = 0.2126 R + 0.7152 G + 0.0722 B (the older 0.03928 threshold gives the same
                linear value for every 8-bit channel: none lies between the two)
  a ratio       (L1 + 0.05) / (L2 + 0.05), the lighter's over the darker's, kept to 2 decimals
  text tiers    the design's table gives Slate's text, its secondary text (a label of live
                information) and its disabled text (what cannot be used now): DCE0E5, ACB2BA,
                6B7179. Its secondary keeps a share of the text's CIE L* and its disabled a share
                of the secondary's; a theme the design gives no secondary text (Graphite) takes
                the first share of its text's L*, a theme it gives none disabled (Gilt, Garnet,
                Fjord) the second of its secondary's, a* and b* kept (CIE L*a*b* as for the white
                keys below). Graphite's disabled is the dim text it always drew, as given.
  unlit steps   the rack's two unlit step tones a shade down (for the 3:1 rule): Graphite's pair
                4A4F57 / 3A3E45 to 34383E / 2A2E34 sets the share of L* each tone keeps; every
                other theme's tones before take those shares, a* and b* kept. Slate's were
                designed so, and Graphite's pair stands as given (the law gives
                333840 / 2A2E35).
  the rules     text >= 7 and text_secondary >= 4.5 on bg, panel and raised; text >= 4.5 on
                hover and active; accent >= 4.5 on bg, panel and raised; each content colour >= 3
                against raised (the lanes, the rack rows); each channel colour >= 3 against both
                unlit step tones (a lit step reads apart from an unlit one); line_strong >= 2.5
                on panel; line_soft from 1.1 to 1.6 on raised (decorative); audio_clip_body
                >= 1.5 on raised (an audio clip's body reads apart from the lane it lies on)
  failing       the rules a theme's ratios miss, in the rules' order
  ceiling       for a content colour that misses its rule, the most luminance a darker raised
                could have and pass, (L + 0.05) / 3 - 0.05; below 0 no dark surface passes
  white keys    a shade down in every theme: the design's Graphite pair, CFD3D8 to B4B9BF,
                sets the share of CIE L* a white key keeps; every other theme's white as
                designed takes that share of its L*, its a* and b* kept. CIE L*a*b* from linear
                sRGB (as above) by IEC 61966-2-1's matrix to XYZ and its inverse back, D65 white
                the matrix's row sums: f(t) = t^(1/3) above (6/29)^3, else t / (3 (6/29)^2) +
                4/29; L* = 116 f(Y) - 16, a* = 500 (f(X/Xn) - f(Y)), b* = 200 (f(Y) - f(Z/Zn));
                back to 8 bits by the sRGB curve (12.92 c at or below 0.0031308, else
                1.055 c^(1/2.4) - 0.055), each channel held to 0..1 and rounded half up. The
                pair stands as given (the law gives B5B9BE for CFD3D8), and so for Slate, whose
                white was Graphite's.
  audio body    an audio clip's body, a token of each theme: AUDIO_BODY (crates/app/src/ui/
                style.rs, 23332C), which every theme drew, in Graphite, Slate, Garnet and Fjord
                (each reads it apart from its lanes by hue, not luminance: their rule fails, as
                listed); a theme of [AUDIO_LIFTED], whose lanes are AUDIO_BODY's own green (Gilt:
                1.06:1 on its raised band), takes AUDIO_BODY with its L* raised to the least whole
                L* at which it reads 1.5:1 on the theme's raised band, its a* and b* kept (CIE
                L*a*b* as for the white keys above)

  ui_contrast_expected.json  the rules, the colours, each theme's ratios and failing rules, the
                             text tiers, the unlit steps, the white keys and the audio body

Run:
  python3 -I gen_ui_contrast.py              print the known answers
  python3 -I gen_ui_contrast.py --write DIR  also write ui_contrast_expected.json into DIR
  python3 -I gen_ui_contrast.py --check DIR  exit 1 unless DIR's ui_contrast_expected.json is
                                             what --write writes
"""
import json, os, sys

FILE = "ui_contrast_expected.json"

# The design's table, the columns the rules name (Graphite's the constants the app drew with
# before themes). The text tiers a theme lacks here and every theme's unlit steps are worked out
# below ([text_tiers], [unlit_steps]).
THEMES = {
    "graphite": {
        "bg": "1F2125", "panel": "2B2E33", "raised": "34373D", "hover": "464B53",
        "active": "3D4148", "text": "C9CDD3", "text_disabled": "6E747C", "accent": "F29A2E",
        "line_strong": "5C626B", "line_soft": "3E4248",
    },
    "slate": {
        "bg": "15171B", "panel": "1C1F24", "raised": "262A30", "hover": "30353C",
        "active": "3A4048", "text": "DCE0E5", "text_secondary": "ACB2BA",
        "text_disabled": "6B7179", "accent": "F29A2E", "line_strong": "7A818B",
        "line_soft": "2F343B",
    },
    "gilt": {
        "bg": "15241C", "panel": "1B2D23", "raised": "23382C", "hover": "2E4738",
        "active": "395545", "text": "E9E2C8", "text_secondary": "9FA98F", "accent": "F2BE4C",
        "line_strong": "8C7230", "line_soft": "2A3F33",
    },
    "garnet": {
        "bg": "26161D", "panel": "301B25", "raised": "3B222E", "hover": "492E3A",
        "active": "583947", "text": "F3DEDB", "text_secondary": "BB9FA8", "accent": "F0B39B",
        "line_strong": "9F6A5A", "line_soft": "422A35",
    },
    "fjord": {
        "bg": "092237", "panel": "0E2B43", "raised": "123652", "hover": "1E4465",
        "active": "2C5275", "text": "D9E4EA", "text_secondary": "96A8B6", "accent": "95DDEF",
        "line_strong": "578296", "line_soft": "1E3D58",
    },
}

# Slate's text, secondary text and disabled text as the design's table gives them: the shares of L*
# the lower two keep.
TEXT_TIERS = ("DCE0E5", "ACB2BA", "6B7179")

# Graphite's unlit step tones before and after the shade down, which set the shares of L* each keeps.
UNLIT_SHADE = (("4A4F57", "3A3E45"), ("34383E", "2A2E34"))
# Each theme's unlit step tones before the shade down (Gilt's, Garnet's and Fjord's: the active band 2
# lighter a channel, then the hover band); Slate's were designed darker, so it has none.
UNLIT_BEFORE = {"graphite": ("4A4F57", "3A3E45"), "gilt": ("3B5747", "2E4738"),
                "garnet": ("5A3B49", "492E3A"), "fjord": ("2E5477", "1E4465")}
UNLIT_DESIGNED = {"slate": ("34383E", "2A2E34")}

# The design's white key a shade down: Graphite's before and after.
KEY_SHADE = ("CFD3D8", "B4B9BF")
# Each theme's white key as designed, before the shade.
KEY_WHITES = {"graphite": "CFD3D8", "slate": "CFD3D8", "gilt": "E8DCB4", "garnet": "E8D6D2",
              "fjord": "D1DCE2"}

# The audio clip's body every theme drew (crates/app/src/ui/style.rs's AUDIO_BODY), the least
# ratio it is held to on a theme's raised band, and the themes whose lanes all but swallow it,
# which lift its L* to that ratio.
AUDIO_BODY = "23332C"
AUDIO_LEAST = 1.5
AUDIO_LIFTED = ("gilt",)

# The content colours, the same in every theme: the eight channel colours by rack order, then
# the clips', the LEDs' and record's (crates/app/src/ui/style.rs's constants of those names).
CONTENT = {
    "channel_1": "E5533D", "channel_2": "4DA3E0", "channel_3": "6CC24A", "channel_4": "D9B23A",
    "channel_5": "B06AD4", "channel_6": "3FC1B0", "channel_7": "E07AA8", "channel_8": "9AA0A8",
    "CLIP_BODY": "5B7FA6", "AUDIO_TITLE": "3E6B57", "AUTOMATION_TITLE": "6B4A7A",
    "LED_ON": "6CD84A", "RECORD_ARMED": "E5533D",
}


def rules():
    """Each rule as (id, colour, ground, least, most): the ratio of colour on ground must lie
    from least to most (most None: no upper bound)."""
    listed = []
    for ground in ("bg", "panel", "raised"):
        listed.append(("text", ground, 7.0, None))
        listed.append(("text_secondary", ground, 4.5, None))
    for ground in ("hover", "active"):
        listed.append(("text", ground, 4.5, None))
    for ground in ("bg", "panel", "raised"):
        listed.append(("accent", ground, 4.5, None))
    for name in CONTENT:
        listed.append((name, "raised", 3.0, None))
    for name in CONTENT:
        if name.startswith("channel_"):
            for tone in ("unlit_0", "unlit_1"):
                listed.append((name, tone, 3.0, None))
    listed.append(("line_strong", "panel", 2.5, None))
    listed.append(("line_soft", "raised", 1.1, 1.6))
    listed.append(("audio_clip_body", "raised", AUDIO_LEAST, None))
    return [(f"{colour}/{ground}", colour, ground, least, most)
            for colour, ground, least, most in listed]


def linear_rgb(hex_colour):
    def linear(value):
        c = value / 255
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
    return [linear(int(hex_colour[i:i + 2], 16)) for i in (0, 2, 4)]


def luminance(hex_colour):
    r, g, b = linear_rgb(hex_colour)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


# IEC 61966-2-1's linear sRGB to XYZ; its middle row is the luminance's weights.
TO_XYZ = ((0.4124, 0.3576, 0.1805), (0.2126, 0.7152, 0.0722), (0.0193, 0.1192, 0.9505))
WHITE = tuple(sum(row) for row in TO_XYZ)


def times(matrix, vector):
    return [sum(m * v for m, v in zip(row, vector)) for row in matrix]


def inverse(m):
    (a, b, c), (d, e, f), (g, h, i) = m
    det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    return ((( e * i - f * h) / det, -(b * i - c * h) / det, ( b * f - c * e) / det),
            (-(d * i - f * g) / det, ( a * i - c * g) / det, -(a * f - c * d) / det),
            (( d * h - e * g) / det, -(a * h - b * g) / det, ( a * e - b * d) / det))


FROM_XYZ = inverse(TO_XYZ)
EDGE = 6 / 29


def to_lab(hex_colour):
    def f(t):
        return t ** (1 / 3) if t > EDGE ** 3 else t / (3 * EDGE ** 2) + 4 / 29
    fx, fy, fz = (f(v / w) for v, w in zip(times(TO_XYZ, linear_rgb(hex_colour)), WHITE))
    return 116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)


def from_lab(lightness, a, b):
    def f_inverse(t):
        return t ** 3 if t > EDGE else 3 * EDGE ** 2 * (t - 4 / 29)
    fy = (lightness + 16) / 116
    xyz = [f_inverse(t) * w for t, w in zip((fy + a / 500, fy, fy - b / 200), WHITE)]
    def encode(c):
        c = min(max(c, 0.0), 1.0)
        v = 12.92 * c if c <= 0.0031308 else 1.055 * c ** (1 / 2.4) - 0.055
        return int(v * 255 + 0.5)
    return "".join(f"{encode(c):02X}" for c in times(FROM_XYZ, xyz))


def white_keys():
    """Each theme's white key as designed and a shade down: the design's pair as given, every
    other white its L* times the pair's share, its a* and b* kept."""
    before, after = KEY_SHADE
    share = to_lab(after)[0] / to_lab(before)[0]
    keys = {}
    for theme, designed in KEY_WHITES.items():
        lightness, a, b = to_lab(designed)
        shaded = after if designed == before else from_lab(lightness * share, a, b)
        keys[theme] = {"designed": designed, "shaded": shaded}
    return {"from": before, "to": after, "share": round(share, 6), "themes": keys}


def shade(hex_colour, share):
    """hex_colour with its L* times share, its a* and b* kept."""
    lightness, a, b = to_lab(hex_colour)
    return from_lab(lightness * share, a, b)


def l_share(pair):
    """The share of L* the second colour of pair keeps of the first's."""
    return to_lab(pair[1])[0] / to_lab(pair[0])[0]


def text_tiers():
    """Each theme's secondary and disabled text: as designed, else by the shares of Slate's
    tiers ([TEXT_TIERS]), the secondary of the text, the disabled of the secondary."""
    text, secondary, disabled = TEXT_TIERS
    shares = (l_share((text, secondary)), l_share((secondary, disabled)))
    themes = {}
    for theme, colours in THEMES.items():
        shown = colours.get("text_secondary") or shade(colours["text"], shares[0])
        dim = colours.get("text_disabled") or shade(shown, shares[1])
        derived = [name for name in ("text_secondary", "text_disabled") if name not in colours]
        themes[theme] = {"text_secondary": shown, "text_disabled": dim, "derived": derived}
    return {"from": list(TEXT_TIERS), "shares": [round(share, 6) for share in shares],
            "themes": themes}


def unlit_steps():
    """Each theme's two unlit step tones: as designed or given, else its tones before the shade down
    with Graphite's pair's shares of L* ([UNLIT_SHADE])."""
    before, after = UNLIT_SHADE
    shares = [l_share(pair) for pair in zip(before, after)]
    themes = {}
    for theme in THEMES:
        if theme in UNLIT_DESIGNED:
            themes[theme] = {"before": None, "after": list(UNLIT_DESIGNED[theme])}
            continue
        old = UNLIT_BEFORE[theme]
        new = after if old == before else tuple(map(shade, old, shares))
        themes[theme] = {"before": list(old), "after": list(new)}
    return {"from": list(before), "to": list(after), "shares": [round(share, 6) for share in shares],
            "themes": themes}


def ratio(a, b):
    la, lb = luminance(a), luminance(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


def audio_bodies():
    """Each theme's audio clip body: AUDIO_BODY, or for a theme of AUDIO_LIFTED AUDIO_BODY with its
    L* raised to the least whole L* that reads AUDIO_LEAST on the theme's raised band."""
    lightness, a, b = to_lab(AUDIO_BODY)
    themes = {}
    for theme, colours in THEMES.items():
        if theme not in AUDIO_LIFTED:
            themes[theme] = {"body": AUDIO_BODY, "lightness": None}
            continue
        whole = int(lightness) + 1
        while ratio(from_lab(whole, a, b), colours["raised"]) < AUDIO_LEAST:
            whole += 1
        themes[theme] = {"body": from_lab(whole, a, b), "lightness": whole}
    return {"from": AUDIO_BODY, "from_lightness": round(lightness, 4), "least": AUDIO_LEAST,
            "themes": themes}


def answers():
    listed = rules()
    tiers, steps, bodies = text_tiers(), unlit_steps(), audio_bodies()
    themes = {}
    for theme, designed in THEMES.items():
        tier = tiers["themes"][theme]
        unlit = steps["themes"][theme]["after"]
        colours = {**designed, "text_secondary": tier["text_secondary"],
                   "text_disabled": tier["text_disabled"], "unlit_0": unlit[0],
                   "unlit_1": unlit[1], "audio_clip_body": bodies["themes"][theme]["body"]}
        named = {**colours, **CONTENT}
        ratios, failing = {}, []
        for rule, colour, ground, least, most in listed:
            value = ratio(named[colour], named[ground])
            ratios[rule] = round(value, 2)
            if value < least or (most is not None and value > most):
                failing.append(rule)
        themes[theme] = {"colors": colours, "ratios": ratios, "failing": failing}
    ceilings = {
        name: round((luminance(hex_colour) + 0.05) / 3 - 0.05, 4)
        for name, hex_colour in CONTENT.items()
        if any(f"{name}/raised" in theme["failing"] for theme in themes.values())
    }
    return {
        "rules": [{"id": rule, "color": colour, "on": ground, "least": least, "most": most}
                  for rule, colour, ground, least, most in listed],
        "content": CONTENT,
        "themes": themes,
        "ceiling": ceilings,
        "text_tiers": tiers,
        "unlit_steps": steps,
        "white_keys": white_keys(),
        "audio_body": bodies,
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
            sys.exit(f"gen_ui_contrast: {path} is not what --write writes")
        print(f"gen_ui_contrast: {FILE} matches")
        return
    if args:
        sys.exit(__doc__)
    print(text, end="")


if __name__ == "__main__":
    main()
