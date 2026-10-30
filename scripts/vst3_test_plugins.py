#!/usr/bin/env python3
"""Fetches the real VST3 plugins the VST3 host's integration tests load, as locks/vst3-test.lock pins
them: test inputs under their own licenses, never shipped, installed or packaged.

usage: python3 -I scripts/vst3_test_plugins.py fetch [--lock FILE] --dir DIR
                                                     [--offline | --if-fetched] [NAME...]
       (scripts/fetch-vst3-test-plugins.sh runs it with DIR = <target dir>/vst3-test)

fetch NAME (dexed, js80p, dpf; default: all) makes DIR/NAME hold what the tests load of that
asset, at the place below the archive's own top folder it has there:
- dexed (Dexed 1.0.1, GPL-3.0): `Dexed.vst3` (its Linux binary and moduleinfo.json) and
  `LICENSE`;
- js80p (JS80P 4.1.1, GPL-3.0): `js80p.vst3` (its Linux and Windows binaries, moduleinfo.json,
  documentation, a snapshot) and `LICENSE.txt`;
- dpf (DPF-Plugins 1.7, 3-Band EQ under LGPL-3.0, DPF under ISC): `3BandEQ.vst3` and
  `3BandEQ.clap`, the one DSP in both formats.
Prints the folders, a line each.

The download, the archive rules and the stamps are scripts/surge.py's, read from its path: each
asset downloaded from its url into DIR/downloads/<file>/, kept only when its size and SHA-256
are the ones pinned; nothing downloaded is run; only regular files are taken, and an archive
holding a name that could climb out, or expanding past 1 GiB (256 MiB a file), is refused
whole. One difference: a link or device elsewhere in an archive is passed over, not refused
(DPF-Plugins' holds 12,687 hard links among the presets of a plugin not taken), while one among
the members taken refuses the archive. Each folder is made under a hidden name and moved into
place, with a stamp DIR/NAME.stamp naming the pin, the rules and the SHA-256 of every file: a
folder is current only while all of that holds, else it is made again from the download.
--offline downloads nothing and gives only what is fetched (status 3 otherwise); --if-fetched
leaves what is not fetched out of what it prints instead.

Exit status: 0 done; 1 a file was refused or could not be fetched; 2 the command line or the
lock is wrong; 3 (--offline) not fetched.

Standard library only (Python 3.11 or later).
"""

import argparse
import importlib.util
import json
import os
import shutil
import stat
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def load_surge():
    """scripts/surge.py as a module, read from its path (run with -I, no folder is on
    sys.path)."""
    spec = importlib.util.spec_from_file_location(
        "surge", os.path.join(os.path.dirname(os.path.abspath(__file__)), "surge.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


surge = load_surge()

# What each asset's folder holds: the archive's top folder (`top`, "" for none) and, below it,
# the files and folders taken (`take`), each at the same place in DIR/NAME.
TAKEN = {
    "dexed": {"top": "", "take": ("Dexed.vst3", "LICENSE")},
    "js80p": {"top": "js80p-4_1_1-avx-vst3_bundle", "take": ("js80p.vst3", "LICENSE.txt")},
    "dpf": {"top": "DPF-Plugins-v1.7", "take": ("3BandEQ.vst3", "3BandEQ.clap")},
}
# Raised when extract() changes what it takes in a way TAKEN does not show: a folder made under
# other rules is made again (its stamp records them).
RULES = 1


def parts_of(path):
    return tuple(part for part in path.split("/") if part)


def extract(name, archive_path, staged):
    """What TAKEN names for `name` from the archive at `archive_path` into `staged`; returns the
    SHA-256 of each file written, by its path below `staged` (`/` between the parts)."""
    rules = TAKEN[name]
    top = parts_of(rules["top"])
    written = {}
    with surge.Members(archive_path, others=True) as archive:
        for taken in rules["take"]:
            wanted = top + parts_of(taken)
            links = [parts for parts in archive.others if parts[:len(wanted)] == wanted]
            if links:
                raise surge.Refused(f"{archive.name}: {'/'.join(links[0])} is not a file or a "
                                    "folder")
            files = [parts for parts in archive.files if parts[:len(wanted)] == wanted]
            if not files:
                raise surge.Refused(f"{archive.name} holds no {'/'.join(wanted)}")
            for parts in files:
                rest = parts[len(top):]
                target = surge.inside(staged, rest)
                os.makedirs(os.path.dirname(target), exist_ok=True)
                written["/".join(rest)] = archive.copy(parts, target, mode(archive, parts))
    return written


def mode(archive, parts):
    """0o755 for a member the archive marks executable, else 0o644."""
    member = archive.files[parts]
    raw = member.mode if archive.kind == "tar" else member.external_attr >> 16
    return 0o755 if raw & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH) else 0o644


def made_by(lock, name):
    """What a folder's stamp records besides its files: the lock's version, the asset's pin and
    the rules it is made by."""
    return {
        "version": lock.version,
        "asset": list(lock.asset(name)),
        "rules": {"version": RULES, "top": TAKEN[name]["top"],
                  "take": list(TAKEN[name]["take"])},
    }


def plugin_folder(lock, name, folder, offline):
    """DIR/NAME, current with the lock and the rules: made from the download when it is not."""
    target = os.path.join(folder, name)
    stamp = target + ".stamp"
    made = made_by(lock, name)
    if surge.is_current(target, stamp, made):
        return target
    if os.path.exists(target):
        print(f"vst3-test: {target} is not the folder its stamp records (a file changed, or it "
              "was made under other rules): it is made again", file=sys.stderr)
    archive = surge.download(lock, name, folder, offline)
    staged = os.path.join(folder, f".{name}.{os.getpid()}.tmp")
    shutil.rmtree(staged, ignore_errors=True)
    os.makedirs(staged)
    try:
        files = extract(name, archive, staged)
        if os.path.exists(stamp):
            os.remove(stamp)
        shutil.rmtree(target, ignore_errors=True)
        os.replace(staged, target)
    finally:
        shutil.rmtree(staged, ignore_errors=True)
    with open(stamp + ".tmp", "w", encoding="utf-8") as file:
        json.dump({**made, "files": files}, file, indent=1, sort_keys=True)
        file.write("\n")
    os.replace(stamp + ".tmp", stamp)
    return target


def main(argv):
    parser = argparse.ArgumentParser(prog="vst3_test_plugins.py",
                                     description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    fetch = commands.add_parser("fetch")
    fetch.add_argument("--lock", default=os.path.join(ROOT, "locks", "vst3-test.lock"))
    fetch.add_argument("--dir", required=True)
    fetch_mode = fetch.add_mutually_exclusive_group()
    fetch_mode.add_argument("--offline", action="store_true")
    fetch_mode.add_argument("--if-fetched", action="store_true")
    fetch.add_argument("names", nargs="*", choices=sorted(TAKEN), metavar="NAME")
    try:
        args = parser.parse_args(argv)
    except SystemExit as exit_:
        return int(exit_.code or 0)
    try:
        lock = surge.Lock(args.lock)
        folder = os.path.abspath(args.dir)
        offline = args.offline or args.if_fetched
        found = []
        for name in args.names or sorted(TAKEN):
            try:
                found.append(plugin_folder(lock, name, folder, offline))
            except surge.NotFetched:
                if not args.if_fetched:
                    raise
    except (OSError, ValueError) as error:
        print(f"vst3-test: {error}", file=sys.stderr)
        return 2
    except surge.Refused as error:
        print(f"vst3-test: refused: {error}", file=sys.stderr)
        return 1
    except surge.NotFetched as error:
        print(f"vst3-test: not fetched ({error}): scripts/fetch-vst3-test-plugins.sh fetches it",
              file=sys.stderr)
        return 3
    if found:
        print("\n".join(found))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
