"""Known answers for the bundled font and the widths the app measures in it.

Expected values come from the font file's own tables and these rules, never from app code:
  the font      crates/app/assets/fonts/fonts.lock pins each file the app builds in by its size
                and SHA-256, and the version string (name ID 5) both carry; a file that differs
                fails --check before any width is worked out
  a width       a text of ASCII characters is shaped without kerning (iced shapes ASCII text
                "basic": each glyph's advance, nothing between), so its width at size s px is
                s * (sum of the hmtx advance of each character's glyph) / unitsPerEm
  "Notes"       at 12 px: the known answer the app's measure, and a canvas text's, must give
  a readout     reserves the width of its widest value: its shape (the tempo "000.000", 10 to
                522 BPM with three decimals; the position "00:00:00", bar:step:tick) with every
                digit the widest digit of the font, at 16 px, plus 4 px either side, rounded up
                to a whole pixel; its digits right-aligned in it. Why not tabular figures: Inter's
                digits are proportional (its '1' is far narrower than its '4'), iced 0.14 cannot
                ask a font for tnum, and a changed copy of the font was ruled out (one more
                font to license, ship and lock, for looks alone)
  the signature the time signature's readout beside the tempo, the same rule at the type
                scale's numbers size, 11 px: "00/00" (up to 24 beats over 16) with the widest
                digit, plus 4 px either side, rounded up
  the column    beside Record: its widest word of "Notes", "Audio", "Capture" at 12 px (the type
                scale's label size; 11 px before) and 4 px either side, rounded up

  u1_font.json  the version, the advances used, and each width above

Run:
  python3 -I gen_u1_expected.py              print the known answers
  python3 -I gen_u1_expected.py --write DIR  also write u1_font.json into DIR
  python3 -I gen_u1_expected.py --check DIR  exit 1 unless the font files are the lock's and
                                             DIR's u1_font.json is what --write writes
"""
import hashlib, json, math, os, struct, sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FONTS = os.path.join(ROOT, "crates", "app", "assets", "fonts")
LOCK = os.path.join(FONTS, "fonts.lock")
REGULAR = "Inter-Regular.ttf"

READOUT_SIZE = 16
READOUT_PADDING = 4
TEMPO_SHAPE = "000.000"
POSITION_SHAPE = "00:00:00"
SIGNATURE_SHAPE = "00/00"
SIGNATURE_SIZE = 11
TAKE_TEXT = 12
TAKE_SIDE = 4
TAKE_WORDS = ("Notes", "Audio", "Capture")


def read_lock():
    """The lock's version string and its files, by name: (bytes, SHA-256)."""
    version, files = None, {}
    with open(LOCK, encoding="utf-8") as lock:
        for line in lock:
            line = line.rstrip("\n")
            if line.startswith("version="):
                version = line.split("=", 1)[1]
            elif line.startswith("file."):
                name, value = line[len("file."):].split("=", 1)
                _, size, digest = value.split(" ")
                files[name] = (int(size), digest)
    if version is None or not files:
        sys.exit(f"gen_u1: {LOCK} has no version= or no file. lines")
    return version, files


def tables(data):
    count = struct.unpack(">H", data[4:6])[0]
    found = {}
    for index in range(count):
        tag, _, offset, length = struct.unpack(">4sIII", data[12 + 16 * index:28 + 16 * index])
        found[tag.decode("latin-1")] = (offset, length)
    return found


def version_string(data):
    """The name table's version string (name ID 5, Windows Unicode, English US)."""
    offset = tables(data)["name"][0]
    _, count, strings = struct.unpack(">HHH", data[offset:offset + 6])
    for index in range(count):
        at = offset + 6 + 12 * index
        platform, encoding, language, name, length, start = struct.unpack(">HHHHHH", data[at:at + 12])
        if (platform, encoding, language, name) == (3, 1, 0x409, 5):
            text = offset + strings + start
            return data[text:text + length].decode("utf-16-be")
    sys.exit("gen_u1: the font has no version string")


def glyph_of(data, character):
    """The glyph a character maps to in the cmap's Windows Unicode BMP subtable (format 4)."""
    found = tables(data)
    cmap = found["cmap"][0]
    count = struct.unpack(">H", data[cmap + 2:cmap + 4])[0]
    for index in range(count):
        platform, encoding, offset = struct.unpack(">HHI", data[cmap + 4 + 8 * index:cmap + 12 + 8 * index])
        table = cmap + offset
        if (platform, encoding) == (3, 1) and struct.unpack(">H", data[table:table + 2])[0] == 4:
            break
    else:
        sys.exit("gen_u1: no format 4 Windows Unicode cmap")
    code = ord(character)
    pairs = struct.unpack(">H", data[table + 6:table + 8])[0] // 2
    ends = table + 14
    starts = ends + 2 * pairs + 2
    deltas = starts + 2 * pairs
    ranges = deltas + 2 * pairs
    for k in range(pairs):
        end = struct.unpack(">H", data[ends + 2 * k:ends + 2 * k + 2])[0]
        if code > end:
            continue
        start = struct.unpack(">H", data[starts + 2 * k:starts + 2 * k + 2])[0]
        if code < start:
            break
        delta = struct.unpack(">h", data[deltas + 2 * k:deltas + 2 * k + 2])[0]
        offset = struct.unpack(">H", data[ranges + 2 * k:ranges + 2 * k + 2])[0]
        if offset == 0:
            return (code + delta) & 0xFFFF
        at = ranges + 2 * k + offset + 2 * (code - start)
        glyph = struct.unpack(">H", data[at:at + 2])[0]
        return (glyph + delta) & 0xFFFF if glyph else 0
    sys.exit(f"gen_u1: the font maps no glyph to {character!r}")


def metrics(data):
    found = tables(data)
    head, hhea, hmtx = found["head"][0], found["hhea"][0], found["hmtx"][0]
    units = struct.unpack(">H", data[head + 18:head + 20])[0]
    long_metrics = struct.unpack(">H", data[hhea + 34:hhea + 36])[0]

    def advance(character):
        glyph = min(glyph_of(data, character), long_metrics - 1)
        return struct.unpack(">H", data[hmtx + 4 * glyph:hmtx + 4 * glyph + 2])[0]

    return units, advance


def answers():
    version, files = read_lock()
    for name, (size, digest) in sorted(files.items()):
        path = os.path.join(FONTS, name)
        with open(path, "rb") as font:
            data = font.read()
        if len(data) != size or hashlib.sha256(data).hexdigest() != digest:
            sys.exit(f"gen_u1: {name} is not the file fonts.lock pins")
        if version_string(data) != version:
            sys.exit(f"gen_u1: {name} says {version_string(data)!r}, fonts.lock {version!r}")
    with open(os.path.join(FONTS, REGULAR), "rb") as font:
        data = font.read()
    units, advance = metrics(data)

    def width(text, size):
        return size * sum(advance(character) for character in text) / units

    digits = "0123456789"
    # The first of the widest, should two be as wide.
    widest = max(digits, key=lambda digit: (advance(digit), -digits.index(digit)))

    def readout(shape, size=READOUT_SIZE):
        widest_form = "".join(widest if c.isdigit() else c for c in shape)
        return math.ceil(width(widest_form, size) + 2 * READOUT_PADDING)

    used = sorted(set(digits + ".:/" + "Notes" + "".join(TAKE_WORDS)))
    return {
        "font": REGULAR,
        "version": version,
        "units_per_em": units,
        "advances": {character: advance(character) for character in used},
        "notes_12px": width("Notes", 12),
        "widest_digit": widest,
        "readout": {
            "size": READOUT_SIZE,
            "padding": READOUT_PADDING,
            "tempo_shape": TEMPO_SHAPE,
            "tempo_width": readout(TEMPO_SHAPE),
            "signature_shape": SIGNATURE_SHAPE,
            "signature_size": SIGNATURE_SIZE,
            "signature_width": readout(SIGNATURE_SHAPE, SIGNATURE_SIZE),
            "position_shape": POSITION_SHAPE,
            "position_width": readout(POSITION_SHAPE),
        },
        "take_column_words": math.ceil(
            max(width(word, TAKE_TEXT) for word in TAKE_WORDS) + 2 * TAKE_SIDE
        ),
    }


def main():
    args = sys.argv[1:]
    text = json.dumps(answers(), indent=1, sort_keys=True) + "\n"
    if len(args) == 2 and args[0] in ("--write", "--check"):
        path = os.path.join(args[1], "u1_font.json")
        if args[0] == "--write":
            with open(path, "w", encoding="utf-8") as out:
                out.write(text)
            print(f"wrote {path}")
            return
        with open(path, encoding="utf-8") as committed:
            if committed.read() != text:
                sys.exit(f"gen_u1: {path} is not what --write writes")
        print("gen_u1: the fonts are fonts.lock's and u1_font.json matches")
        return
    if args:
        sys.exit(__doc__)
    print(text, end="")


if __name__ == "__main__":
    main()
