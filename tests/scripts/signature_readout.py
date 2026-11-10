"""Checks that the toolbar's time signature reads as it should in a window's layout dump
(`dawsome-app --dump-layout <file>`, the format tests/scripts/layout_bounds.py reads): exactly one
text run says the signature (such as "4/4"), its readout's box ending where the tempo's box ends,
top to bottom (the readout's bottom on the tempo's, so the two read as one row), and right of it
(it starts at or past where the tempo's run ends). The tempo's run is the leftmost run that reads
as a tempo, digits, a point and digits ("120.0"); a run's box is the smallest of the dump's boxes
holding it (the readout's glass).

Run:
  python3 -I signature_readout.py DUMP SIGNATURE   exit 1 unless the dump holds the signature
                                                   where the toolbar puts it
  python3 -I signature_readout.py --self-test      the check passes a dump of its own and
                                                   refuses the faults planted in copies of it
                                                   (no signature, two, one left of the tempo,
                                                   one on another row, one centred on the
                                                   tempo's box); exit 1 if not
"""
import json, re, sys

TEMPO = re.compile(r"^\d+\.\d+$")
# Laid-out bounds are floats: a hundredth of a pixel is the same edge (layout_bounds.py's SLACK).
SLACK = 0.01


def holder(dump, run):
    """The smallest box of `dump` holding `run`, or None."""
    holding = [box for box in dump["boxes"]
               if box["x"] <= run["x"] + SLACK and box["y"] <= run["y"] + SLACK
               and run["x"] + run["width"] <= box["x"] + box["width"] + SLACK
               and run["y"] + run["height"] <= box["y"] + box["height"] + SLACK]
    return min(holding, key=lambda box: box["width"] * box["height"], default=None)


def problem(dump, signature):
    """Why `dump` does not show `signature` beside the tempo, or None when it does."""
    texts = dump["texts"]
    tempos = sorted((run for run in texts if TEMPO.match(run["text"])), key=lambda run: run["x"])
    if not tempos:
        return "no tempo readout in the dump"
    tempo = tempos[0]
    found = [run for run in texts if run["text"] == signature]
    if len(found) != 1:
        return "%d text runs read %r, not one" % (len(found), signature)
    run = found[0]
    tempo_box, readout = holder(dump, tempo), holder(dump, run)
    if tempo_box is None or readout is None:
        return "no box holds the %s run" % ("tempo's" if tempo_box is None else repr(signature))
    bottom = readout["y"] + readout["height"]
    tempo_bottom = tempo_box["y"] + tempo_box["height"]
    if abs(bottom - tempo_bottom) > SLACK:
        return "%r's readout ends at y %s, not at the tempo's bottom (%s)" % (
            signature, bottom, tempo_bottom)
    if run["x"] < tempo["x"] + tempo["width"]:
        return "%r starts at x %s, before the tempo's run ends (%s)" % (
            signature, run["x"], tempo["x"] + tempo["width"])
    return None


def run(text, x, y, width=40.0, height=20.0):
    return {"text": text, "x": x, "y": y, "width": width, "height": height}


def self_test():
    # The toolbar's tempo group as the design lays it out in the 64 px bar: the 75 x 46 tempo
    # box at y 9 (its 16 px digits' run 20.8 tall, centred: y 21.6), then 2 px on the 41 x 24
    # signature box at 9 + 46 - 24 = y 31 (its 11 px digits' run 14.3 tall, centred: y 35.85),
    # and the group's row holding both; the position's readout further right.
    tempo, sig, position = (run("120.0", 190.0, 21.6, 40.0, 20.8), run("4/4", 268.0, 35.85, 20.0, 14.3),
                            run("1:01:00", 600.0, 21.6, 60.0, 20.8))
    boxes = [run("", 186.0, 9.0, 75.0, 46.0), run("", 263.0, 31.0, 41.0, 24.0),
             run("", 186.0, 9.0, 118.0, 46.0), run("", 590.0, 9.0, 80.0, 46.0)]
    clean = {"texts": [tempo, sig, position], "boxes": boxes}

    def fault(texts, readout=None):
        return {"texts": texts, "boxes": boxes[:1] + [readout or boxes[1]] + boxes[2:]}

    faults = {
        "none": fault([tempo, position]),
        "two": {"texts": clean["texts"] + [run("4/4", 700.0, 35.85, 20.0, 14.3)],
                "boxes": boxes + [run("", 690.0, 31.0, 41.0, 24.0)]},
        "left": fault([tempo, run("4/4", 125.0, 35.85, 20.0, 14.3), position],
                      run("", 120.0, 31.0, 41.0, 24.0)),
        "row": fault([tempo, run("4/4", 268.0, 304.85, 20.0, 14.3), position],
                     run("", 263.0, 300.0, 41.0, 24.0)),
        # The box centred on the tempo's row, not on its bottom: 9 + (46 - 24) / 2 = y 20.
        "centred": fault([tempo, run("4/4", 268.0, 24.85, 20.0, 14.3), position],
                         run("", 263.0, 20.0, 41.0, 24.0)),
    }
    failed = []
    if problem(clean, "4/4") is not None:
        failed.append("the clean dump: " + problem(clean, "4/4"))
    for name, dump in faults.items():
        if problem(dump, "4/4") is None:
            failed.append("the %r fault passed" % name)
    for line in failed:
        print("signature_readout self-test:", line)
    if failed:
        sys.exit(1)
    print("signature_readout self-test: the clean dump passes and the %d faults are found"
          % len(faults))


def main():
    args = sys.argv[1:]
    if args == ["--self-test"]:
        self_test()
        return
    if len(args) != 2:
        sys.exit(__doc__)
    with open(args[0], encoding="utf-8") as f:
        dump = json.load(f)
    reason = problem(dump, args[1])
    if reason:
        sys.exit("signature_readout: %s: %s" % (args[0], reason))
    print("signature_readout: %s: %r stands right of the tempo, its bottom on the tempo's"
          % (args[0], args[1]))


if __name__ == "__main__":
    main()
