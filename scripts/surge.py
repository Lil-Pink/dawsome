#!/usr/bin/env python3
"""Fetches Surge XT, the CLAP plugin the programs bundle, as locks/surge.lock pins it, and writes the
sources text the release zips carry.

usage: python3 -I scripts/surge.py fetch [--lock FILE] [--dir DIR] [--offline | --if-fetched]
                                         [--source] [--vst3] [PLATFORM...]
       python3 -I scripts/surge.py sources [--lock FILE] PLATFORM
       (scripts/fetch-surge.sh runs `fetch` with DIR = <target dir>/surge)

fetch PLATFORM (linux-x86_64, windows-x86_64): makes DIR/PLATFORM hold `Surge XT.clap` from the
platform's plugins asset and the factory data from the data asset as `SurgeXTData/` (Surge
looks for a folder of that name beside its plugin: src/common/SurgeStorage.cpp of
release_xt_1.3.4, lines 208-229 on Linux and 273-292 on Windows), the `patches_3rdparty` and
`wavetables_3rdparty` folders (other authors' patches and wavetables) and `skins/Tutorials`
(tutorial skins, whose fonts are under the OFL) left out (Surge skips a patch or wavetable
folder that is not there: the same file, 767-770, 964 and 1129-1135). Prints the folder, a line
each. --source: the source asset, its download's path printed. --vst3: DIR/PLATFORM-vst3 instead,
for the VST3 host's tests and never installed or packaged: the platform's two VST3 bundles,
`Surge XT.vst3` and `Surge XT Effects.vst3`, each with its binary alone at its place in the
bundle, and the same factory data as `SurgeXTData/` beside them (Surge walks up from its binary
to the first folder of that name, the lines named above), made, stamped and kept as a
platform's folder is.

Each asset is downloaded from the lock's url into DIR/downloads/<file>/, a folder of its own,
through a partial file of a name of its own, renamed into place only when its size and SHA-256
are the ones the lock pins, so two fetches at once never put a partial file in place; a file
already there that is not the one pinned is refused and left for a look, never downloaded over.
Nothing downloaded is run: the archives are read here, and only regular files and folders named
without `..`, a root or a drive are taken from them; an archive holding anything else (a link, a
device, a sparse file, a name that could climb out), or whose files add up to more than 1 GiB or
hold one of more than 256 MiB, is refused whole. Each platform's folder is made beside itself
under a hidden name and moved into place, with a stamp DIR/PLATFORM.stamp naming the pins and
the rules it was made by and the SHA-256 of every file in it (DIR/PLATFORM-vst3.stamp for the
VST3 folder): a folder is current, and not made again, only while its stamp is the lock's and
the rules' and its files are the ones the stamp records, no more and no fewer; otherwise it is
made again from the downloads. --offline downloads nothing and gives only what is fetched;
--if-fetched does the same, but what is not fetched is left out of what it prints rather than
status 3.

sources PLATFORM: the text of SOURCES.md, which the platform's release zip carries: where the
bundled files come from and the complete source of the release, by name, size, SHA-256 and
place.

Exit status: 0 done; 1 a file was refused or could not be fetched; 2 the command line or the
lock is wrong; 3 (--offline) not fetched.

Standard library only (Python 3.11 or later).
"""

import argparse
import hashlib
import json
import os
import shutil
import stat
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLATFORMS = ("linux-x86_64", "windows-x86_64")
PLUGIN = "Surge XT.clap"
# What fetch --vst3 takes from each platform's plugins asset, for the VST3 host's tests: each
# bundle's binary for that platform, at its place in the bundle. The Windows bundles' desktop.ini
# and Plugin.ico (Explorer's icon for the folder) are left out: a host reads neither.
VST3_PLUGINS = {
    "linux-x86_64": ("Surge XT.vst3/Contents/x86_64-linux/Surge XT.so",
                     "Surge XT Effects.vst3/Contents/x86_64-linux/Surge XT Effects.so"),
    "windows-x86_64": ("Surge XT.vst3/Contents/x86_64-win/Surge XT.vst3",
                       "Surge XT Effects.vst3/Contents/x86_64-win/Surge XT Effects.vst3"),
}
DATA = "SurgeXTData"
# Where the factory data lies in the data asset, and what of it is left out, as parts below
# SurgeXTData: the third-party patches and wavetables (other authors' licenses), and the
# tutorial skins, which carry fonts under the OFL (found
# 2026-10-22).
DATA_PREFIX = ("Surge Synth Team", DATA)
LEFT_OUT = (("patches_3rdparty",), ("wavetables_3rdparty",), ("skins", "Tutorials"))
# Raised when extract() changes what it takes in a way the constants above do not show: a
# platform's folder made under other rules is made again (its stamp records them).
RULES = 2
# The most an archive's files may add up to, and the most one of them may hold, as the archive
# states them: the release's largest expands to 367 MiB, its largest file 32 MiB.
MAX_TOTAL = 1 << 30
MAX_MEMBER = 256 << 20
CHUNK = 1 << 20
TIMEOUT = 60


class Refused(Exception):
    """A file is not the one pinned, cannot be fetched, or is not an archive that is safe to
    read: status 1."""


class NotFetched(Exception):
    """--offline, and what is asked for is not fetched: status 3."""


class Lock:
    """locks/surge.lock: the version, the url the assets are downloaded from, and each asset's file,
    size and SHA-256. A lock of the same form may give an asset a url of its own
    (`url.<what>=`; locks/vst3-test.lock, whose assets come from three projects), and then needs no
    url of its own for that one."""

    def __init__(self, path):
        values = {}
        with open(path, encoding="utf-8") as file:
            for number, line in enumerate(file, 1):
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                key, sep, value = line.partition("=")
                if not sep:
                    raise ValueError(f"{path}:{number}: not key=value: {line}")
                values[key] = value
        self.path = path
        self.name = os.path.basename(path)
        self.version = values.get("version", "")
        self.url = values.get("url", "").rstrip("/")
        self.urls = {key.removeprefix("url."): value.rstrip("/")
                     for key, value in values.items() if key.startswith("url.")}
        self.assets = {}
        for key, value in values.items():
            if not key.startswith("asset."):
                continue
            parts = value.split()
            if len(parts) != 3 or not parts[1].isdigit() or len(parts[2]) != 64:
                raise ValueError(f"{path}: {key} is not <file> <bytes> <SHA-256>")
            name, size, digest = parts
            if os.path.basename(name) != name or name in ("", ".", ".."):
                raise ValueError(f"{path}: {key} names no plain file: {name}")
            self.assets[key.removeprefix("asset.")] = (name, int(size), digest.lower())
        if not self.version or not all(map(self.url_of, self.assets)):
            raise ValueError(f"{path} needs a version and a url")

    def url_of(self, what):
        """The url the asset `what` is downloaded from, its file name after a `/`."""
        return self.urls.get(what, self.url)

    def asset(self, what):
        try:
            return self.assets[what]
        except KeyError:
            raise ValueError(f"{self.path} pins no asset.{what}") from None


def digest_of(path):
    """The size and SHA-256 of the file at `path`."""
    sha = hashlib.sha256()
    size = 0
    with open(path, "rb") as file:
        while block := file.read(CHUNK):
            sha.update(block)
            size += len(block)
    return size, sha.hexdigest()


def download(lock, what, folder, offline):
    """The path of asset `what`, downloaded into DIR/downloads/<file>/ and held to its pin."""
    name, size, digest = lock.asset(what)
    place = os.path.join(folder, "downloads", name)
    path = os.path.join(place, name)
    if os.path.isfile(path):
        found = digest_of(path)
        if found != (size, digest):
            raise Refused(f"{path} is not the file {lock.name} pins ({size} bytes, SHA-256 "
                          f"{digest}; it has {found[0]} bytes, SHA-256 {found[1]}); delete it to "
                          "download it again")
        return path
    if offline:
        raise NotFetched(f"{name} is not downloaded")
    os.makedirs(place, exist_ok=True)
    # A name of this download's own: another fetch of the same asset at the same time writes
    # its own, so neither can rename the other's partial file into place.
    handle, partial = tempfile.mkstemp(prefix=f".{name}.", suffix=".part", dir=place)
    url = f"{lock.url_of(what)}/{name}"
    sha = hashlib.sha256()
    got = 0
    try:
        with os.fdopen(handle, "wb") as file, \
                urllib.request.urlopen(url, timeout=TIMEOUT) as response:
            while block := response.read(CHUNK):
                got += len(block)
                if got > size:
                    break
                sha.update(block)
                file.write(block)
        if (got, sha.hexdigest()) != (size, digest):
            raise Refused(f"{url} is not the file {lock.name} pins ({size} bytes, SHA-256 "
                          f"{digest}); nothing of it was kept")
        os.replace(partial, path)
    except OSError as error:
        raise Refused(f"{url} could not be downloaded: {error}") from None
    finally:
        if os.path.exists(partial):
            os.remove(partial)
    return path


def plain_parts(name, archive):
    """The parts of member `name` of `archive`, `.` parts dropped; Refused when the name could
    lead anywhere but below the folder it is extracted into."""
    if "\\" in name or name.startswith("/") or (len(name) > 1 and name[1] == ":"):
        raise Refused(f"{archive}: a member names a place outside its folder: {name!r}")
    parts = [part for part in name.split("/") if part not in ("", ".")]
    if ".." in parts:
        raise Refused(f"{archive}: a member climbs out of its folder: {name!r}")
    return parts


class Members:
    """An archive's regular files by their parts, in the archive's order (read in that order, a
    compressed archive is expanded once more, not once per file), its folders checked too;
    Refused when it holds anything else or a name that could lead outside. A context manager:
    the archive is closed on leaving, and when it is refused. With `others` true, a member that
    is neither (a link, a device) is listed in `others` by its parts rather than refused, for a
    caller that takes none of them (scripts/vst3_test_plugins.py: DPF-Plugins' archive holds
    hard links among the presets of a plugin it does not take); a name that could lead outside
    is refused all the same."""

    def __init__(self, path, others=False):
        self.path = path
        self.name = os.path.basename(path)
        self.files = {}
        self.others = [] if others else None
        try:
            if zipfile.is_zipfile(path):
                self.kind = "zip"
                self.archive = zipfile.ZipFile(path)
            else:
                self.kind = "tar"
                self.archive = tarfile.open(path, "r:*")
        except (zipfile.BadZipFile, tarfile.TarError, OSError, EOFError) as error:
            raise Refused(f"{self.name} is not an archive that can be read: {error}") from None
        try:
            self.list()
        except BaseException:
            self.archive.close()
            raise

    def list(self):
        if self.kind == "zip":
            for info in self.archive.infolist():
                parts = plain_parts(info.filename, self.name)
                mode = info.external_attr >> 16
                if info.flag_bits & 0x1:
                    raise Refused(f"{self.name}: {info.filename!r} is encrypted")
                if stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR):
                    self.other(parts, info.filename)
                elif not info.is_dir():
                    self.files[tuple(parts)] = info
        else:
            try:
                members = self.archive.getmembers()
            except (tarfile.TarError, OSError, EOFError) as error:
                raise Refused(f"{self.name} is not an archive that can be read: {error}") from None
            for member in members:
                parts = plain_parts(member.name, self.name)
                if not (member.isfile() or member.isdir()):
                    self.other(parts, member.name)
                    continue
                if member.issparse():
                    raise Refused(f"{self.name}: {member.name!r} is a sparse file")
                if member.isfile():
                    self.files[tuple(parts)] = member
        total = 0
        for parts in self.files:
            size = self.size(parts)
            if size > MAX_MEMBER:
                raise Refused(f"{self.name}: {'/'.join(parts)} would expand to {size} bytes, more "
                              f"than {MAX_MEMBER}")
            total += size
        if total > MAX_TOTAL:
            raise Refused(f"{self.name} would expand to {total} bytes, more than {MAX_TOTAL}")

    def other(self, parts, name):
        """A member that is not a file or a folder: listed, or the archive refused."""
        if self.others is None:
            raise Refused(f"{self.name}: {name!r} is not a file or a folder")
        self.others.append(tuple(parts))

    def size(self, parts):
        """The size the archive states of its file at `parts`."""
        member = self.files[parts]
        return member.file_size if self.kind == "zip" else member.size

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.archive.close()

    def copy(self, parts, target, mode):
        """Writes the member at `parts` to `target`, its size held to the archive's word;
        returns its SHA-256."""
        member = self.files[parts]
        size = self.size(parts)
        if self.kind == "zip":
            source = self.archive.open(member)
        else:
            source = self.archive.extractfile(member)
        sha = hashlib.sha256()
        written = 0
        with source, open(target, "wb") as file:
            while block := source.read(CHUNK):
                written += len(block)
                if written > size:
                    break
                sha.update(block)
                file.write(block)
        if written != size:
            raise Refused(f"{self.name}: {'/'.join(parts)} does not hold the {size} bytes it "
                          "states")
        os.chmod(target, mode)
        return sha.hexdigest()


def inside(folder, parts):
    """`folder` joined with `parts`, Refused when that is not inside `folder`."""
    target = os.path.join(folder, *parts)
    base = os.path.abspath(folder)
    if os.path.commonpath([base, os.path.abspath(target)]) != base:
        raise Refused(f"{'/'.join(parts)} would be written outside {folder}")
    return target


def left_out(rest):
    """Whether the data file at `rest` (its parts below SurgeXTData) is left out."""
    return any(rest[:len(parts)] == parts for parts in LEFT_OUT)


def extract(plugins, data, staged, taken=(PLUGIN,)):
    """The files `taken` (their paths in the archive, `/` between the parts: Surge XT.clap
    unless told otherwise) from the archive `plugins`, each at that path, and the factory data
    from the archive `data`, into `staged`; returns the SHA-256 of each file written, by its
    path below `staged` (`/` between the parts)."""
    written = {}
    with Members(plugins) as archive:
        for name in taken:
            parts = tuple(name.split("/"))
            if parts not in archive.files:
                raise Refused(f"{archive.name} holds no {name}")
            target = inside(staged, parts)
            os.makedirs(os.path.dirname(target), exist_ok=True)
            written[name] = archive.copy(parts, target, 0o755)
    with Members(data) as archive:
        for parts in archive.files:
            if parts[:len(DATA_PREFIX)] != DATA_PREFIX:
                continue
            rest = parts[len(DATA_PREFIX):]
            if not rest or left_out(rest):
                continue
            target = inside(staged, [DATA, *rest])
            os.makedirs(os.path.dirname(target), exist_ok=True)
            written["/".join((DATA, *rest))] = archive.copy(parts, target, 0o644)
        if len(written) == len(taken):
            raise Refused(f"{archive.name} holds no {'/'.join(DATA_PREFIX)} files")
    return written


def made_by(lock, platform, vst3=False):
    """What a platform's stamp records besides its files: the version, the two assets it is
    made from and the rules it is made by (for its VST3 folder, the bundles' binaries taken)."""
    rules = {"version": RULES, "plugin": PLUGIN, "data": "/".join(DATA_PREFIX),
             "left out": ["/".join(parts) for parts in LEFT_OUT]}
    if vst3:
        del rules["plugin"]
        rules["vst3"] = list(VST3_PLUGINS[platform])
    return {
        "version": lock.version,
        "assets": {what: list(lock.asset(what)) for what in (platform, "data")},
        "rules": rules,
    }


def tree_digests(top):
    """The SHA-256 of every file under `top`, by its path below it (`/` between the parts);
    None when `top` holds anything but regular files and folders."""
    found = {}
    for here, folders, files in os.walk(top):
        for name in folders + files:
            path = os.path.join(here, name)
            mode = os.lstat(path).st_mode
            if stat.S_ISDIR(mode):
                continue
            if not stat.S_ISREG(mode):
                return None
            found[os.path.relpath(path, top).replace(os.sep, "/")] = digest_of(path)[1]
    return found


def is_current(target, stamp, made):
    """Whether the folder `target` is the one its stamp records, made as `made` says: every
    file the stamp names there with its SHA-256, and no other."""
    try:
        with open(stamp, encoding="utf-8") as file:
            recorded = json.load(file)
    except (OSError, ValueError):
        return False
    if not isinstance(recorded, dict) or not os.path.isdir(target):
        return False
    files = recorded.pop("files", None)
    return recorded == made and isinstance(files, dict) and tree_digests(target) == files


def platform_folder(lock, platform, folder, offline, vst3=False):
    """DIR/PLATFORM (with `vst3`, DIR/PLATFORM-vst3), current with the lock and the rules: made
    from the downloads when it is not."""
    name = f"{platform}-vst3" if vst3 else platform
    target = os.path.join(folder, name)
    stamp = target + ".stamp"
    made = made_by(lock, platform, vst3)
    if is_current(target, stamp, made):
        return target
    if os.path.exists(target):
        print(f"surge: {target} is not the folder its stamp records (a file changed, or it was "
              "made under other rules): it is made again", file=sys.stderr)
    plugins = download(lock, platform, folder, offline)
    data = download(lock, "data", folder, offline)
    staged = os.path.join(folder, f".{name}.{os.getpid()}.tmp")
    shutil.rmtree(staged, ignore_errors=True)
    os.makedirs(staged)
    try:
        files = extract(plugins, data, staged, VST3_PLUGINS[platform] if vst3 else (PLUGIN,))
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


def sources_text(lock, platform):
    """SOURCES.md of the release zip of `platform`."""
    source, source_size, source_digest = lock.asset("source")
    lines = [
        "# Sources of the bundled plugin",
        "",
        f"`plugins/` (beside `licenses/`, where this file is) holds Surge XT {lock.version}, made by the Surge Synth Team: `{PLUGIN}`, the",
        f"plugin, and `{DATA}/`, its factory data (the third-party patches and wavetables and the",
        "tutorial skins left out). Surge XT is free software under the GNU General Public License,",
        "version 3 or later;",
        "`LICENSE-SURGE` holds its notice and the license. These files are the release's own,",
        "unchanged, taken from these of its assets:",
        "",
    ]
    for what in (platform, "data"):
        name, size, digest = lock.asset(what)
        lines.append(f"- `{name}`, {size} bytes, SHA-256 `{digest}`")
    lines += [
        "",
        "The complete source of that release, from which these files are built:",
        "",
        f"- `{source}`, {source_size} bytes, SHA-256 `{source_digest}`, published with every",
        "  release of Dawsome that bundles the plugin, beside its zips, and by the Surge Synth",
        f"  Team at {lock.url}/{source}",
        "",
    ]
    return "\n".join(lines)


def main(argv):
    parser = argparse.ArgumentParser(prog="surge.py", description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    fetch = commands.add_parser("fetch")
    fetch.add_argument("--lock", default=os.path.join(ROOT, "locks", "surge.lock"))
    fetch.add_argument("--dir", required=True)
    fetch_mode = fetch.add_mutually_exclusive_group()
    fetch_mode.add_argument("--offline", action="store_true")
    fetch_mode.add_argument("--if-fetched", action="store_true")
    fetch.add_argument("--source", action="store_true")
    fetch.add_argument("--vst3", action="store_true")
    fetch.add_argument("platforms", nargs="*", choices=PLATFORMS, metavar="PLATFORM")
    sources = commands.add_parser("sources")
    sources.add_argument("--lock", default=os.path.join(ROOT, "locks", "surge.lock"))
    sources.add_argument("platform", choices=PLATFORMS, metavar="PLATFORM")
    try:
        args = parser.parse_args(argv)
    except SystemExit as exit_:
        return int(exit_.code or 0)
    try:
        lock = Lock(args.lock)
        if args.command == "sources":
            sys.stdout.write(sources_text(lock, args.platform))
            return 0
        if not args.platforms and not args.source:
            print("surge: fetch needs a PLATFORM or --source", file=sys.stderr)
            return 2
        folder = os.path.abspath(args.dir)
        offline = args.offline or args.if_fetched
        wanted = [lambda platform=platform: platform_folder(lock, platform, folder, offline,
                                                            args.vst3)
                  for platform in args.platforms]
        if args.source:
            wanted.append(lambda: download(lock, "source", folder, offline))
        found = []
        for fetched in wanted:
            try:
                found.append(fetched())
            except NotFetched:
                if not args.if_fetched:
                    raise
    except (OSError, ValueError) as error:
        print(f"surge: {error}", file=sys.stderr)
        return 2
    except Refused as error:
        print(f"surge: refused: {error}", file=sys.stderr)
        return 1
    except NotFetched as error:
        print(f"surge: not fetched ({error}): scripts/fetch-surge.sh fetches it", file=sys.stderr)
        return 3
    if found:
        print("\n".join(found))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
