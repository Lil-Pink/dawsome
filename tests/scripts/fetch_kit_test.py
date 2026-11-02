"""Tests of scripts/fetch-kit.sh, the fetch of the instrument sets at locks/kit.lock's commit: a
fresh fetch lands the kit at its commit, and a kit folder whose .git is not a repository (a
cache restored it in part, as CI's did) is fetched again, while the repository around it is left
exactly as it was. Why the second: git run in such a folder finds the enclosing repository
instead, and the script's forced checkout and clean then acted on that one: on CI they put the
kit's files over the checkout and deleted the target dir.

The repository and the kit here are small ones made by the test in a temporary folder (the
script, its helpers and a package of its own), the kit served by its path; nothing is fetched
from the network and nothing outside the temporary folder is written.
Run (from the repository root; scripts/check.sh runs it):
  python3 -I -B tests/scripts/fetch_kit_test.py
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCRIPTS = ("fetch-kit.sh", "target-dir.sh", "python.sh")


def git(cwd, *args):
    env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
               GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.invalid",
               GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@example.invalid")
    done = subprocess.run(["git", "-C", cwd, *args], env=env, capture_output=True, text=True)
    if done.returncode != 0:
        raise AssertionError(f"git {' '.join(args)}: {done.stderr}")
    return done.stdout.strip()


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as file:
        file.write(text)


class Setup:
    """A kit repository with one commit and a repository holding the script, locked to it."""

    def __init__(self, test):
        temp = tempfile.TemporaryDirectory()
        test.addCleanup(temp.cleanup)
        self.kit_origin = os.path.join(temp.name, "kit-origin")
        write(os.path.join(self.kit_origin, "tools", "check.py"), "print('check: the kit passes')\n")
        write(os.path.join(self.kit_origin, "sets", "a.sfz"), "<region> sample=a.wav\n")
        git(temp.name, "init", "-q", "--initial-branch=main", self.kit_origin)
        git(self.kit_origin, "add", "-A")
        git(self.kit_origin, "commit", "-q", "-m", "kit")
        self.commit = git(self.kit_origin, "rev-parse", "HEAD")
        self.repo = os.path.join(temp.name, "repo")
        for script in SCRIPTS:
            os.makedirs(os.path.join(self.repo, "scripts"), exist_ok=True)
            shutil.copy2(os.path.join(ROOT, "scripts", script),
                         os.path.join(self.repo, "scripts", script))
        # A workspace of its own, so cargo never takes it for a member of one around it.
        write(os.path.join(self.repo, "Cargo.toml"),
              '[package]\nname = "kit-test"\nversion = "0.0.0"\nedition = "2021"\n\n[workspace]\n')
        write(os.path.join(self.repo, "src", "lib.rs"), "")
        write(os.path.join(self.repo, "locks", "kit.lock"),
              f"url={self.kit_origin}\ncommit={self.commit}\n")
        write(os.path.join(self.repo, ".gitignore"), "/target\n")
        git(temp.name, "init", "-q", "--initial-branch=main", self.repo)
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "repo")
        self.head = git(self.repo, "rev-parse", "HEAD")
        write(os.path.join(self.repo, "unsaved.txt"), "work not committed yet\n")
        self.kit = os.path.join(self.repo, "target", "kit")

    def fetch(self):
        env = {name: value for name, value in os.environ.items()
               if name not in ("CARGO_TARGET_DIR", "DAWSOME_KIT_URL")}
        return subprocess.run(["bash", os.path.join(self.repo, "scripts", "fetch-kit.sh")],
                              env=env, capture_output=True, text=True)

    def assert_left_alone(self, test):
        test.assertEqual(git(self.repo, "rev-parse", "HEAD"), self.head)
        test.assertEqual(git(self.repo, "status", "--porcelain"), "?? unsaved.txt")
        test.assertEqual(git(self.repo, "for-each-ref", "--format=%(refname)"),
                         "refs/heads/main")
        test.assertTrue(os.path.isfile(os.path.join(self.repo, "Cargo.toml")))


class FetchKit(unittest.TestCase):
    # A fresh fetch: the kit at its commit, its folder printed, the repository as it was.
    def test_a_fresh_fetch_lands_the_kit_at_its_commit(self):
        setup = Setup(self)
        done = setup.fetch()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(os.path.realpath(done.stdout.strip()), os.path.realpath(setup.kit))
        self.assertEqual(git(setup.kit, "rev-parse", "HEAD"), setup.commit)
        setup.assert_left_alone(self)

    # A kit folder whose .git is an empty folder (not a repository): fetched again, and the
    # repository around it keeps its commit, its files, its refs and the work not committed.
    def test_a_broken_kit_folder_is_fetched_again_and_the_repository_left_alone(self):
        setup = Setup(self)
        os.makedirs(os.path.join(setup.kit, ".git"))
        done = setup.fetch()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(git(setup.kit, "rev-parse", "HEAD"), setup.commit)
        self.assertTrue(os.path.isfile(os.path.join(setup.kit, "sets", "a.sfz")))
        setup.assert_left_alone(self)


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    unittest.main(verbosity=1)
