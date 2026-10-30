#!/usr/bin/env python3
"""The Help window's pictures (help guide H4): screenshots of the real app, cut to the part a
section talks about, with a ring or an arrow in the record red around the thing it names.

usage: python3 -I scripts/help_shots.py make --app <dawsome-app> --work <dir> --instruments <dir>
       python3 -I scripts/help_shots.py check
       (from the repository root; scripts/help-shots.sh builds the app and runs `make`, and
       scripts/check.sh runs `check`)

assets/help/source/shots.json is the list: each shot (a project of assets/help/source, the
window's flags, a settings file to start from, the MIDI ports it lists) and each picture (the
shot it is cut from, the part, its width in the guide, and its rings and arrows, all in the
window's logical pixels).

`make` takes each shot in turn, one window at a time, with per-user folders of its own under
--work (the starter kit's drums copied into its data folder) and --instruments as its installed
sets (DAWSOME_SHIPPED_INSTRUMENTS: the sets the programs ship, the kit's and assets/instruments,
as scripts/stage-instruments.sh puts them together), so the projects' `dawsome:` sounds load and
the Library lists what an install lists; DAWSOME_APP_ID=dawsome-test, playing through no device
(--null-output) and listing only the MIDI ports the shot names (--midi-ports, never connected):
nothing reads or writes the user's settings, and no audio or MIDI device is opened. Each picture
is cut from its shot (a shot taken at a scale factor other than 1 is first scaled back to
logical pixels), scaled to its width, its rings and arrows drawn over it (Pillow; drawn four
times larger and scaled down, so their edges are smooth), and written to assets/help/<file>.
Last it writes assets/help/source/shots.lock: the SHA-256 of every source (shots.json, the
projects it names, this script) and of every picture.

`check` (standard library only) fails when a source or a picture differs from the lock (a
source changed without a rerun, or a picture changed by hand), when a picture the lock names
is missing or a PNG in assets/help is not in it, or when the guide (assets/help/guide.md) names
a picture the lock does not. Why a lock and not the pictures made again: a check cannot open a
window, and the pictures are meant to change only when someone looks at them.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

HELP = Path("assets/help")
SOURCE = HELP / "source"
SHOTS = SOURCE / "shots.json"
LOCK = SOURCE / "shots.lock"
GUIDE = HELP / "guide.md"
SCRIPT = Path("scripts/help_shots.py")

# The toolbar's Record when armed (crates/app/src/ui/style.rs RECORD_ARMED): the colour the
# highlights were asked for in.
RED = (0xE5, 0x53, 0x3D, 255)
# The rings' line, its rounding and how far they stand off the thing they ring, in pixels.
LINE = 3
RADIUS = 7
# Drawn this many times larger, then scaled down: smooth edges.
SUPER = 4
# How long a shot waits for the window to settle, and when it quits.
AFTER_MS = 1800
QUIT_MS = 6000


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load_list():
    return json.loads(SHOTS.read_text())


def sources(shots):
    """Every file the pictures are made from, in a fixed order."""
    files = [SHOTS, SCRIPT]
    projects = sorted({shot["project"] for shot in shots["shots"].values()})
    files += [SOURCE / project for project in projects]
    return files


def guide_pictures():
    """The pictures the guide names: `![caption](file)` alone on a line."""
    names = []
    for line in GUIDE.read_text().splitlines():
        found = re.fullmatch(r"!\[[^\]]*\]\(([^/\\()]+)\)", line.strip())
        if found and found.group(1) not in names:
            names.append(found.group(1))
    return names


def shot_command(app, shot, out, home):
    """The command line of `shot`, its screenshot at `out`, its per-user folders under `home`."""
    command = [str(app), "--load", str((SOURCE / shot["project"]).resolve())]
    command += shot.get("args", [])
    command += ["--null-output", "--midi-ports", shot.get("midi_ports", "none")]
    command += ["--browser-root", str(home / "My samples")]
    command += ["--screenshot", str(out), "--screenshot-after", str(AFTER_MS)]
    command += ["--quit-after", str(QUIT_MS)]
    window = shot.get("window")
    if window:
        command += ["--screenshot-window", window]
    return command


def prepare_home(home, settings):
    """Per-user folders for one shot: the starter kit's drums in its data folder,
    `settings` (when given) as its settings file, an empty folder for the Library's samples
    (its one folder, named as a beginner's might be)."""
    if home.exists():
        shutil.rmtree(home)
    for part in ("config/dawsome", "cache", "state", "data/dawsome/starter-kit"):
        (home / "xdg" / part).mkdir(parents=True)
    (home / "My samples").mkdir()
    for sound in sorted(Path("assets/sounds").glob("*.wav")):
        shutil.copy(sound, home / "xdg/data/dawsome/starter-kit" / sound.name)
    if settings is not None:
        (home / "xdg/config/dawsome/settings.json").write_text(json.dumps(settings) + "\n")


def take_shot(app, name, shot, work, instruments):
    home = work / name
    prepare_home(home, shot.get("settings"))
    out = work / f"{name}.png"
    env = dict(os.environ)
    env.update(
        {
            "XDG_CONFIG_HOME": str(home / "xdg/config"),
            "XDG_CACHE_HOME": str(home / "xdg/cache"),
            "XDG_DATA_HOME": str(home / "xdg/data"),
            "XDG_STATE_HOME": str(home / "xdg/state"),
            "DAWSOME_APP_ID": "dawsome-test",
            "DAWSOME_SHIPPED_INSTRUMENTS": str(instruments),
        }
    )
    for name_var in ("DAWSOME_CONFIG_HOME", "DAWSOME_DATA_HOME", "DAWSOME_CACHE_HOME",
                     "DAWSOME_STATE_HOME"):
        env.pop(name_var, None)
    command = shot_command(app, shot, out, home)
    done = subprocess.run(command, env=env, capture_output=True, text=True, timeout=60)
    if done.returncode != 0 or not out.is_file():
        sys.exit(f"help_shots: the shot {name} failed:\n{done.stdout}{done.stderr}")
    print(f"help_shots: shot {name}")
    return out


def draw(picture, marks, Image, ImageDraw):
    """`marks` (rings and arrows, in the picture's own pixels) drawn over `picture`."""
    overlay = Image.new("RGBA", (picture.width * SUPER, picture.height * SUPER), (0, 0, 0, 0))
    pen = ImageDraw.Draw(overlay)
    for kind, points in marks:
        big = [value * SUPER for value in points]
        if kind == "ring":
            x0, y0, x1, y1 = big
            pen.rounded_rectangle(
                (x0, y0, x1, y1), radius=RADIUS * SUPER, outline=RED, width=LINE * SUPER
            )
        else:
            (x0, y0, x1, y1) = big
            pen.line((x0, y0, x1, y1), fill=RED, width=LINE * SUPER)
            # The head: a triangle at the tip, 14 px long and 12 px wide.
            dx, dy = x1 - x0, y1 - y0
            length = max((dx * dx + dy * dy) ** 0.5, 1.0)
            ux, uy = dx / length, dy / length
            back, half = 14 * SUPER, 6 * SUPER
            base = (x1 - ux * back, y1 - uy * back)
            pen.polygon(
                [
                    (x1, y1),
                    (base[0] - uy * half, base[1] + ux * half),
                    (base[0] + uy * half, base[1] - ux * half),
                ],
                fill=RED,
            )
    overlay = overlay.resize(picture.size, Image.LANCZOS)
    picture.alpha_composite(overlay)


def make_picture(entry, shot_png, Image, ImageDraw):
    shot = Image.open(shot_png)
    scale = float(shot.info.get("Scale Factor", "1"))
    shot = shot.convert("RGBA")
    if scale != 1.0:
        logical = (round(shot.width / scale), round(shot.height / scale))
        shot = shot.resize(logical, Image.LANCZOS)
    x0, y0, x1, y1 = entry["crop"]
    picture = shot.crop((x0, y0, x1, y1))
    factor = 1.0
    if "width" in entry and entry["width"] != picture.width:
        factor = entry["width"] / picture.width
        size = (entry["width"], round(picture.height * factor))
        picture = picture.resize(size, Image.LANCZOS)
    marks = []
    for ring in entry.get("rings", []):
        marks.append(
            ("ring", [(ring[0] - x0) * factor, (ring[1] - y0) * factor,
                      (ring[2] - x0) * factor, (ring[3] - y0) * factor])
        )
    for arrow in entry.get("arrows", []):
        marks.append(
            ("arrow", [(arrow[0] - x0) * factor, (arrow[1] - y0) * factor,
                       (arrow[2] - x0) * factor, (arrow[3] - y0) * factor])
        )
    draw(picture, marks, Image, ImageDraw)
    out = HELP / entry["file"]
    picture.convert("RGB").save(out, optimize=True)
    print(f"help_shots: {out} ({picture.width}x{picture.height})")


def make(app, work, instruments):
    from PIL import Image, ImageDraw

    shots = load_list()
    work.mkdir(parents=True, exist_ok=True)
    taken = {}
    for name, shot in shots["shots"].items():
        taken[name] = take_shot(app, name, shot, work, instruments)
    for entry in shots["pictures"]:
        make_picture(entry, taken[entry["shot"]], Image, ImageDraw)
    lock = {
        "sources": {str(path): sha256(path) for path in sources(shots)},
        "pictures": {entry["file"]: sha256(HELP / entry["file"]) for entry in shots["pictures"]},
    }
    LOCK.write_text(json.dumps(lock, indent=1, sort_keys=True) + "\n")
    missing = [name for name in guide_pictures() if name not in lock["pictures"]]
    if missing:
        sys.exit(f"help_shots: the guide names pictures shots.json does not make: {missing}")
    print(f"help_shots: {len(lock['pictures'])} pictures, {LOCK} written")


def check():
    problems = []
    if not LOCK.is_file():
        sys.exit(f"help_shots: no {LOCK}: run scripts/help-shots.sh")
    lock = json.loads(LOCK.read_text())
    shots = load_list()
    wanted = {str(path) for path in sources(shots)}
    if wanted != set(lock["sources"]):
        problems.append(f"the sources are {sorted(wanted)}, the lock has {sorted(lock['sources'])}")
    for path, digest in lock["sources"].items():
        if not Path(path).is_file() or sha256(path) != digest:
            problems.append(f"{path} changed since the pictures were made")
    listed = [entry["file"] for entry in shots["pictures"]]
    if sorted(listed) != sorted(lock["pictures"]):
        problems.append("shots.json's pictures are not the lock's")
    for name, digest in lock["pictures"].items():
        path = HELP / name
        if not path.is_file():
            problems.append(f"{path} is missing")
        elif sha256(path) != digest:
            problems.append(f"{path} is not the picture the shots made")
    for path in sorted(HELP.glob("*.png")):
        if path.name not in lock["pictures"]:
            problems.append(f"{path} is no picture of the shots")
    for name in guide_pictures():
        if name not in lock["pictures"]:
            problems.append(f"the guide names {name}, which the shots do not make")
    if problems:
        print("help_shots: the Help pictures are out of date; run scripts/help-shots.sh:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        sys.exit(1)
    print(f"help_shots: {len(lock['pictures'])} pictures match their sources")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    made = sub.add_parser("make")
    made.add_argument("--app", type=Path, required=True)
    made.add_argument("--work", type=Path, required=True)
    made.add_argument("--instruments", type=Path, required=True)
    sub.add_parser("check")
    args = parser.parse_args()
    if args.command == "make":
        make(args.app.resolve(), args.work.resolve(), args.instruments.resolve())
    else:
        check()


if __name__ == "__main__":
    main()
