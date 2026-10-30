"""Tests of scripts/surge.py, the fetch of the bundled Surge XT, against what it must do by its
pins (locks/surge.lock's rules): each asset downloaded into a folder of its own and kept only when its
size and SHA-256 are the ones pinned, a changed file refused (downloaded or already there), two
fetches at once each landing the whole file, the CLAP plugin and the factory data taken and
nothing else (the third-party patches and wavetables and the tutorial skins left out, the folder
named SurgeXTData), an archive whose members could write outside the folder, or that expands
past its caps, refused whole, and a platform's folder made again when a file in it changed or
the rules it was made by did. The VST3 folder the VST3 host's tests load (fetch --vst3): both
bundles' binaries at their places and the same factory data, nothing else, under a stamp of its
own that leaves the CLAP folder as it is.

The releases here are small archives made by the tests, in a temporary folder, served through
file: URLs; nothing is downloaded from the network and nothing outside the temporary folder is
written.

Run (from the repository root; scripts/check.sh runs it):
  python3 -I -B tests/scripts/fetch_surge_test.py
"""
import contextlib
import gzip
import hashlib
import importlib.util
import io
import os
import sys
import json
import tarfile
import tempfile
import unittest
import zipfile
from unittest import mock

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def load_surge():
    """scripts/surge.py as a module, read from its path (no folder of it on sys.path)."""
    spec = importlib.util.spec_from_file_location("surge", os.path.join(ROOT, "scripts", "surge.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


surge = load_surge()

CLAP = b"\x7fELF the plugin"
WIN_CLAP = b"MZ the plugin"
FACTORY = {
    "patches_factory/Leads/Lead.fxp": b"factory patch",
    "wavetables/Basic/Saw.wt": b"factory wavetable",
    "skins/dark/skin.xml": b"<skin/>",
    "tuning_library/a.scl": b"! a scale",
    "WHERE TO PLACE USER DATA.txt": b"read me",
}
THIRD_PARTY = {
    "patches_3rdparty/Someone/Other.fxp": b"third-party patch",
    "wavetables_3rdparty/Someone/Other.wt": b"third-party wavetable",
}
DATA = "./Surge Synth Team/SurgeXTData/"


def tar_gz(members):
    """A .tar.gz of `members`: (name, bytes) for a file, (name, None) for a folder, (name,
    ("link", target)) for a symbolic link. The same members give the same bytes at any time
    (every time stamp 0), so a test can pin them twice."""
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as packed, \
            tarfile.open(fileobj=packed, mode="w") as archive:
        for name, data in members:
            info = tarfile.TarInfo(name)
            if data is None:
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                archive.addfile(info)
            elif isinstance(data, tuple):
                info.type = tarfile.SYMTYPE
                info.linkname = data[1]
                archive.addfile(info)
            else:
                info.size = len(data)
                info.mode = 0o755
                archive.addfile(info, io.BytesIO(data))
    return out.getvalue()


def zipped(members):
    """A .zip of `members`, (name, bytes) each, the same bytes at any time."""
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as archive:
        for name, data in members:
            archive.writestr(zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0)), data)
    return out.getvalue()


def linux_plugins(clap=CLAP):
    return tar_gz([(".", None), ("./Surge XT.clap", clap), ("./Surge XT Effects.clap", b"fx"),
                   ("./Surge XT.vst3", None), ("./Surge XT.vst3/x.so", b"vst3")])


def windows_plugins():
    return zipped([("Surge XT.clap", WIN_CLAP), ("Surge XT Effects.clap", b"fx"),
                   ("Surge XT.exe", b"standalone")])


def data():
    files = {**FACTORY, **THIRD_PARTY}
    return tar_gz([(".", None), ("./README.txt", b"portable"), ("./Surge Synth Team", None)]
                  + [(DATA + name, content) for name, content in files.items()])


class Release:
    """A release folder served through a file: URL, a lock pinning what it holds, and the folder
    the fetch works in, all under one temporary folder."""

    def __init__(self, test, assets):
        self.temp = tempfile.TemporaryDirectory(prefix="dawsome-surge-")
        test.addCleanup(self.temp.cleanup)
        self.root = self.temp.name
        self.release = os.path.join(self.root, "release")
        self.dir = os.path.join(self.root, "surge")
        os.makedirs(self.release)
        lines = ["version=9.9.9", "url=" + self.uri()]
        for what, (name, content) in assets.items():
            self.serve(name, content)
            digest = hashlib.sha256(content).hexdigest()
            lines.append(f"asset.{what}={name} {len(content)} {digest}")
        self.lock = os.path.join(self.root, "surge.lock")
        with open(self.lock, "w", encoding="utf-8") as file:
            file.write("# a lock of the tests' own\n" + "\n".join(lines) + "\n")

    def uri(self):
        return "file://" + self.release.replace(os.sep, "/")

    def serve(self, name, content):
        with open(os.path.join(self.release, name), "wb") as file:
            file.write(content)

    def fetch(self, *args):
        """The exit status of `surge.py fetch` with this lock and folder, and its output."""
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = surge.main(["fetch", "--lock", self.lock, "--dir", self.dir, *args])
        return status, out.getvalue(), err.getvalue()

    def files(self, folder):
        """Every file under `folder` of the fetch's, relative to it, with its bytes."""
        found = {}
        base = os.path.join(self.dir, folder)
        for top, _, names in os.walk(base):
            for name in names:
                path = os.path.join(top, name)
                with open(path, "rb") as file:
                    found[os.path.relpath(path, base).replace(os.sep, "/")] = file.read()
        return found

    def downloaded(self, name):
        """The files in an asset's download folder."""
        folder = os.path.join(self.dir, "downloads", name)
        return sorted(os.listdir(folder)) if os.path.isdir(folder) else []


def standard(test, **changes):
    assets = {
        "linux-x86_64": ("plugins-linux.tar.gz", linux_plugins()),
        "windows-x86_64": ("plugins-win64.zip", windows_plugins()),
        "data": ("content.tar.gz", data()),
        "source": ("src.tar.gz", tar_gz([("./surge/LICENSE", b"GPL")])),
    }
    assets.update(changes)
    return Release(test, assets)


WANT_DATA = {"SurgeXTData/" + name: content for name, content in FACTORY.items()}


class Fetch(unittest.TestCase):
    # Each platform's folder holds its Surge XT.clap and the factory data as SurgeXTData/, and
    # nothing else: no effects plugin, no VST3, no standalone, no third-party folder.
    def test_the_plugin_and_the_factory_data_alone(self):
        release = standard(self)
        status, out, err = release.fetch("linux-x86_64", "windows-x86_64")
        self.assertEqual(status, 0, err)
        linux = os.path.join(release.dir, "linux-x86_64")
        windows = os.path.join(release.dir, "windows-x86_64")
        self.assertEqual(out.splitlines(), [linux, windows])
        self.assertEqual(release.files("linux-x86_64"), {"Surge XT.clap": CLAP, **WANT_DATA})
        self.assertEqual(release.files("windows-x86_64"), {"Surge XT.clap": WIN_CLAP, **WANT_DATA})
        if os.name == "posix":
            self.assertTrue(os.access(os.path.join(linux, "Surge XT.clap"), os.X_OK))
        self.assertEqual(release.downloaded("content.tar.gz"), ["content.tar.gz"])

    # A second fetch finds the platform's folder current and downloads nothing: --offline gives
    # it; with nothing fetched, --offline says so with status 3 and makes nothing.
    def test_offline_gives_what_is_fetched_and_nothing_else(self):
        release = standard(self)
        status, _, err = release.fetch("--offline", "linux-x86_64")
        self.assertEqual(status, 3, err)
        self.assertFalse(os.path.exists(os.path.join(release.dir, "linux-x86_64")))
        self.assertEqual(release.fetch("linux-x86_64")[0], 0)
        os.remove(os.path.join(release.release, "content.tar.gz"))
        status, out, err = release.fetch("--offline", "linux-x86_64")
        self.assertEqual((status, out), (0, os.path.join(release.dir, "linux-x86_64") + "\n"), err)

    # The release serves a file other than the one pinned: the fetch refuses it, keeps nothing
    # of it and extracts nothing.
    def test_a_changed_download_is_refused_and_not_kept(self):
        release = standard(self)
        release.serve("plugins-linux.tar.gz", linux_plugins(clap=b"\x7fELF another plugin"))
        status, _, err = release.fetch("linux-x86_64")
        self.assertEqual(status, 1)
        self.assertIn("plugins-linux.tar.gz", err)
        self.assertIn("SHA-256", err)
        self.assertEqual(release.downloaded("plugins-linux.tar.gz"), [])
        self.assertFalse(os.path.exists(os.path.join(release.dir, "linux-x86_64")))

    # A file already downloaded that changed since is refused, not replaced: the fetch says
    # which and leaves it for a look.
    def test_a_changed_file_already_downloaded_is_refused(self):
        release = standard(self)
        self.assertEqual(release.fetch("linux-x86_64")[0], 0)
        kept = os.path.join(release.dir, "downloads", "content.tar.gz", "content.tar.gz")
        with open(kept, "r+b") as file:
            file.seek(10)
            file.write(b"\0")
        stamp = os.path.join(release.dir, "linux-x86_64.stamp")
        os.remove(stamp)
        status, _, err = release.fetch("linux-x86_64")
        self.assertEqual(status, 1)
        self.assertIn(kept, err)
        self.assertTrue(os.path.exists(kept))
        self.assertFalse(os.path.exists(stamp))

    # The source: fetched only when asked for, and its download's path printed.
    # A PLATFORM the script does not know is refused, saying which, and nothing is fetched.
    def test_an_unknown_platform_is_refused(self):
        release = standard(self)
        status, _, err = release.fetch("nosuch-x86_64")
        self.assertEqual(status, 2)
        self.assertIn("'nosuch-x86_64'", err)
        self.assertFalse(os.path.exists(release.dir))

    def test_the_source_is_fetched_when_asked_for(self):
        release = standard(self)
        status, out, err = release.fetch("--source")
        self.assertEqual(status, 0, err)
        path = os.path.join(release.dir, "downloads", "src.tar.gz", "src.tar.gz")
        self.assertEqual(out, path + "\n")
        self.assertEqual(release.downloaded("plugins-linux.tar.gz"), [])

    # An archive (pinned as it is) whose members climb out with `..`, start at the root or
    # link elsewhere is refused whole: nothing of it is extracted.
    def test_an_archive_that_could_write_outside_is_refused(self):
        bad = [
            [(DATA + "../../../escape.txt", b"x")],
            [("/abs/Surge XT.clap", b"x")],
            [(DATA + "patches_factory/link.fxp", ("link", "/etc/passwd"))],
        ]
        for members in bad:
            with self.subTest(members=members[0][0]):
                content = tar_gz([(DATA + "wavetables/ok.wt", b"ok")] + members)
                release = standard(self, data=("content.tar.gz", content))
                status, _, err = release.fetch("linux-x86_64")
                self.assertEqual(status, 1)
                self.assertIn("content.tar.gz", err)
                self.assertFalse(os.path.exists(os.path.join(release.dir, "linux-x86_64")))
                self.assertFalse(os.path.exists(os.path.join(release.root, "escape.txt")))

    # A plugin archive without the plugin is refused, naming what it lacks.
    def test_an_archive_without_the_plugin_is_refused(self):
        release = standard(self, **{"linux-x86_64": ("plugins-linux.tar.gz",
                                                     tar_gz([("./other.clap", b"x")]))})
        status, _, err = release.fetch("linux-x86_64")
        self.assertEqual(status, 1)
        self.assertIn("Surge XT.clap", err)

    # The sources text a platform's zip carries names the source's file, size, SHA-256 and
    # where it is, and the two assets that zip's files come from, not the other platform's.
    def test_the_sources_text_names_the_pinned_source(self):
        release = standard(self)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            status = surge.main(["sources", "--lock", release.lock, "windows-x86_64"])
        self.assertEqual(status, 0)
        source = tar_gz([("./surge/LICENSE", b"GPL")])
        text = out.getvalue()
        self.assertIn("src.tar.gz", text)
        self.assertIn(hashlib.sha256(source).hexdigest(), text)
        self.assertIn(release.uri() + "/src.tar.gz", text)
        self.assertIn(f"{len(source)} bytes", text)
        self.assertIn(hashlib.sha256(windows_plugins()).hexdigest(), text)
        self.assertIn(hashlib.sha256(data()).hexdigest(), text)
        self.assertNotIn("plugins-linux.tar.gz", text)


TUTORIAL_FONT = "skins/Tutorials/10 Adding Fonts.surge-skin/fonts/Lobster-Regular.ttf"


def data_with_tutorials():
    """The data asset with a tutorial skin's font, which the trim leaves out."""
    files = {**FACTORY, **THIRD_PARTY, TUTORIAL_FONT: b"an OFL font"}
    return tar_gz([(".", None), ("./Surge Synth Team", None)]
                  + [(DATA + name, content) for name, content in files.items()])


class Interrupted:
    """A download that runs `meanwhile` once, after its first block is read."""

    def __init__(self, response, meanwhile):
        self.response = response
        self.meanwhile = meanwhile

    def __enter__(self):
        self.response.__enter__()
        return self

    def __exit__(self, *exc):
        return self.response.__exit__(*exc)

    def read(self, size):
        block = self.response.read(size)
        if self.meanwhile is not None:
            meanwhile, self.meanwhile = self.meanwhile, None
            meanwhile()
        return block


class Kept(unittest.TestCase):
    """What a fetch keeps between runs: a platform's folder only while it is the one its stamp
    records, file by file, under the rules it was made by."""

    # The stamp records the SHA-256 of every file the folder was made with.
    def test_the_stamp_records_every_file_extracted(self):
        release = standard(self)
        self.assertEqual(release.fetch("linux-x86_64")[0], 0)
        with open(os.path.join(release.dir, "linux-x86_64.stamp"), encoding="utf-8") as file:
            stamp = json.load(file)
        want = {name: hashlib.sha256(content).hexdigest()
                for name, content in {"Surge XT.clap": CLAP, **WANT_DATA}.items()}
        self.assertEqual(stamp["files"], want)

    # A file of the folder changed, removed or added after it was made: the next fetch, offline,
    # makes the folder again from the downloads, so it never gives a changed file as the plugin.
    def test_a_changed_folder_is_made_again(self):
        release = standard(self)
        self.assertEqual(release.fetch("linux-x86_64")[0], 0)
        linux = os.path.join(release.dir, "linux-x86_64")

        def write(path, content):
            with open(os.path.join(linux, path), "wb") as file:
                file.write(content)

        changes = [
            lambda: write("Surge XT.clap", b"TAMPERED"),
            lambda: os.remove(os.path.join(linux, "SurgeXTData", "skins", "dark", "skin.xml")),
            lambda: write(os.path.join("SurgeXTData", "extra.txt"), b"x"),
        ]
        for number, change in enumerate(changes):
            with self.subTest(change=number):
                change()
                status, out, err = release.fetch("--offline", "linux-x86_64")
                self.assertEqual((status, out), (0, linux + "\n"), err)
                self.assertEqual(release.files("linux-x86_64"),
                                 {"Surge XT.clap": CLAP, **WANT_DATA})

    # A folder made under other rules (here: before the tutorial skins were left out) is made
    # again by the next fetch, offline, under the rules as they are.
    def test_a_folder_made_under_other_rules_is_made_again(self):
        release = standard(self, data=("content.tar.gz", data_with_tutorials()))
        with mock.patch.object(surge, "LEFT_OUT", surge.LEFT_OUT[:-1]):
            self.assertEqual(release.fetch("linux-x86_64")[0], 0)
        self.assertIn("SurgeXTData/" + TUTORIAL_FONT, release.files("linux-x86_64"))
        status, _, err = release.fetch("--offline", "linux-x86_64")
        self.assertEqual(status, 0, err)
        self.assertEqual(release.files("linux-x86_64"), {"Surge XT.clap": CLAP, **WANT_DATA})


class Trim(unittest.TestCase):
    # The tutorial skins, whose fonts are under the OFL, are left out with the third-party
    # patches and wavetables; every other skin is kept.
    def test_the_tutorial_skins_are_left_out(self):
        release = standard(self, data=("content.tar.gz", data_with_tutorials()))
        status, _, err = release.fetch("linux-x86_64")
        self.assertEqual(status, 0, err)
        self.assertEqual(release.files("linux-x86_64"), {"Surge XT.clap": CLAP, **WANT_DATA})

    # An archive may expand to at most 1 GiB, and no member to more than 256 MiB: past either
    # (here with the caps lowered to fit the tests' small archives) it is refused whole, before
    # anything of it is extracted.
    def test_an_archive_expanding_past_its_caps_is_refused_whole(self):
        self.assertEqual((surge.MAX_TOTAL, surge.MAX_MEMBER), (1 << 30, 256 << 20))
        big = DATA + "wavetables/big.wt"
        content = tar_gz([(DATA + "wavetables/a.wt", b"a" * 40), (big, b"b" * 65)])
        for cap, value in (("MAX_MEMBER", 64), ("MAX_TOTAL", 104)):
            with self.subTest(cap=cap):
                release = standard(self, data=("content.tar.gz", content))
                with mock.patch.object(surge, cap, value):
                    status, _, err = release.fetch("linux-x86_64")
                self.assertEqual(status, 1, err)
                self.assertIn("content.tar.gz", err)
                self.assertFalse(os.path.exists(os.path.join(release.dir, "linux-x86_64")))
        release = standard(self, data=("content.tar.gz", content))
        with mock.patch.object(surge, "MAX_MEMBER", 65), mock.patch.object(surge, "MAX_TOTAL", 105):
            self.assertEqual(release.fetch("linux-x86_64")[0], 0)


class Downloads(unittest.TestCase):
    # Two fetches of one asset at once: the second runs from start to end while the first is
    # downloading. Each downloads to a name of its own, so both end well, with the whole file
    # in place and no partial file left.
    def test_two_fetches_at_once_each_land_the_whole_file(self):
        release = standard(self)
        real = surge.urllib.request.urlopen
        second = []

        def opened(url, timeout):
            response = real(url, timeout=timeout)
            if second:
                return response
            second.append(None)
            return Interrupted(response, lambda: second.append(release.fetch("--source")))

        with mock.patch.object(surge.urllib.request, "urlopen", opened):
            status, out, err = release.fetch("--source")
        path = os.path.join(release.dir, "downloads", "src.tar.gz", "src.tar.gz")
        self.assertEqual((status, out), (0, path + "\n"), err)
        self.assertEqual(second[1][:2], (0, path + "\n"), second[1][2])
        self.assertEqual(release.downloaded("src.tar.gz"), ["src.tar.gz"])
        with open(path, "rb") as file:
            self.assertEqual(file.read(), tar_gz([("./surge/LICENSE", b"GPL")]))

    # --if-fetched: what is fetched, and nothing (status 0) for what is not; a file there that
    # is not the one pinned is still refused (status 1), never taken for "not fetched".
    def test_if_fetched_skips_only_what_is_not_fetched(self):
        release = standard(self)
        self.assertEqual(release.fetch("--if-fetched", "linux-x86_64"), (0, "", ""))
        self.assertFalse(os.path.exists(os.path.join(release.dir, "linux-x86_64")))
        self.assertEqual(release.fetch("linux-x86_64")[0], 0)
        linux = os.path.join(release.dir, "linux-x86_64")
        status, out, err = release.fetch("--if-fetched", "linux-x86_64", "windows-x86_64")
        self.assertEqual((status, out), (0, linux + "\n"), err)
        os.remove(linux + ".stamp")
        kept = os.path.join(release.dir, "downloads", "content.tar.gz", "content.tar.gz")
        with open(kept, "r+b") as file:
            file.write(b"junk")
        status, out, err = release.fetch("--if-fetched", "linux-x86_64")
        self.assertEqual((status, out), (1, ""))
        self.assertIn("refused", err)


VST3 = b"\x7fELF the instrument"
VST3_FX = b"\x7fELF the effects"
WIN_VST3 = b"MZ the instrument"
WIN_VST3_FX = b"MZ the effects"
LINUX_BUNDLES = {"Surge XT.vst3/Contents/x86_64-linux/Surge XT.so": VST3,
                 "Surge XT Effects.vst3/Contents/x86_64-linux/Surge XT Effects.so": VST3_FX}
WINDOWS_BUNDLES = {"Surge XT.vst3/Contents/x86_64-win/Surge XT.vst3": WIN_VST3,
                   "Surge XT Effects.vst3/Contents/x86_64-win/Surge XT Effects.vst3": WIN_VST3_FX}


def linux_plugins_vst3(bundles=LINUX_BUNDLES):
    """A Linux plugins asset laid out as the release's: the VST3 bundles (`bundles`, by their
    binaries' paths) beside the CLAP, an LV2 and the standalone."""
    folders = sorted({"./" + "/".join(name.split("/")[:depth])
                      for name in bundles for depth in (1, 2, 3)})
    return tar_gz([(".", None), ("./Surge XT.clap", CLAP), ("./Surge XT", b"standalone"),
                   ("./Surge XT.lv2", None), ("./Surge XT.lv2/libSurge XT.so", b"lv2")]
                  + [(folder, None) for folder in folders]
                  + [("./" + name, content) for name, content in bundles.items()])


def windows_plugins_vst3():
    """A Windows plugins asset laid out as the release's: each VST3 bundle with its binary, its
    desktop.ini and its icon."""
    members = [("Surge XT.clap", WIN_CLAP), ("Surge XT.exe", b"standalone")]
    for name, content in WINDOWS_BUNDLES.items():
        bundle = name.split("/")[0]
        members += [(name, content), (bundle + "/desktop.ini", b"[.ShellClassInfo]"),
                    (bundle + "/Plugin.ico", b"an icon")]
    return zipped(members)


def vst3_release(test, linux=None):
    return standard(test, **{
        "linux-x86_64": ("plugins-linux.tar.gz", linux or linux_plugins_vst3()),
        "windows-x86_64": ("plugins-win64.zip", windows_plugins_vst3()),
    })


class Vst3(unittest.TestCase):
    """fetch --vst3: the folder the VST3 host's tests load, never installed or packaged."""

    # Each platform's VST3 folder holds the two bundles, each with its binary alone at its
    # place, and the factory data as SurgeXTData/ beside them: no CLAP, LV2, standalone or
    # Explorer files. The CLAP folder is not made.
    def test_the_bundles_and_the_factory_data_alone(self):
        release = vst3_release(self)
        status, out, err = release.fetch("--vst3", "linux-x86_64", "windows-x86_64")
        self.assertEqual(status, 0, err)
        linux = os.path.join(release.dir, "linux-x86_64-vst3")
        windows = os.path.join(release.dir, "windows-x86_64-vst3")
        self.assertEqual(out.splitlines(), [linux, windows])
        self.assertEqual(release.files("linux-x86_64-vst3"), {**LINUX_BUNDLES, **WANT_DATA})
        self.assertEqual(release.files("windows-x86_64-vst3"), {**WINDOWS_BUNDLES, **WANT_DATA})
        if os.name == "posix":
            for name in LINUX_BUNDLES:
                self.assertTrue(os.access(os.path.join(linux, name), os.X_OK), name)
        self.assertFalse(os.path.exists(os.path.join(release.dir, "linux-x86_64")))

    # The VST3 folder has a stamp of its own: a file of it changed is made again by the next
    # fetch, offline, while the CLAP folder and its stamp stay as they were, current.
    def test_a_stamp_of_its_own_leaves_the_clap_folder_alone(self):
        release = vst3_release(self)
        self.assertEqual(release.fetch("linux-x86_64")[0], 0)
        self.assertEqual(release.fetch("--vst3", "linux-x86_64")[0], 0)
        clap_stamp = os.path.join(release.dir, "linux-x86_64.stamp")
        with open(clap_stamp, "rb") as file:
            clap_before = file.read()
        with open(os.path.join(release.dir, "linux-x86_64-vst3.stamp"), encoding="utf-8") as file:
            stamp = json.load(file)
        want = {name: hashlib.sha256(content).hexdigest()
                for name, content in {**LINUX_BUNDLES, **WANT_DATA}.items()}
        self.assertEqual(stamp["files"], want)
        vst3 = os.path.join(release.dir, "linux-x86_64-vst3")
        binary = os.path.join(vst3, *next(iter(LINUX_BUNDLES)).split("/"))
        with open(binary, "wb") as file:
            file.write(b"TAMPERED")
        status, out, err = release.fetch("--offline", "--vst3", "linux-x86_64")
        self.assertEqual((status, out), (0, vst3 + "\n"), err)
        self.assertIn("made again", err)
        self.assertEqual(release.files("linux-x86_64-vst3"), {**LINUX_BUNDLES, **WANT_DATA})
        status, out, err = release.fetch("--offline", "linux-x86_64")
        clap = os.path.join(release.dir, "linux-x86_64")
        self.assertEqual((status, out, err), (0, clap + "\n", ""))
        with open(clap_stamp, "rb") as file:
            self.assertEqual(file.read(), clap_before)
        self.assertEqual(release.files("linux-x86_64"), {"Surge XT.clap": CLAP, **WANT_DATA})

    # A VST3 folder made under other rules (here: with the instrument's binary alone) is made
    # again by the next fetch, offline, under the rules as they are.
    def test_a_folder_made_under_other_rules_is_made_again(self):
        release = vst3_release(self)
        instrument = next(iter(LINUX_BUNDLES))
        with mock.patch.dict(surge.VST3_PLUGINS, {"linux-x86_64": (instrument,)}):
            self.assertEqual(release.fetch("--vst3", "linux-x86_64")[0], 0)
        self.assertEqual(release.files("linux-x86_64-vst3"),
                         {instrument: LINUX_BUNDLES[instrument], **WANT_DATA})
        status, _, err = release.fetch("--offline", "--vst3", "linux-x86_64")
        self.assertEqual(status, 0, err)
        self.assertEqual(release.files("linux-x86_64-vst3"), {**LINUX_BUNDLES, **WANT_DATA})

    # --offline: status 3 and nothing made while not fetched, the folder once it is;
    # --if-fetched: nothing (status 0) while not fetched.
    def test_offline_and_if_fetched(self):
        release = vst3_release(self)
        status, _, err = release.fetch("--offline", "--vst3", "linux-x86_64")
        self.assertEqual(status, 3, err)
        self.assertEqual(release.fetch("--if-fetched", "--vst3", "linux-x86_64"), (0, "", ""))
        self.assertFalse(os.path.exists(os.path.join(release.dir, "linux-x86_64-vst3")))
        self.assertEqual(release.fetch("--vst3", "linux-x86_64")[0], 0)
        os.remove(os.path.join(release.release, "plugins-linux.tar.gz"))
        status, out, err = release.fetch("--if-fetched", "--vst3", "linux-x86_64")
        self.assertEqual((status, out), (0, os.path.join(release.dir, "linux-x86_64-vst3") + "\n"),
                         err)

    # A plugins asset without one of the bundles' binaries is refused, naming the one it lacks,
    # and nothing is made.
    def test_an_archive_without_a_bundle_is_refused(self):
        effects = "Surge XT Effects.vst3/Contents/x86_64-linux/Surge XT Effects.so"
        alone = {name: content for name, content in LINUX_BUNDLES.items() if name != effects}
        release = vst3_release(self, linux=linux_plugins_vst3(alone))
        status, _, err = release.fetch("--vst3", "linux-x86_64")
        self.assertEqual(status, 1)
        self.assertIn(effects, err)
        self.assertFalse(os.path.exists(os.path.join(release.dir, "linux-x86_64-vst3")))

    # The CLAP folder's rules are the ones its stamps were made under before --vst3 existed
    # (RULES 2, from the rules as they were written then), so a CLAP folder fetched before is
    # still current and not made again.
    def test_the_clap_rules_are_unchanged(self):
        release = standard(self)
        lock = surge.Lock(release.lock)
        self.assertEqual(surge.made_by(lock, "linux-x86_64")["rules"], {
            "version": 2, "plugin": "Surge XT.clap", "data": "Surge Synth Team/SurgeXTData",
            "left out": ["patches_3rdparty", "wavetables_3rdparty", "skins/Tutorials"]})


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    unittest.main(verbosity=1)
