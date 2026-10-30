#!/usr/bin/env python3
"""Writes licenses/NOTICES.txt, the license and notice texts of the works built into the two programs, or
checks that the committed file is what it would write.

usage: python3 -I scripts/notices.py [--check] [--out FILE]
       (from anywhere; it reads the repository it is in)

The Rust crates are the normal dependencies of `dawsome` and `dawsome-app` on each target in
deny.toml's [graph] targets (Linux and Windows), united; proc macros and what only they use are
left out (they run only while compiling), and so are the workspace's own crates: the listing
scripts/license-check.py reads, `cargo tree -p dawsome -p dawsome-app --target <each> --edges
normal,no-proc-macro`. Why not the resolve `cargo metadata --filter-platform` gives: it unites
the features of the whole workspace, so a feature only the test plugins turn on adds crates the
release build never compiles (clack-plugin, through clack-extensions, and seven more on
2026-10-29). `cargo metadata` gives each crate's folder and manifest fields. For each: its name, version,
the license expression and repository from its manifest, the platforms that build it in, and the
texts of its license and notice files (LICENSE*, LICENCE*, COPYING*, COPYRIGHT*, NOTICE*,
UNLICENSE*, and the manifest's license-file) from its folder in the cargo registry, word for word.
A crate with none on disk gets the line "license text: see the crate's repository": no text is
made up for it, since a standard license's text without the crate's own copyright line would
state a notice nobody wrote. The C and C++ libraries the crates' build scripts compile in are in
C_LIBRARIES below, their texts read from the crate that vendors them; then Surge XT, which ships
beside the programs, points at LICENSE-SURGE and SOURCES.md.

--check: the file is made in memory and compared with the committed one; exit 1 with what
differs, so a Cargo.lock change without a new licenses/NOTICES.txt fails scripts/check.sh. Exit 2 when it
cannot run (cargo fails, a C library's files are not where C_LIBRARIES says).

Why not cargo-about: one more tool for every contributor to install, and it reads the network.
This needs neither: cargo metadata runs offline and the registry holds every crate's files once
the crates are fetched (any build fetches them). The output depends only on Cargo.lock, those
files and this script: everything is sorted, line endings become LF and each text ends with one
line break, so two runs write the same bytes.

Standard library only (Python 3.11 or later, for tomllib).
"""

import argparse
import difflib
import json
import os
import re
import subprocess
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = "licenses/NOTICES.txt"
PROGRAMS = ("dawsome", "dawsome-app")
PLATFORMS = {"x86_64-unknown-linux-gnu": "Linux", "x86_64-pc-windows-msvc": "Windows"}
# A file in a crate's top folder whose name starts with one of these (any case) is a license or
# notice text; a folder of that name (LICENSES/) gives each file in it.
TEXT_NAMES = ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE")
NO_TEXT = "license text: see the crate's repository"
RULE = "=" * 78
LINE = "-" * 78

# The C and C++ code the crates' build scripts compile into the programs (read from each
# build.rs: cc, cmake or autotools building sources, not pkg-config linking a system library).
# The crate's own small C files (wayland-backend's log shim, libspa's and libspa-sys's wrappers)
# are the crate's, under its license above. "texts" are paths in the vendoring crate's folder;
# a missing one stops the script, so a crate update that moves them is noticed.
C_LIBRARIES = (
    {
        "name": "LAME 3.100",
        "license": "LGPL-2.0-or-later",
        "crate": "mp3lame-sys",
        "what": "the MP3 encoder, built from the source in mp3lame-sys's lame-3.100/ (MP3 export)",
        "repository": "https://lame.sourceforge.io",
        "texts": ("lame-3.100/COPYING", "lame-3.100/LICENSE"),
    },
    {
        "name": "Signalsmith Stretch",
        "license": "MIT",
        "crate": "signalsmith-stretch",
        "what": "the time-stretch and pitch-shift library (C++), compiled from the copy in "
                "signalsmith-stretch's signalsmith-stretch/",
        "repository": "https://github.com/Signalsmith-Audio/signalsmith-stretch",
        "texts": ("signalsmith-stretch/LICENSE.txt",),
    },
    {
        "name": "Signalsmith Linear",
        "license": "MIT",
        "crate": "signalsmith-stretch",
        "what": "the FFT and STFT that Signalsmith Stretch uses (C++), compiled from the copy in "
                "signalsmith-stretch's signalsmith-linear/",
        "repository": "https://github.com/Signalsmith-Audio/linear",
        "texts": ("signalsmith-linear/LICENSE.txt",),
    },
    {
        "name": "SPA, PipeWire's plugin API headers",
        "license": "MIT",
        "crate": "libspa-sys",
        "what": "inline functions of the headers, compiled into libspa-sys's wrappers from the "
                "PipeWire development headers of the machine that builds Linux programs",
        "repository": "https://gitlab.freedesktop.org/pipewire/pipewire",
        "texts": (),
    },
)

HEADER = """\
Dawsome {version} bundles these works; each keeps its own license.

Dawsome itself is GPL-3.0-or-later: NOTICE holds its notice and LICENSE the license. This file
holds the license and notice texts of the works built into its two programs, dawsome and
dawsome-app, word for word as each ships them, in three parts:

  Rust crates: {crates} ({counts})
  C and C++ libraries the crates' build scripts compile in: {libraries}
  Beside the programs: Surge XT, a separate plugin

A crate whose folder holds no license file is listed with its license and repository, where the
text is. Code ported into Dawsome from others (Airwindows) is in THIRD_PARTY.md with its license
text; THIRD_PARTY.md also lists each crate under a license other than MIT or Apache-2.0.

The programs also use libraries the system provides when they run (on Linux ALSA, JACK, PipeWire,
D-Bus, the X11 and Wayland libraries and the graphics driver; on Windows the system's DLLs);
those are not part of Dawsome and are not listed here.

scripts/notices.py writes this file from Cargo.lock and each crate's own files.

"""


class Problem(Exception):
    """The file cannot be made: cargo failed, or a file is not where this script expects it."""


def cargo(*args):
    done = subprocess.run(["cargo", *args, "--locked", "--offline"], cwd=ROOT,
                          capture_output=True, text=True)
    if done.returncode != 0:
        raise Problem(f"cargo {' '.join(args)} failed:\n{done.stderr.strip()}")
    return done.stdout


def release_targets():
    with open(os.path.join(ROOT, "deny.toml"), "rb") as file:
        targets = tomllib.load(file).get("graph", {}).get("targets")
    if not targets:
        raise Problem("deny.toml has no [graph] targets")
    return targets


def built_in(target, packages, own):
    """The packages the programs are built from on `target`, by "name version": their normal
    dependencies, proc macros left out, as the release build resolves their features."""
    listing = cargo("tree", "--quiet", "--color", "never", "--target", target, *(f"-p={p}" for p in PROGRAMS),
                    "--edges", "normal,no-proc-macro", "--prefix", "none", "--format", "{p}")
    built = {}
    for line in filter(None, listing.splitlines()):
        name, version = line.split(" ")[:2]
        if name in own:
            continue
        if not version.startswith("v") or (name, version[1:]) not in packages:
            raise Problem(f"cargo tree printed a crate cargo metadata does not hold: {line!r}")
        built[f"{name} {version[1:]}"] = packages[(name, version[1:])]
    return built


def workspace_version(metadata, name):
    """The version of the workspace's package `name`."""
    return next(p["version"] for p in metadata["packages"]
                if p["name"] == name and p["id"] in metadata["workspace_members"])


def version_key(version):
    return [(0, int(part), "") if part.isdigit() else (1, 0, part)
            for part in re.split(r"[.+-]", version)]


def read_text(path):
    """The file's text, line endings as LF, ending with one line break."""
    with open(path, "rb") as file:
        data = file.read()
    try:
        text = data.decode("utf-8-sig")
    except UnicodeDecodeError:
        text = data.decode("latin-1")
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    return text.rstrip("\n") + "\n"


def license_files(folder, license_file):
    """The license and notice files of the crate in `folder`, as (name, path), sorted."""
    found = {}
    for name in os.listdir(folder):
        if not name.upper().startswith(TEXT_NAMES):
            continue
        path = os.path.join(folder, name)
        if os.path.isdir(path):
            for inner in os.listdir(path):
                if os.path.isfile(os.path.join(path, inner)):
                    found[f"{name}/{inner}"] = os.path.join(path, inner)
        elif os.path.isfile(path):
            found[name] = path
    if license_file:
        path = os.path.normpath(os.path.join(folder, license_file))
        if os.path.isfile(path):
            found.setdefault(os.path.relpath(path, folder).replace(os.sep, "/"), path)
    return sorted(found.items())


def texts_block(files):
    out = []
    for name, path in files:
        out += ["", f"----- {name} -----", "", read_text(path).rstrip("\n")]
    return out


def make():
    """The file's text, and what the caller reports: crates per platform, those without text."""
    metadata = json.loads(cargo("metadata", "--format-version", "1"))
    own = {p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]}
    missing = set(PROGRAMS) - own
    if missing:
        raise Problem(f"the workspace does not hold the packages {', '.join(sorted(missing))}")
    packages = {(p["name"], p["version"]): p for p in metadata["packages"]}
    by_target = {target: built_in(target, packages, own) for target in release_targets()}
    crates = {}
    for target, built in by_target.items():
        for key, package in built.items():
            crates.setdefault(key, (package, []))[1].append(PLATFORMS.get(target, target))
    version = workspace_version(metadata, "dawsome")
    counts = ", ".join(f"{len(built)} on {PLATFORMS.get(target, target)}"
                       for target, built in by_target.items())
    lines = HEADER.format(version=version, crates=len(crates), counts=counts,
                          libraries=len(C_LIBRARIES)).split("\n")
    lines = lines[:-1]
    lines += [RULE, "Rust crates", RULE]
    without_text = []
    order = sorted(crates.values(),
                   key=lambda entry: (entry[0]["name"], version_key(entry[0]["version"])))
    # The folder and platforms of each crate by name, for the C libraries they vendor.
    vendors = {}
    for package, platforms in order:
        folder = os.path.dirname(package["manifest_path"])
        vendors.setdefault(package["name"], (folder, platforms))
        files = license_files(folder, package.get("license_file"))
        license = package.get("license") or (
            f"none in its manifest; its license file is {package['license_file']}"
            if package.get("license_file") else "none in its manifest")
        lines += ["", LINE, f"{package['name']} {package['version']}",
                  f"License: {license}",
                  f"Repository: {package.get('repository') or 'none in its manifest'}",
                  f"Platforms: {', '.join(sorted(platforms))}"]
        if files:
            lines += texts_block(files)
        else:
            lines += [NO_TEXT]
            without_text.append(f"{package['name']} {package['version']} ({license})")
    lines += ["", "", RULE, "C and C++ libraries compiled in", RULE]
    for library in C_LIBRARIES:
        if library["crate"] not in vendors:
            raise Problem(f"C_LIBRARIES names {library['crate']}, which the programs are no "
                          f"longer built from: remove or change its entry ({library['name']})")
        folder, platforms = vendors[library["crate"]]
        files = []
        for name in library["texts"]:
            path = os.path.join(folder, name)
            if not os.path.isfile(path):
                raise Problem(f"{library['crate']} has no {name} ({library['name']}): read its "
                              f"build script again and change C_LIBRARIES")
            files.append((name, path))
        lines += ["", LINE, library["name"], f"License: {library['license']}",
                  f"Repository: {library['repository']}",
                  f"Built in by: {library['crate']}, {library['what']}",
                  f"Platforms: {', '.join(sorted(platforms))}"]
        lines += texts_block(files) if files else [
            "license text: see the library's repository"]
    surge = surge_version()
    lines += ["", "", RULE, "Beside the programs: Surge XT", RULE, "",
              f"Surge XT {surge}, the Surge Synth Team's synthesizer, GPL-3.0-or-later, ships as a",
              "separate CLAP plugin with its factory data, unchanged; it is not built into the",
              "programs. LICENSE-SURGE holds its notice and license, and SOURCES.md in a release",
              "zip says where its complete source is."]
    text = "\n".join(lines) + "\n"
    return text, by_target, len(crates), without_text


def surge_version():
    with open(os.path.join(ROOT, "locks", "surge.lock"), encoding="utf-8") as file:
        for line in file:
            if line.startswith("version="):
                return line.strip().split("=", 1)[1]
    raise Problem("locks/surge.lock has no version= line")


def titles(text):
    """The section titles of a NOTICES.txt: the line after each rule of dashes that a License
    line follows (a license text may hold such a rule itself)."""
    lines = text.split("\n")
    return {lines[i + 1] for i in range(len(lines) - 2)
            if lines[i] == LINE and lines[i + 2].startswith("License: ")}


def check(text):
    path = os.path.join(ROOT, OUT)
    try:
        with open(path, encoding="utf-8", newline="") as file:
            committed = file.read()
    except FileNotFoundError:
        committed = ""
    if committed == text:
        return []
    made, have = titles(text), titles(committed)
    out = [f"notices: {OUT} is not what scripts/notices.py writes: write it with 'python3 -I scripts/notices.py' and commit it"]
    for word, names in (("missing", made - have), ("not built in any more", have - made)):
        if names:
            out.append(f"  {len(names)} {word}: {', '.join(sorted(names)[:10])}"
                       + (" ..." if len(names) > 10 else ""))
    diff = list(difflib.unified_diff(committed.split("\n"), text.split("\n"), OUT,
                                     "written", n=0, lineterm=""))
    changed = sum(1 for line in diff if line[:1] in "+-" and line[:3] not in ("+++", "---"))
    out.append(f"  {changed} lines differ; the first:")
    out += ["    " + line for line in diff[2:12]]
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true",
                        help=f"compare with the committed {OUT} instead of writing it")
    parser.add_argument("--out", default=os.path.join(ROOT, OUT), help="where to write it")
    args = parser.parse_args()
    try:
        text, by_target, count, without_text = make()
    except Problem as problem:
        print(f"notices: {problem}", file=sys.stderr)
        return 2
    platforms = ", ".join(f"{PLATFORMS.get(t, t)} {len(b)}" for t, b in by_target.items())
    if args.check:
        differences = check(text)
        if differences:
            print("\n".join(differences), file=sys.stderr)
            return 1
        print(f"notices: {OUT} matches the {count} crates built in ({platforms}), "
              f"{len(without_text)} without a license text on disk")
        return 0
    with open(args.out, "w", encoding="utf-8", newline="") as file:
        file.write(text)
    print(f"notices: wrote {args.out}: {count} crates ({platforms}), "
          f"{len(text.encode('utf-8'))} bytes")
    print(f"notices: {len(without_text)} crates have no license text on disk "
          f"(\"{NO_TEXT}\"):")
    print("\n".join("  " + crate for crate in without_text))
    return 0


if __name__ == "__main__":
    sys.exit(main())
