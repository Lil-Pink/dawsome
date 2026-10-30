"""Tests of scripts/vst3_test_plugins.py, the fetch of the real VST3 plugins the VST3 host's
integration tests load, against what it must do by locks/vst3-test.lock's rules: each asset kept only
when its size and SHA-256 are the ones pinned, from its own url; only the files and folders named
taken, below the archive's top folder, and nothing else; a link or device elsewhere in an archive
passed over and one among the members taken refused; a name that could climb out refused whole;
a folder made again when a file in it changed; --offline and --if-fetched; and the real lock
pinning an asset for every plugin the script takes.

The releases are small archives made here, in a temporary folder, served through file: URLs;
nothing is downloaded from the network and nothing outside the temporary folder is written.

Run (from the repository root; scripts/check.sh runs it):
  python3 -I -B tests/scripts/fetch_vst3_test_plugins_test.py
"""
import contextlib
import hashlib
import importlib.util
import io
import lzma
import os
import sys
import tarfile
import tempfile
import unittest
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def load(name):
    """scripts/<name>.py as a module, read from its path (no folder of it on sys.path)."""
    spec = importlib.util.spec_from_file_location(name, os.path.join(ROOT, "scripts", f"{name}.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


plugins = load("vst3_test_plugins")

DEXED = {
    "Dexed.vst3/Contents/x86_64-linux/Dexed.so": b"\x7fELF dexed",
    "Dexed.vst3/Contents/Resources/moduleinfo.json": b"{}",
    "LICENSE": b"GPL-3.0",
}
DPF = {
    "3BandEQ.vst3/Contents/x86_64-linux/3BandEQ.so": b"\x7fELF eq vst3",
    "3BandEQ.clap": b"\x7fELF eq clap",
}


def zipped(members):
    """A .zip of `members`, (name, bytes) each; executables marked so (mode 0755)."""
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as archive:
        for name, data in members:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.external_attr = (0o100755 if b"ELF" in data else 0o100644) << 16
            archive.writestr(info, data)
    return out.getvalue()


def tar_xz(members):
    """A .tar.xz of `members`: (name, bytes) a file, (name, ("hard", target)) a hard link,
    (name, ("sym", target)) a symbolic link."""
    out = io.BytesIO()
    with lzma.LZMAFile(out, "wb") as packed, tarfile.open(fileobj=packed, mode="w") as archive:
        for name, data in members:
            info = tarfile.TarInfo(name)
            if isinstance(data, tuple):
                info.type = tarfile.LNKTYPE if data[0] == "hard" else tarfile.SYMTYPE
                info.linkname = data[1]
                archive.addfile(info)
            else:
                info.size = len(data)
                info.mode = 0o755
                archive.addfile(info, io.BytesIO(data))
    return out.getvalue()


def dexed_zip(extra=()):
    return zipped([("Dexed", b"\x7fELF standalone"), ("Dexed.clap", b"\x7fELF clap")]
                  + list(DEXED.items()) + list(extra))


def dpf_tar(extra=()):
    top = "DPF-Plugins-v1.7/"
    members = [(top + name, data) for name, data in DPF.items()]
    members += [(top + "MVerb.clap", b"\x7fELF mverb"),
                (top + "ProM.lv2/presets/a.milk", b"preset"),
                (top + "ProM.vst/presets/a.milk", ("hard", top + "ProM.lv2/presets/a.milk"))]
    return tar_xz(members + [(top + name, data) for name, data in extra])


class Release:
    """Each asset served from a folder of its own through a file: URL, a lock pinning them, and
    the folder the fetch works in, under one temporary folder."""

    def __init__(self, test, dexed=None, dpf=None):
        self.temp = tempfile.TemporaryDirectory(prefix="dawsome-vst3-test-")
        test.addCleanup(self.temp.cleanup)
        self.root = self.temp.name
        self.dir = os.path.join(self.root, "vst3-test")
        lines = ["version=test"]
        for what, name, content in (("dexed", "Dexed-lnx.zip", dexed or dexed_zip()),
                                    ("dpf", "DPF-linux.tar.xz", dpf or dpf_tar())):
            release = os.path.join(self.root, "release", what)
            os.makedirs(release)
            self.serve(what, name, content)
            lines.append("url.%s=file://%s" % (what, release.replace(os.sep, "/")))
            digest = hashlib.sha256(content).hexdigest()
            lines.append(f"asset.{what}={name} {len(content)} {digest}")
        self.lock = os.path.join(self.root, "vst3-test.lock")
        with open(self.lock, "w", encoding="utf-8") as file:
            file.write("\n".join(lines) + "\n")

    def serve(self, what, name, content):
        with open(os.path.join(self.root, "release", what, name), "wb") as file:
            file.write(content)

    def fetch(self, *args):
        """The exit status of `vst3_test_plugins.py fetch` with this lock and folder, and its
        output."""
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = plugins.main(["fetch", "--lock", self.lock, "--dir", self.dir, *args])
        return status, out.getvalue(), err.getvalue()

    def files(self, folder):
        found = {}
        base = os.path.join(self.dir, folder)
        for top, _, names in os.walk(base):
            for name in names:
                path = os.path.join(top, name)
                with open(path, "rb") as file:
                    found[os.path.relpath(path, base).replace(os.sep, "/")] = file.read()
        return found


class Fetch(unittest.TestCase):
    # Each folder holds the files and folders named, at their places below the archive's top
    # folder, and nothing else (no standalone, no other plugin, no presets); the binaries keep
    # their executable mark; each asset came from its own url.
    def test_only_what_is_named_is_taken(self):
        release = Release(self)
        status, out, err = release.fetch("dexed", "dpf")
        self.assertEqual(status, 0, err)
        self.assertEqual(out.splitlines(), [os.path.join(release.dir, "dexed"),
                                            os.path.join(release.dir, "dpf")])
        self.assertEqual(release.files("dexed"), DEXED)
        self.assertEqual(release.files("dpf"), DPF)
        if os.name == "posix":
            so = os.path.join(release.dir, "dexed", "Dexed.vst3", "Contents", "x86_64-linux",
                              "Dexed.so")
            self.assertTrue(os.access(so, os.X_OK))
            license = os.path.join(release.dir, "dexed", "LICENSE")
            self.assertFalse(os.access(license, os.X_OK))

    # A served file that is not the one pinned is refused and nothing of it kept.
    def test_a_changed_download_is_refused(self):
        release = Release(self)
        release.serve("dexed", "Dexed-lnx.zip", dexed_zip([("extra", b"x")]))
        status, _, err = release.fetch("dexed")
        self.assertEqual(status, 1)
        self.assertIn("SHA-256", err)
        self.assertFalse(os.path.exists(os.path.join(release.dir, "dexed")))
        self.assertEqual(os.listdir(os.path.join(release.dir, "downloads", "Dexed-lnx.zip")), [])

    # A link among the members taken refuses the archive; a name that climbs out anywhere in it
    # refuses it whole, taken or not.
    def test_a_link_taken_or_a_climbing_name_is_refused(self):
        bad = [("3BandEQ.vst3/Contents/x86_64-linux/link.so", ("sym", "/usr/lib/libc.so.6")),
               ("../../escape.txt", b"x")]
        for name, data in bad:
            with self.subTest(member=name):
                release = Release(self, dpf=dpf_tar([(name, data)]))
                status, _, err = release.fetch("dpf")
                self.assertEqual(status, 1, err)
                self.assertIn("DPF-linux.tar.xz", err)
                self.assertFalse(os.path.exists(os.path.join(release.dir, "dpf")))
                self.assertFalse(os.path.exists(os.path.join(release.root, "escape.txt")))

    # A file of a folder changed after it was made: the next fetch, offline, makes it again.
    def test_a_changed_folder_is_made_again(self):
        release = Release(self)
        self.assertEqual(release.fetch("dpf")[0], 0)
        with open(os.path.join(release.dir, "dpf", "3BandEQ.clap"), "wb") as file:
            file.write(b"TAMPERED")
        status, out, err = release.fetch("--offline", "dpf")
        self.assertEqual((status, out), (0, os.path.join(release.dir, "dpf") + "\n"), err)
        self.assertIn("made again", err)
        self.assertEqual(release.files("dpf"), DPF)

    # --offline: status 3 while not fetched; --if-fetched: nothing, status 0; each gives what is
    # fetched once it is.
    def test_offline_and_if_fetched(self):
        release = Release(self)
        self.assertEqual(release.fetch("--offline", "dexed")[0], 3)
        self.assertEqual(release.fetch("--if-fetched", "dexed", "dpf"), (0, "", ""))
        self.assertEqual(release.fetch("dexed")[0], 0)
        dexed = os.path.join(release.dir, "dexed") + "\n"
        self.assertEqual(release.fetch("--if-fetched", "dexed", "dpf")[:2], (0, dexed))
        self.assertEqual(release.fetch("--offline", "dexed")[:2], (0, dexed))

    # No NAME means every plugin (Python 3.11's argparse refused the empty list once); a NAME
    # the script does not take is refused, saying which.
    def test_no_name_is_every_plugin_and_an_unknown_one_is_refused(self):
        with tempfile.TemporaryDirectory() as empty:
            lock = os.path.join(ROOT, "locks", "vst3-test.lock")
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                status = plugins.main(["fetch", "--lock", lock, "--dir", empty, "--if-fetched"])
            self.assertEqual((status, out.getvalue()), (0, ""), err.getvalue())
            self.assertEqual(os.listdir(empty), [])
        status, _, err = Release(self).fetch("nosuch")
        self.assertEqual(status, 2)
        self.assertIn("'nosuch'", err)

    # The real lock pins an asset, with a url of its own, for every plugin the script takes.
    def test_the_lock_pins_every_plugin_taken(self):
        lock = plugins.surge.Lock(os.path.join(ROOT, "locks", "vst3-test.lock"))
        self.assertEqual(sorted(lock.assets), sorted(plugins.TAKEN))
        for what in plugins.TAKEN:
            self.assertTrue(lock.url_of(what).startswith("https://github.com/"), what)


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    unittest.main(verbosity=1)
