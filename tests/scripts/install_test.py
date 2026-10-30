"""Tests of scripts/install.sh and scripts/install-check.sh against what an install must do: the
files it put are its own (lib/dawsome/install-manifest), so a reinstall replaces those and
removes those it no longer puts, an uninstall removes those and each folder it made once that is
empty, and a plugin of the user's own in lib/dawsome/clap outlives both; and a bundled plugin
whose fetch refuses a file fails the install and its check, while one not fetched only leaves
the plugin out, keeping the one installed before.

The installs go into temporary prefixes with stand-in programs and a stand-in plugin folder;
the instrument sets are the kit scripts/fetch-kit.sh fetched into the target dir (scripts/check.sh
fetches it first). The test that needs a target dir of its own clones the kit into it from that
one, so nothing is downloaded. Every temporary folder lies in <target dir>/install-test, and
nothing outside it is written but the install's own staging folder in the target dir.

Run (from the repository root; scripts/check.sh runs it):
  python3 -I -B tests/scripts/install_test.py
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CLAP = "lib/dawsome/clap"


def run(*command, env=None):
    """The finished `command` (a script of the repository and its arguments), run from the
    repository root."""
    return subprocess.run(["bash", *command], cwd=ROOT, env=env, capture_output=True, text=True,
                          check=False)


def target_dir():
    """The target dir the scripts build into, as scripts/target-dir.sh names it."""
    found = run("scripts/target-dir.sh")
    if found.returncode != 0:
        raise RuntimeError(found.stderr)
    return found.stdout.strip()


TARGET = target_dir()


def write(path, content):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as file:
        file.write(content)


def files(top):
    """Every file under `top`, by its path below it (`/` between the parts), with its bytes."""
    found = {}
    for here, _, names in os.walk(top):
        for name in names:
            path = os.path.join(here, name)
            with open(path, "rb") as file:
                found[os.path.relpath(path, top).replace(os.sep, "/")] = file.read()
    return found


def folders(top):
    """Every folder under `top`, by its path below it."""
    return {os.path.relpath(here, top).replace(os.sep, "/")
            for here, _, _ in os.walk(top)} - {"."}


def linux_asset():
    """The file name locks/surge.lock pins for the Linux plugins."""
    with open(os.path.join(ROOT, "locks", "surge.lock"), encoding="utf-8") as file:
        for line in file:
            if line.startswith("asset.linux-x86_64="):
                return line.split("=", 1)[1].split()[0]
    raise RuntimeError("locks/surge.lock pins no linux-x86_64 asset")


class Install(unittest.TestCase):
    def setUp(self):
        parent = os.path.join(TARGET, "install-test")
        os.makedirs(parent, exist_ok=True)
        temp = tempfile.TemporaryDirectory(prefix="install-", dir=parent)
        self.addCleanup(temp.cleanup)
        self.temp = temp.name
        self.prefix = os.path.join(self.temp, "prefix")
        self.programs = os.path.join(self.temp, "programs")
        for program in ("dawsome", "dawsome-app"):
            write(os.path.join(self.programs, program), f"stand-in {program}".encode())

    def surge(self, name, contents):
        """A stand-in for the fetched plugin folder, holding `contents` (path: bytes)."""
        folder = os.path.join(self.temp, name)
        for path, content in contents.items():
            write(os.path.join(folder, path), content)
        return folder

    def install(self, *args, env=None):
        return run("scripts/install.sh", "--prefix", self.prefix, "--programs", self.programs,
                   *args, env=env)

    def uninstall(self):
        return run("scripts/uninstall.sh", "--prefix", self.prefix)

    def manifest(self):
        with open(os.path.join(self.prefix, "lib/dawsome/install-manifest"),
                  encoding="utf-8") as file:
            return [line for line in file.read().splitlines() if not line.startswith("#")]

    # The manifest names every file the install put. A reinstall replaces those and removes
    # those it no longer puts (with the folder that leaves empty); the uninstall removes them all
    # and each folder the install made once it is empty. A plugin of the user's own beside the
    # bundled one, and a file of theirs in a folder the install made, outlive both.
    def test_a_users_own_files_outlive_reinstall_and_uninstall(self):
        first = self.surge("surge-1", {"Surge XT.clap": b"plugin 1", "SurgeXTData/a.txt": b"a",
                                       "SurgeXTData/sub/b.txt": b"b"})
        done = self.install("--surge", first)
        self.assertEqual(done.returncode, 0, done.stderr)
        clap = os.path.join(self.prefix, CLAP)
        self.assertEqual(files(clap), {"Surge XT.clap": b"plugin 1", "SurgeXTData/a.txt": b"a",
                                       "SurgeXTData/sub/b.txt": b"b"})
        manifest = self.manifest()
        for place in ("bin/dawsome", "bin/dawsome-app", "share/applications/dawsome.desktop",
                      "share/mime/packages/dawsome.xml", f"{CLAP}/Surge XT.clap",
                      f"{CLAP}/SurgeXTData/sub/b.txt"):
            self.assertIn(place, manifest)
        # The project file type, as shipped; the temporary prefix keeps no MIME database, so
        # none is made there (the uninstall would not own it), and the install says so.
        with open(os.path.join(self.prefix, "share/mime/packages/dawsome.xml"), "rb") as file, \
                open(os.path.join(ROOT, "assets/dawsome-mime.xml"), "rb") as shipped:
            self.assertEqual(file.read(), shipped.read())
        self.assertFalse(os.path.exists(os.path.join(self.prefix, "share/mime/mime.cache")))
        self.assertIn("keeps no MIME database", done.stderr)
        self.assertTrue(any(place.startswith("share/dawsome/instruments/") for place in manifest))
        with open(os.path.join(self.prefix, "bin/dawsome"), "rb") as file:
            self.assertEqual(file.read(), b"stand-in dawsome")
        self.assertTrue(os.access(os.path.join(self.prefix, "bin/dawsome"), os.X_OK))

        write(os.path.join(clap, "My Synth.clap"), b"mine")
        write(os.path.join(self.prefix, "share/applications/other.desktop"), b"other")
        second = self.surge("surge-2", {"Surge XT.clap": b"plugin 2", "SurgeXTData/a.txt": b"a2",
                                        "SurgeXTData/c.txt": b"c"})
        done = self.install("--surge", second)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(files(clap), {"My Synth.clap": b"mine", "Surge XT.clap": b"plugin 2",
                                       "SurgeXTData/a.txt": b"a2", "SurgeXTData/c.txt": b"c"})
        self.assertFalse(os.path.exists(os.path.join(clap, "SurgeXTData", "sub")))
        self.assertNotIn(f"{CLAP}/SurgeXTData/sub/b.txt", self.manifest())

        done = self.uninstall()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(files(self.prefix), {f"{CLAP}/My Synth.clap": b"mine",
                                              "share/applications/other.desktop": b"other"})
        self.assertEqual(folders(self.prefix), {"lib", "lib/dawsome", CLAP, "share",
                                                "share/applications"})

    # A prefix that keeps a MIME database (as ~/.local does on most desktops) has it refreshed:
    # after the install the database knows *.dawsome as the project type, after the uninstall it
    # no longer does, and the database stays, being the desktop's.
    @unittest.skipUnless(shutil.which("update-mime-database"), "update-mime-database is not installed")
    def test_a_kept_mime_database_learns_the_project_type_and_forgets_it(self):
        mime = os.path.join(self.prefix, "share/mime")
        os.makedirs(os.path.join(mime, "packages"))
        made = subprocess.run(["update-mime-database", mime], capture_output=True, text=True,
                              check=False)
        self.assertEqual(made.returncode, 0, made.stderr)
        globs = os.path.join(mime, "globs2")

        def knows():
            with open(globs, encoding="utf-8") as file:
                return any(line.split(":")[1:] == ["application/x-dawsome-project", "*.dawsome"]
                           for line in file.read().splitlines() if not line.startswith("#"))

        self.assertFalse(knows())
        done = self.install("--surge", self.surge("surge", {"Surge XT.clap": b"plugin"}))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertNotIn("MIME database", done.stderr)
        self.assertTrue(knows())
        done = self.uninstall()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertFalse(knows())
        self.assertTrue(os.path.isfile(os.path.join(mime, "mime.cache")))

    # With a target dir of its own: a downloaded plugin archive that is not the one pinned makes
    # the fetch refuse it, and that fails the install and the install check with the fetch's
    # message, the earlier install left as it was. Once the file is gone, the plugin is not
    # fetched: the install then keeps the plugin installed before, still its own.
    def test_a_refused_plugin_fails_the_install_and_one_not_fetched_is_kept(self):
        plugin = self.surge("surge", {"Surge XT.clap": b"plugin", "SurgeXTData/a.txt": b"a"})
        done = self.install("--surge", plugin)
        self.assertEqual(done.returncode, 0, done.stderr)
        clap = os.path.join(self.prefix, CLAP)
        installed = files(clap)

        target = os.path.join(self.temp, "target")
        name = linux_asset()
        junk = os.path.join(target, "surge", "downloads", name, name)
        write(junk, b"not the pinned file")
        env = {**os.environ, "CARGO_TARGET_DIR": target,
               "DAWSOME_KIT_URL": os.path.join(TARGET, "kit")}
        for command in (("scripts/install.sh", "--prefix", self.prefix, "--programs",
                         self.programs), ("scripts/install-check.sh",)):
            with self.subTest(command=command[0]):
                done = run(*command, env=env)
                self.assertNotEqual(done.returncode, 0, done.stdout)
                self.assertIn("refused", done.stderr)
                self.assertEqual(files(clap), installed)

        os.remove(junk)
        done = self.install(env=env)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("keeping the bundled plugin installed before", done.stdout)
        self.assertEqual(files(clap), installed)
        self.assertIn(f"{CLAP}/Surge XT.clap", self.manifest())
        done = self.uninstall()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertFalse(os.path.exists(self.prefix))


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    unittest.main(verbosity=1)
