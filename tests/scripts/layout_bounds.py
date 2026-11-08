"""Checks a window's layout as the app dumps it (`dawsome-app --dump-layout <file>`, written as
its screenshot is taken; crates/app/src/app/layout_dump.rs says what it holds and what it cannot
see): its text runs and its boxes, in the window's logical pixels. Why a check of the layout: at
a zoom other than 100 % no pixel is pinned (scripts/ui-check.sh), and these faults are faults at
any zoom.

It fails on:
  - text cut short: a text run whose measured width (its paragraph's widest line) is wider than
    its box (its laid-out bounds) by more than SLACK;
  - two text runs overlapping: two runs whose boxes share more than SLACK across and down (text
    drawn over text; the toolkit reports no widget's kind, so boxes may overlap, a stack's
    layers do by design, but no two runs of words may);
  - a box or a text run outside the window: reaching past the window's edges by more than SLACK,
    unless it lies in a scrollable, which may hold it past the window to scroll it into view.
Each fault names what it found: the text and its widths, both runs, the box or run and the
window.

Run:
  python3 -I layout_bounds.py DUMP...   check each dump; print one line a dump, and each fault;
                                        exit 1 when any fault was found
  python3 -I layout_bounds.py --self-test
                                        the check finds the faults planted in a dump of its own
                                        (one overlap and one cut label, both named, and a box
                                        past the window's edge) and passes the clean one; exit 1
                                        if not
"""
import json, sys

# Measured widths are sums of glyph advances in floats, and laid-out boxes are made from them:
# a hundredth of a pixel is no cut, a hundredth more is.
SLACK = 0.01


def shared(a, b):
    """How far the rects `a` and `b` share across and down (negative when apart)."""
    across = min(a["x"] + a["width"], b["x"] + b["width"]) - max(a["x"], b["x"])
    down = min(a["y"] + a["height"], b["y"] + b["height"]) - max(a["y"], b["y"])
    return across, down


def where(item):
    return (f'at {item["path"]} ({item["x"]:g}, {item["y"]:g}, '
            f'{item["width"]:g} x {item["height"]:g})')


def faults(dump):
    """Every fault of `dump`, a line each."""
    found = []
    texts = dump["texts"]
    for run in texts:
        if run["measured"] > run["width"] + SLACK:
            found.append(f'text cut short: "{run["text"]}" measures {run["measured"]:g} px in a '
                         f'box {run["width"]:g} px wide, {where(run)}')
    for index, a in enumerate(texts):
        for b in texts[index + 1:]:
            across, down = shared(a, b)
            if across > SLACK and down > SLACK:
                found.append(f'text over text by {across:g} x {down:g} px: "{a["text"]}" '
                             f'{where(a)} and "{b["text"]}" {where(b)}')
    window = dump["window"]
    for what, items in (("box", dump["boxes"]), ("text", texts)):
        for item in items:
            if item.get("scrolled"):
                continue
            right, bottom = item["x"] + item["width"], item["y"] + item["height"]
            if (item["x"] < -SLACK or item["y"] < -SLACK
                    or right > window["width"] + SLACK or bottom > window["height"] + SLACK):
                name = f' "{item["text"]}"' if what == "text" else ""
                found.append(f'{what}{name} outside the window ({window["width"]:g} x '
                             f'{window["height"]:g}): {where(item)}')
    return found


def check(path):
    with open(path, encoding="utf-8") as file:
        dump = json.load(file)
    found = faults(dump)
    window = dump["window"]
    print(f'layout_bounds: {path}: {window["width"]:g} x {window["height"]:g} at '
          f'{dump.get("zoom", 100)} %, {len(dump["texts"])} text runs, {len(dump["boxes"])} '
          f'boxes: {"ok" if not found else f"{len(found)} faults"}')
    for fault in found:
        print(f"  {fault}")
    return not found


def box(path, x, y, width, height, scrolled=False):
    return {"path": path, "x": x, "y": y, "width": width, "height": height, "scrolled": scrolled}


def text(path, words, x, y, width, height, measured, scrolled=False):
    return {**box(path, x, y, width, height, scrolled), "text": words, "measured": measured}


# A window of 400 x 100: a bar of two buttons side by side, File and Edit, their labels fitting
# their boxes to a hair (a sum of floats can come out that close); a stack's two layers
# overlapping (boxes may); and a list in a scrollable reaching past the window's foot (allowed).
CLEAN = {
    "window": {"width": 400.0, "height": 100.0},
    "zoom": 125,
    "texts": [
        text("0.0.0", "File", 8, 9, 26.5, 18, 26.5),
        text("0.1.0", "Edit", 40, 9, 27.25, 18, 27.255),
        text("2.0.3.1", "Kick 909", 4, 140, 60, 18, 52, scrolled=True),
    ],
    "boxes": [
        box("0", 0, 0, 400, 64),
        box("1", 0, 0, 400, 64),
        box("0.0", 4, 4, 34.5, 46),
        box("0.1", 38.5, 4, 35.25, 46),
        box("2.0", 0, 60, 400, 180, scrolled=True),
    ],
}


def self_test():
    failures = []
    if faults(CLEAN):
        failures.append(f"the clean dump was refused: {faults(CLEAN)}")
    planted = json.loads(json.dumps(CLEAN))
    # Edit's label moved left over File's, and cut to a box narrower than it.
    planted["texts"][1]["x"] = 30.0
    planted["texts"][1]["width"] = 20.0
    found = faults(planted)
    wanted = [
        ("the overlap, both runs named", lambda line: "text over text" in line
         and '"File"' in line and '"Edit"' in line),
        ("the cut label, named", lambda line: 'text cut short: "Edit"' in line),
    ]
    for what, matches in wanted:
        if not any(matches(line) for line in found):
            failures.append(f"{what} was not found (got {found})")
    if len(found) != 2:
        failures.append(f"two faults were planted, {len(found)} found: {found}")
    outside = json.loads(json.dumps(CLEAN))
    outside["boxes"][3]["width"] = 400.0
    found = faults(outside)
    if not any("box outside the window" in line and "at 0.1 " in line for line in found):
        failures.append(f"a box past the window's edge was not found (got {found})")
    if failures:
        print("layout_bounds: self-test failed:")
        for failure in failures:
            print(f"  {failure}")
        return False
    print("layout_bounds: self-test: the overlap, the cut label and the box past the edge are "
          "found and named, the clean dump passes")
    return True


def main():
    args = sys.argv[1:]
    if args == ["--self-test"]:
        sys.exit(0 if self_test() else 1)
    if not args or any(arg.startswith("-") for arg in args):
        sys.exit(__doc__)
    results = [check(path) for path in args]
    sys.exit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
