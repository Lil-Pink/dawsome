#!/usr/bin/env python3
"""Checks licenses/THIRD_PARTY.md and licenses/NOTICE against the crates the two programs are built from, and
licenses/SOUNDS.md against the sounds they ship.

usage: python3 -I scripts/license-check.py [--third-party FILE] [--notice FILE]
       python3 -I scripts/license-check.py --sounds KIT [--write]
       (from the repository root; scripts/license-check.sh runs it there)

--sounds KIT: licenses/SOUNDS.md must be what the shipped sounds' manifests make of it: the starter kit
and the electronic sets (assets/sounds/kit.json, assets/instruments/*/set.json) and the kit's
sets (KIT, the dawsome-kit repository scripts/fetch-kit.sh fetched: its MANIFEST.json and each
set's set.json), every sound, set and file CC0-1.0 with its source; and the bundled plugin's
factory data, under the plugin's own license, as locks/surge.lock pins it; --write writes it. The
kit's own tools/check.py --manifest holds each of its files to MANIFEST.json's SHA-256.

For each target in deny.toml's [graph] targets, the crates `dawsome` and `dawsome-app` are
built from are their normal dependencies, proc macros left out (those run only while
compiling). Each one whose license cannot be met with MIT or Apache-2.0 alone must have a row
in licenses/THIRD_PARTY.md's crate table, with the license its manifest declares and the platforms that
build it in, and must be named in NOTICE. A row for a crate neither program is built from fails
too. Exit 1 on any difference, every difference listed; exit 2 when the check cannot run.

Why not the listing `cargo deny list` prints: it flattens an expression into its license
names, so "0BSD OR MIT OR Apache-2.0" (met by MIT alone) looks like "(MIT OR Apache-2.0) AND
Unicode-3.0" (which also asks for the Unicode notice), and it counts the tools used only while
building (bindgen). The licenses here come from the same manifest field cargo-deny reads.

Standard library only (Python 3.11 or later, for tomllib).
"""

import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys
import tomllib

PROGRAMS = ("dawsome", "dawsome-app")
# The name licenses/THIRD_PARTY.md gives each release target, in the order its Platforms cells use.
PLATFORMS = {"x86_64-unknown-linux-gnu": "Linux", "x86_64-pc-windows-msvc": "Windows"}
# A crate whose license can be met with these alone needs no row.
WITHOUT_ROW = {"MIT", "Apache-2.0"}
COLUMNS = ("Crate", "License", "Platforms")
# One line of `cargo tree --format "{p}\t{l}"`: name, version, the source of a path or git
# dependency, the license expression, and the mark of a crate printed before.
TREE_LINE = re.compile(
    r"(?P<name>\S+) v(?P<version>\S+)(?: \([^)]*\))?\t(?P<license>.*?)(?: \(\*\))?"
)


class Problem(Exception):
    """The check cannot run: a tool failed, or a file is not laid out as the check reads it."""


def run(*args):
    """The standard output of `args`; Problem with its error output when it fails."""
    done = subprocess.run(args, capture_output=True, text=True)
    if done.returncode != 0:
        raise Problem(f"{' '.join(args)} failed:\n{done.stderr.strip()}")
    return done.stdout


def met_without_row(expression):
    """Whether the SPDX `expression` can be met with the licenses in WITHOUT_ROW alone. An
    exception (`WITH`) only adds permissions and a `+` only admits later versions, so
    `Apache-2.0 WITH LLVM-exception` counts as Apache-2.0; the old `/` separator reads as OR.
    AND binds tighter than OR."""
    words = re.findall(r"[()]|[^\s()]+", expression.replace("/", " OR "))
    position = 0

    def peek():
        return words[position] if position < len(words) else None

    def take():
        nonlocal position
        if position == len(words):
            raise Problem(f"the license expression {expression!r} ends early")
        position += 1
        return words[position - 1]

    def either():
        met = both()
        while peek() == "OR":
            take()
            right = both()
            met = met or right
        return met

    def both():
        met = term()
        while peek() == "AND":
            take()
            right = term()
            met = met and right
        return met

    def term():
        word = take()
        if word == "(":
            met = either()
            if take() != ")":
                raise Problem(f"the license expression {expression!r} has an unclosed (")
            return met
        if word in ("AND", "OR", "WITH", ")"):
            raise Problem(f"the license expression {expression!r} has {word!r} out of place")
        if peek() == "WITH":
            take()
            take()
        return word.removesuffix("+") in WITHOUT_ROW

    met = either()
    if peek() is not None:
        raise Problem(f"the license expression {expression!r} has {peek()!r} out of place")
    return met


def release_targets(path):
    """The targets in the [graph] table of the cargo-deny configuration at `path`."""
    with open(path, "rb") as config:
        targets = tomllib.load(config).get("graph", {}).get("targets", [])
    if not targets:
        raise Problem(f"{path} names no [graph] targets")
    unnamed = [target for target in targets if target not in PLATFORMS]
    if unnamed:
        raise Problem(f"give {', '.join(unnamed)} a platform name in PLATFORMS")
    return targets


def workspace_crates():
    """The names of the workspace's own crates."""
    metadata = json.loads(run("cargo", "metadata", "--format-version", "1", "--no-deps"))
    return {package["name"] for package in metadata["packages"]}


def built_in(target, own):
    """name -> {version: license expression} of the crates the programs are built from for
    `target`, the workspace's own crates (`own`) left out."""
    packages = [argument for program in PROGRAMS for argument in ("--package", program)]
    listing = run(
        "cargo", "tree", "--quiet", "--target", target, *packages,
        "--edges", "normal,no-proc-macro", "--prefix", "none", "--format", "{p}\t{l}",
    )
    crates = {}
    for line in filter(None, listing.splitlines()):
        match = TREE_LINE.fullmatch(line)
        if match is None:
            raise Problem(f"cargo tree printed a line this check cannot read: {line!r}")
        if match["name"] not in own:
            crates.setdefault(match["name"], {})[match["version"]] = match["license"]
    return crates


def cells(line):
    """The cells of a Markdown table line, or [] for a line outside a table."""
    line = line.strip()
    if not line.startswith("|"):
        return []
    return [cell.strip() for cell in line.strip("|").split("|")]


def listed_crates(text, source):
    """name -> (license, platforms) from the first table of `text` whose header names every
    column in COLUMNS. A row may name several crates, each in backticks."""
    lines = text.splitlines()
    starts = [index for index, line in enumerate(lines) if set(COLUMNS) <= set(cells(line))]
    if not starts:
        raise Problem(f"{source} has no table with the columns {', '.join(COLUMNS)}")
    header = cells(lines[starts[0]])
    body = lines[starts[0] + 1:]
    if not body or not re.fullmatch(r"\|[-:| ]+\|", body[0].strip()):
        raise Problem(f"{source}: the crate table's header has no delimiter row under it")
    listed = {}
    for line in body[1:]:
        row = cells(line)
        if not row:
            break
        if len(row) != len(header):
            raise Problem(f"{source}: a row of {len(row)} cells under {len(header)}: {line}")
        row = dict(zip(header, row))
        names = re.findall(r"`([^`]+)`", row["Crate"])
        if not names:
            raise Problem(f"{source}: a row that names no crate in backticks: {line}")
        platforms = {platform.strip() for platform in row["Platforms"].split(",")}
        unknown = platforms - set(PLATFORMS.values())
        if unknown:
            raise Problem(f"{source}: unknown platform {', '.join(sorted(unknown))}: {line}")
        for name in names:
            if name in listed:
                raise Problem(f"{source}: two rows for {name}")
            listed[name] = (" ".join(row["License"].split()), platforms)
    return listed


def shown(platforms):
    """The platforms as a Platforms cell writes them: "Linux, Windows"."""
    return ", ".join(name for name in PLATFORMS.values() if name in platforms)


def needs_row(licenses):
    """Whether a crate whose versions declare `licenses` needs a row."""
    return not all(met_without_row(license) for license in licenses)


def differences(listed, crates_by_target, notice):
    """Every way licenses/THIRD_PARTY.md (`listed`) and licenses/NOTICE miss or misstate the crates built in."""
    licenses, platforms = {}, {}
    for target, crates in crates_by_target.items():
        for name, versions in crates.items():
            licenses.setdefault(name, set()).update(versions.values())
            platforms.setdefault(name, set()).add(PLATFORMS[target])
    found = []
    for name in sorted(licenses):
        declared = sorted(licenses[name])
        quoted = " and ".join(map(repr, declared))
        if "" in declared:
            found.append(f"{name}: its manifest declares no license; read its license files")
            continue
        if name not in listed:
            if needs_row(declared):
                found.append(f"{name} ({quoted}; {shown(platforms[name])}) has no row")
            continue
        license, listed_platforms = listed[name]
        if declared != [license]:
            found.append(f"{name}: the row says {license!r}, its manifest {quoted}")
        if listed_platforms != platforms[name]:
            found.append(
                f"{name}: the row says {shown(listed_platforms)}, "
                f"but the programs are built from it on {shown(platforms[name])}"
            )
        if needs_row(declared) and not re.search(rf"(?<![\w-]){re.escape(name)}(?![\w-])", notice):
            found.append(f"{name} is not named in licenses/NOTICE")
    for name in sorted(set(listed) - set(licenses)):
        found.append(f"{name} has a row, but neither program is built from it")
    return found


SOUNDS = "licenses/SOUNDS.md"
LICENSE = "CC0-1.0"


def cell(text):
    """`text` as a Markdown table cell: one line, its bars escaped."""
    return " ".join(str(text).split()).replace("|", "\\|")


def read_json(path):
    with open(path, encoding="utf-8") as file:
        return json.load(file)


def surge_lock():
    """locks/surge.lock, read by scripts/surge.py's own reader."""
    sys.dont_write_bytecode = True                   # no __pycache__ beside the scripts
    here = os.path.dirname(os.path.abspath(__file__))
    spec = importlib.util.spec_from_file_location("surge", os.path.join(here, "surge.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.Lock(os.path.join("locks", "surge.lock"))


def sounds_text(kit):
    """licenses/SOUNDS.md as the manifests of the shipped sounds make it, with the kit at `kit`; Problem
    when a sound, a set or a file is not CC0-1.0 or names no source."""
    found = []
    manifest = read_json("assets/sounds/kit.json")
    if manifest.get("licence") != LICENSE:
        found.append(f"assets/sounds/kit.json: the kit's license is not {LICENSE}")
    rows = []
    for sound in manifest["sounds"]:
        if sound.get("licence") != LICENSE or not sound.get("source"):
            found.append(f"assets/sounds/kit.json: {sound.get('name')}: not {LICENSE} with a source")
        where = ("starter kit" if sound["note"] == "drum"
                 else "set `%s`" % sound["name"].lower().replace(" ", "-"))
        rows.append(f"| {cell(sound['name'])} | {where} | {cell(sound['source'])} |")
    for folder in sorted(os.listdir("assets/instruments")):
        about = read_json(os.path.join("assets/instruments", folder, "set.json"))
        if about.get("licence") != LICENSE:
            found.append(f"assets/instruments/{folder}/set.json: not {LICENSE}")
    commit = run("git", "-C", kit, "rev-parse", "HEAD").strip()
    per_set = {}
    for entry in read_json(os.path.join(kit, "MANIFEST.json"))["files"]:
        name = entry["path"].split("/")[1]
        per_set[name] = per_set.get(name, 0) + 1
        if entry.get("licence") != LICENSE or not entry.get("source"):
            found.append(f"{entry['path']}: not {LICENSE} with a source in the kit's manifest")
    kit_rows = []
    sets = os.path.join(kit, "sets")
    for folder in sorted(os.listdir(sets)):
        about = read_json(os.path.join(sets, folder, "set.json"))
        if about.get("licence") != LICENSE or not about.get("licence_page"):
            found.append(f"{folder}/set.json: not {LICENSE} with a license page")
        kit_rows.append(" | ".join([
            f"| `{folder}`", cell(about["name"]), cell(about["maker"]), cell(about["source"]),
            str(per_set.get(folder, 0)), about["licence"], f"{cell(about['licence_page'])} |"]))
    if found:
        raise Problem("the shipped sounds are not all CC0-1.0 with a source:\n  "
                      + "\n  ".join(found))
    surge = surge_lock()
    data, data_size, data_digest = surge.asset("data")
    return "\n".join([
        "# The sounds Dawsome ships",
        "",
        "Every sound the programs ship is dedicated to the public domain under CC0 1.0 Universal",
        "(CC0-1.0) at its source, or made for Dawsome and dedicated so: use, change, sell and",
        "share it, in songs or in software, with no credit due. The bundled plugin's own factory",
        "data is the one exception, under the plugin's license (the last section). This file says",
        "where each comes from. `scripts/license-check.py --sounds` writes it from the manifests,",
        "and `scripts/check.sh` fails when it is out of date.",
        "",
        "## The starter kit and the electronic instrument sets",
        "",
        "Made for Dawsome, CC0-1.0. `assets/sounds/kit.json`",
        "states each file's SHA-256 and how it was made; the drums are the starter kit",
        "(`assets/sounds/`), each pitched sound an instrument set of its own",
        "(`assets/instruments/`).",
        "",
        "| Sound | Where | Made after |",
        "|-|-|-|",
        *rows,
        "",
        "## The instrument sets of dawsome-kit",
        "",
        "The dawsome-kit repository, installed with the programs, at the commit `locks/kit.lock` pins:",
        f"`{commit}`. Its `MANIFEST.json` names the source, the license page",
        "and the SHA-256 of every one of its files, and its `tools/check.py --manifest` holds the files",
        "to it. Files counts every file of a set: its sounds, its .sfz and its `set.json`.",
        "",
        "| Set | Name | Maker | Source | Files | License | License page |",
        "|-|-|-|-|-|-|-|",
        *kit_rows,
        "",
        "## The bundled plugin: Surge XT",
        "",
        f"Surge XT {surge.version}, the Surge Synth Team's synthesizer, is bundled as a CLAP plugin",
        "(`plugins/` in a release zip, `lib/dawsome/clap/` installed) with its factory data in",
        "`SurgeXTData/`: patches, wavetables, skins, tunings and presets, under the plugin's",
        "license, GPL-3.0-or-later (`LICENSE-SURGE`); the third-party patches and wavetables of its",
        "data, and its tutorial skins, are left out. The sounds you play and record with it are",
        "yours, as Surge's own FAQ says: \"The output of Surge XT is yours to do with as you",
        "want!\" Its data comes unchanged from the release asset",
        f"`{data}` ({data_size} bytes, SHA-256",
        f"`{data_digest}`), which `locks/surge.lock` pins.",
        "",
    ])


def check_sounds(kit, write):
    """licenses/SOUNDS.md against what sounds_text makes; writes it with `write`. The exit status as
    main's: 1 when it differs or a sound is not CC0-1.0, 2 when the check cannot run."""
    try:
        text = sounds_text(kit)
    except Problem as error:
        print(f"license-check: {error}", file=sys.stderr)
        return 1
    except (OSError, KeyError, IndexError, ValueError) as error:
        print(f"license-check: the sounds' manifests cannot be read: {error}", file=sys.stderr)
        return 2
    if write:
        with open(SOUNDS, "w", encoding="utf-8") as file:
            file.write(text)
        return 0
    try:
        with open(SOUNDS, encoding="utf-8") as file:
            same = file.read() == text
    except OSError:
        same = False
    if not same:
        print(f"license-check: {SOUNDS} is out of date: scripts/license-check.py --sounds "
              f"{kit} --write", file=sys.stderr)
        return 1
    print(f"license-check: {SOUNDS} lists every shipped sound, all {LICENSE}, and the bundled "
          "plugin's factory data under its own license")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--third-party", default="licenses/THIRD_PARTY.md")
    parser.add_argument("--notice", default="licenses/NOTICE")
    parser.add_argument("--sounds", metavar="KIT")
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    if args.sounds:
        return check_sounds(args.sounds, args.write)
    if args.write:
        parser.error("--write goes with --sounds")
    try:
        targets = release_targets("deny.toml")
        with open(args.third_party, encoding="utf-8") as file:
            listed = listed_crates(file.read(), args.third_party)
        with open(args.notice, encoding="utf-8") as file:
            notice = file.read()
        own = workspace_crates()
        crates_by_target = {target: built_in(target, own) for target in targets}
        found = differences(listed, crates_by_target, notice)
    except (Problem, OSError, tomllib.TOMLDecodeError) as error:
        print(f"license-check: {error}", file=sys.stderr)
        return 2
    if found:
        print(
            f"license-check: {args.third_party} or {args.notice} is out of date:", file=sys.stderr
        )
        for difference in found:
            print(f"  {difference}", file=sys.stderr)
        return 1
    for target, crates in crates_by_target.items():
        other = sorted(name for name, versions in crates.items() if needs_row(versions.values()))
        print(
            f"license-check: {PLATFORMS[target]}: {len(crates)} crates built in, "
            f"{len(other)} under other licenses, all listed: {', '.join(other)}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
