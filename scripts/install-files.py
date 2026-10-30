#!/usr/bin/env python3
"""The files of an install of Dawsome under a prefix, for scripts/install.sh: puts them in place
and records them, and removes exactly what it recorded.

usage: python3 -I scripts/install-files.py install --prefix DIR [--legacy PLACE]...
           [--legacy-folder FOLDER]... [--keep FOLDER]...
           (--file MODE SOURCE PLACE | --tree SOURCE PLACE)...
       python3 -I scripts/install-files.py uninstall --prefix DIR [--legacy PLACE]...
           [--legacy-folder FOLDER]...

PLACE and FOLDER are paths below the prefix, `/` between their parts.

install: --file puts the file SOURCE at PLACE with MODE (octal); --tree puts every file under
the folder SOURCE at the same path below PLACE, 755 when it is executable, else 644. Each file
is copied beside its place under a hidden name and renamed into it, so a running program keeps
its file. Before any file is put, lib/dawsome/install-record names every folder the install
makes (and keeps those an earlier install made), and lib/dawsome/install-manifest names every
file of this install and of the earlier one, so an install that stops halfway can still be
removed. Once every file is in place, the files of the earlier install that this one does not
put are removed, with each folder the record names that this leaves empty, and the manifest
names this install's files alone. --keep FOLDER: the earlier install's files under FOLDER are
kept as they are and named again (the bundled plugin, when this install has none to put).
Prints how many files it put.

uninstall: removes the files the manifest names, then each folder the record names once it is
empty, deepest first; nothing else, so a file of the user's own in one of those folders (a
plugin put beside the bundled one) stays, and so does its folder. Prints what it removed.

--legacy PLACE: a file or a folder an install made before there was a manifest put, a folder
with everything in it; --legacy-folder FOLDER: a folder such an install made, though not
everything in it is its own (the bundled plugin's, where the user's plugins go too). Without a
manifest, those are the earlier install's files and folders.

Exit status: 0 done; 1 a file could not be put or removed; 2 the command line, a source or a
record is wrong.

Standard library only (Python 3.11 or later).
"""

import argparse
import errno
import os
import shutil
import stat
import sys

RECORD = "lib/dawsome/install-record"
MANIFEST = "lib/dawsome/install-manifest"
PREFIX = "."


class Wrong(Exception):
    """The command line, a source or a record is wrong: status 2."""


def place_of(text, what):
    """`text`, a path below the prefix, with its `.` and empty parts dropped (`.` for the prefix
    itself); Wrong when it could lead outside the prefix or holds a line break (the records are
    a line each)."""
    if "\n" in text or "\r" in text:
        raise Wrong(f"{what} holds a line break: {text!r}")
    parts = [part for part in text.split("/") if part not in ("", ".")]
    if text.startswith("/") or "\\" in text or ".." in parts:
        raise Wrong(f"{what} is not a path below the prefix: {text!r}")
    return "/".join(parts) or PREFIX


def path_of(prefix, place):
    return prefix if place == PREFIX else os.path.join(prefix, *place.split("/"))


def parents(place):
    """The folders holding `place`, the prefix first."""
    parts = place.split("/")
    return [PREFIX] + ["/".join(parts[:end]) for end in range(1, len(parts))]


def depth(place):
    return 0 if place == PREFIX else place.count("/") + 1


def read_list(prefix, place):
    """The places a record of the install's lists, or None when there is none. An entry that
    could lead outside the prefix is left out and said so: a damaged record can then only cost a
    file or a folder left behind."""
    path = path_of(prefix, place)
    try:
        with open(path, encoding="utf-8") as file:
            lines = file.read().splitlines()
    except FileNotFoundError:
        return None
    listed = []
    for line in lines:
        if not line or line.startswith("#"):
            continue
        try:
            listed.append(place_of(line, path))
        except Wrong as error:
            print(f"install-files: {error}, ignored", file=sys.stderr)
    return listed


def write_list(prefix, place, header, places):
    """Writes `places` a line each under `header` to the record at `place`, beside it and
    renamed into it."""
    path = path_of(prefix, place)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    temp = f"{path}.{os.getpid()}.tmp"
    with open(temp, "w", encoding="utf-8") as file:
        file.write("".join(f"# {line}\n" for line in header))
        file.write("".join(f"{place}\n" for place in places))
    os.chmod(temp, 0o644)
    os.replace(temp, path)


def write_record(prefix, folders):
    write_list(prefix, RECORD, ["The folders scripts/install.sh made under this prefix, relative",
                                "to it; the uninstall removes each once it is empty."], folders)


def write_manifest(prefix, files):
    write_list(prefix, MANIFEST, ["The files scripts/install.sh put under this prefix, relative",
                                  "to it; the uninstall removes these and nothing else."], files)


def is_folder(path):
    """A folder itself, not a link to one."""
    return os.path.isdir(path) and not os.path.islink(path)


def walk(top, place):
    """The files under the folder `top` as (path, place below the prefix), and its folders'
    places, `place` being where `top` goes. Wrong when it holds anything but regular files and
    folders."""
    files, folders = [], [place]
    for here, names, file_names in os.walk(top):
        below = os.path.relpath(here, top)
        at = place if below == os.curdir else f"{place}/{below.replace(os.sep, '/')}"
        for name in names:
            if os.path.islink(os.path.join(here, name)):
                raise Wrong(f"{os.path.join(here, name)} is a link, not a folder")
            folders.append(place_of(f"{at}/{name}", top))
        for name in file_names:
            path = os.path.join(here, name)
            if not stat.S_ISREG(os.lstat(path).st_mode):
                raise Wrong(f"{path} is not a regular file")
            files.append((path, place_of(f"{at}/{name}", top)))
    return files, folders


def legacy_install(prefix, legacy, legacy_folders):
    """The files and folders of an install made before there was a manifest: each legacy place
    there, a folder with everything in it, and the legacy folders."""
    files, folders = [], list(legacy_folders)
    for place in legacy:
        path = path_of(prefix, place)
        if is_folder(path):
            found, found_folders = walk(path, place)
            files += [at for _, at in found]
            folders += found_folders
        elif os.path.lexists(path):
            files.append(place)
    return files, folders


def remove_emptied(prefix, folders):
    """Removes each of `folders` that is empty, deepest first; returns (removed, kept), kept
    being those still there."""
    removed, kept = [], []
    for folder in sorted(set(folders), key=depth, reverse=True):
        path = path_of(prefix, folder)
        if not os.path.lexists(path):
            continue
        if is_folder(path) and not os.listdir(path):
            os.rmdir(path)
            removed.append(folder)
        else:
            kept.append(folder)
    return removed, kept


def put(source, path, mode):
    """Copies `source` beside `path` under a hidden name and renames it into place."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    temp = os.path.join(os.path.dirname(path), f".{os.path.basename(path)}.{os.getpid()}.tmp")
    try:
        shutil.copyfile(source, temp)
        os.chmod(temp, mode)
        os.replace(temp, path)
    finally:
        if os.path.lexists(temp):
            os.remove(temp)


def remove_file(path):
    """Removes the file (or link) at `path`; False when there is none, or a folder is there."""
    if not os.path.lexists(path) or is_folder(path):
        return False
    os.remove(path)
    return True


def install(prefix, puts, keep, legacy, legacy_folders):
    earlier = read_list(prefix, MANIFEST)
    earlier_folders = []
    if earlier is None:
        earlier, earlier_folders = legacy_install(prefix, legacy, legacy_folders)
    places = [place for _, place, _ in puts]
    placed = set(places)
    if len(placed) != len(places):
        raise Wrong("two files are to be put at one place")
    if {MANIFEST, RECORD} & placed:
        raise Wrong("a file is to be put where the install keeps its records")
    kept = [place for place in earlier
            if place not in placed and any(place.startswith(f"{folder}/") for folder in keep)
            and os.path.lexists(path_of(prefix, place))]
    files = places + kept + [MANIFEST, RECORD]
    # The record first, so an install that stops halfway has already named the folders it may
    # have made; then the manifest, naming what the earlier install and this one put.
    made = list(dict.fromkeys((read_list(prefix, RECORD) or []) + earlier_folders))
    known = set(made)
    for place in files:
        for folder in parents(place):
            if folder not in known and not os.path.lexists(path_of(prefix, folder)):
                made.append(folder)
                known.add(folder)
    write_record(prefix, made)
    write_manifest(prefix, list(dict.fromkeys(earlier + files)))
    for source, place, mode in puts:
        put(source, path_of(prefix, place), mode)
    listed = set(files)
    for place in earlier:
        if place not in listed:
            remove_file(path_of(prefix, place))
    remove_emptied(prefix, made)
    write_record(prefix, [folder for folder in made
                          if os.path.lexists(path_of(prefix, folder))])
    write_manifest(prefix, files)
    print(len(puts))


def uninstall(prefix, legacy, legacy_folders):
    files = read_list(prefix, MANIFEST)
    folders = read_list(prefix, RECORD)
    earlier_folders = []
    if files is None:
        files, earlier_folders = legacy_install(prefix, legacy, legacy_folders)
    removed = sum(remove_file(path_of(prefix, place)) for place in [*files, MANIFEST, RECORD])
    emptied, kept = remove_emptied(prefix, (folders or []) + earlier_folders)
    print(f"removed {removed} files from {prefix}")
    if folders is None and not earlier_folders:
        print("no record of the folders the install made, so none were removed")
    else:
        print(f"removed {len(emptied)} of the folders the install made")
    if kept:
        print(f"kept, since something else is in them: {' '.join(sorted(kept))}")


def puts_of(args):
    """The files to put, as (source, place, mode), from --file and --tree in their order."""
    puts = []
    for kind, values in args.put or []:
        if kind == "file":
            mode, source, place = values
            try:
                mode = int(mode, 8)
            except ValueError:
                raise Wrong(f"--file {mode}: not a mode in octal") from None
            if not stat.S_ISREG(os.stat(source).st_mode):
                raise Wrong(f"{source} is not a regular file")
            puts.append((source, place_of(place, "--file"), mode))
        else:
            source, place = values
            if not is_folder(source):
                raise Wrong(f"{source} is not a folder")
            found, _ = walk(source, place_of(place, "--tree"))
            for path, at in found:
                executable = os.stat(path).st_mode & 0o111
                puts.append((path, at, 0o755 if executable else 0o644))
    return puts


class Put(argparse.Action):
    """--file and --tree, kept in one list in the order given."""

    def __call__(self, parser, namespace, values, option_string=None):
        listed = getattr(namespace, self.dest) or []
        listed.append((option_string.removeprefix("--"), values))
        setattr(namespace, self.dest, listed)


def main(argv):
    parser = argparse.ArgumentParser(prog="install-files.py",
                                     description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("install", "uninstall"):
        command = commands.add_parser(name)
        command.add_argument("--prefix", required=True)
        command.add_argument("--legacy", action="append", default=[], metavar="PLACE")
        command.add_argument("--legacy-folder", action="append", default=[], metavar="FOLDER")
        if name == "install":
            command.add_argument("--keep", action="append", default=[], metavar="FOLDER")
            command.add_argument("--file", nargs=3, action=Put, dest="put",
                                 metavar=("MODE", "SOURCE", "PLACE"))
            command.add_argument("--tree", nargs=2, action=Put, dest="put",
                                 metavar=("SOURCE", "PLACE"))
    try:
        args = parser.parse_args(argv)
    except SystemExit as exit_:
        return int(exit_.code or 0)
    prefix = os.path.abspath(args.prefix)
    try:
        legacy = [place_of(place, "--legacy") for place in args.legacy]
        legacy_folders = [place_of(folder, "--legacy-folder") for folder in args.legacy_folder]
        if args.command == "uninstall":
            uninstall(prefix, legacy, legacy_folders)
        else:
            keep = [place_of(folder, "--keep") for folder in args.keep]
            install(prefix, puts_of(args), keep, legacy, legacy_folders)
    except Wrong as error:
        print(f"install-files: {error}", file=sys.stderr)
        return 2
    except OSError as error:
        if error.errno == errno.ENOENT and error.filename and not error.filename.startswith(prefix):
            print(f"install-files: {error.filename} is not there", file=sys.stderr)
            return 2
        print(f"install-files: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
