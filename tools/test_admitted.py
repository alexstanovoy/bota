"""tools/admitted.sh fails closed: a rebuild that fails, or a build of another wire, exits non-zero and leaves
no executable of an earlier build to run; only a verified build is kept, the cache keeps what the corpora name,
and stays within its budget.

Each case runs the real script in a throwaway repository with the script's layout, its programs and a corpus,
with a fake `cargo` first on PATH: `metadata` names the target directory, `build` makes executables (the server
says the wire in FAKE_WIRE) or fails as FAKE_BUILD says.
"""

import os
import shutil
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "admitted.sh"
WIRE = "00000000000000aa"
PROGRAMS = [
    "crates/bota_server/examples/admitted_corpus.rs",
    "crates/bota_server/examples/admitted_peer.rs",
    "crates/bota_server/tests/admitted/scenarios.rs",
    "crates/bota_server/tests/admitted/peer.rs",
]

FAKE_CARGO = r"""#!/usr/bin/env bash
set -euo pipefail
case "$1" in
    metadata)
        echo "{\"target_directory\":\"$FAKE_TARGET\"}"
        ;;
    build)
        if [ "$FAKE_BUILD" = fail ]; then
            echo "error[E0425]: cannot find value \`broken\` in this scope" >&2
            exit 101
        fi
        mkdir -p "$CARGO_TARGET_DIR/release/examples"
        printf '#!/bin/sh\necho %s\n' "$FAKE_WIRE" >"$CARGO_TARGET_DIR/release/bota_server"
        printf '#!/bin/sh\necho peer\n' >"$CARGO_TARGET_DIR/release/examples/admitted_peer"
        printf '#!/bin/sh\necho corpus >"$1/corpus.txt"\n' >"$CARGO_TARGET_DIR/release/examples/admitted_corpus"
        chmod +x "$CARGO_TARGET_DIR/release/bota_server" "$CARGO_TARGET_DIR/release/examples/"*
        ;;
    *)
        exit 2
        ;;
esac
"""


class AdmittedBuilds(unittest.TestCase):
    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix="admitted-"))
        self.repo = self.scratch / "repo"
        (self.repo / "tools").mkdir(parents=True)
        shutil.copy(SCRIPT, self.repo / "tools/admitted.sh")
        for program in PROGRAMS:
            path = self.repo / program
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// a program\n")
        (self.repo / "Cargo.toml").write_text("[workspace]\n")
        git = ["git", "-C", str(self.repo), "-c", "user.name=t", "-c", "user.email=t@t"]
        subprocess.run(git[:3] + ["init", "-q"], check=True)
        subprocess.run(git + ["add", "-A"], check=True)
        subprocess.run(git + ["commit", "-qm", "a commit"], check=True)
        commit = subprocess.run(git[:3] + ["rev-parse", "HEAD"], check=True, capture_output=True, text=True)
        corpus = self.repo / "crates/bota_server/tests/admitted" / WIRE
        corpus.mkdir(parents=True)
        (corpus / "corpus.txt").write_text(f"wire {WIRE}\nitems 94\ncommit {commit.stdout.strip()}\n")
        bin_dir = self.scratch / "fake-bin"
        bin_dir.mkdir()
        (bin_dir / "cargo").write_text(FAKE_CARGO)
        (bin_dir / "cargo").chmod(0o755)
        self.target = self.scratch / "target"
        self.cache = self.target / "admitted"
        self.env = {
            **{key: value for key, value in os.environ.items() if key not in ("ADMITTED_BUILDS", "ADMITTED_MAX_BYTES")},
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "FAKE_TARGET": str(self.target),
            "FAKE_WIRE": WIRE,
            "FAKE_BUILD": "ok",
        }

    def tearDown(self):
        shutil.rmtree(self.scratch)

    def build(self, **env):
        return subprocess.run(
            [str(self.repo / "tools/admitted.sh"), "build"],
            env={**self.env, **env},
            capture_output=True,
            text=True,
            timeout=120,
            # Away from any checkout: a relative path the script takes stays in the scratch.
            cwd=self.scratch,
        )

    def test_a_verified_build_keeps_its_executables_sums_wire_and_stamp_and_drops_the_rest(self):
        done = self.build()
        self.assertEqual(done.returncode, 0, done.stderr)
        entry = self.cache / WIRE
        self.assertEqual(sorted(path.name for path in entry.iterdir()), ["bin", "stamp", "sums", "wire"])
        self.assertEqual((entry / "wire").read_text().strip(), WIRE)
        self.assertTrue(os.access(entry / "bin/examples/admitted_peer", os.X_OK))
        self.assertFalse((self.cache / f"{WIRE}.work").exists())

    def test_a_failed_rebuild_over_a_populated_cache_fails_and_leaves_nothing_to_run(self):
        self.assertEqual(self.build().returncode, 0)
        # The programs change, so the cached build is stale; its rebuild fails.
        (self.repo / PROGRAMS[1]).write_text("// a changed program\n")
        failed = self.build(FAKE_BUILD="fail", CARGO_NET_OFFLINE="true")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(f"tools/admitted.sh: building ", failed.stderr)
        self.assertFalse((self.cache / WIRE / "bin/bota_server").exists(), "a stale server is left to run")
        self.assertFalse((self.cache / WIRE / "stamp").exists())

    def test_a_build_of_another_wire_fails_and_is_not_kept(self):
        failed = self.build(FAKE_WIRE="00000000000000bb")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn(f"builds wire 00000000000000bb, its corpus is of {WIRE}", failed.stderr)
        self.assertFalse((self.cache / WIRE).exists())

    def test_the_default_cache_keeps_only_the_wires_the_corpora_name(self):
        gone = self.cache / "00000000000000cc"
        (gone / "bin").mkdir(parents=True)
        self.assertEqual(self.build().returncode, 0)
        self.assertFalse(gone.exists())
        self.assertTrue((self.cache / WIRE / "bin").is_dir())

    def test_a_named_cache_is_never_pruned(self):
        shared = self.scratch / "shared"
        other = shared / "00000000000000cc"
        (other / "bin").mkdir(parents=True)
        self.assertEqual(self.build(ADMITTED_BUILDS=str(shared)).returncode, 0)
        self.assertTrue(other.is_dir(), "another worktree's entry was deleted")

    def test_a_cache_over_its_budget_fails(self):
        failed = self.build(ADMITTED_MAX_BYTES="1")
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("over ADMITTED_MAX_BYTES=1", failed.stderr)

    def fake_tool(self, name, text):
        tool = Path(self.env["PATH"].split(":")[0]) / name
        tool.write_text(text)
        tool.chmod(0o755)

    # A cache is only ever trusted after checking what it serves.

    def test_a_cached_entry_missing_an_executable_is_rebuilt(self):
        self.assertEqual(self.build().returncode, 0)
        peer = self.cache / WIRE / "bin/examples/admitted_peer"
        peer.unlink()
        done = self.build()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(peer.is_file())

    def test_a_cached_server_of_another_wire_is_not_served(self):
        self.assertEqual(self.build().returncode, 0)
        server = self.cache / WIRE / "bin/bota_server"
        server.write_text("#!/bin/sh\necho 00000000000000bb\n")
        done = self.build()
        reported = subprocess.run([str(server)], capture_output=True, text=True).stdout.strip()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(reported, WIRE, "the altered server was kept")

    def test_a_cached_executable_altered_but_still_saying_the_wire_is_rebuilt(self):
        self.assertEqual(self.build().returncode, 0)
        peer = self.cache / WIRE / "bin/examples/admitted_peer"
        peer.write_text("#!/bin/sh\necho altered\n")
        self.assertEqual(self.build().returncode, 0)
        self.assertEqual(subprocess.run([str(peer)], capture_output=True, text=True).stdout.strip(), "peer")

    def test_a_symlinked_cached_entry_is_not_served(self):
        self.assertEqual(self.build().returncode, 0)
        elsewhere = self.scratch / "elsewhere"
        shutil.move(self.cache / WIRE, elsewhere)
        (self.cache / WIRE).symlink_to(elsewhere)
        self.assertEqual(self.build().returncode, 0)
        self.assertFalse((self.cache / WIRE).is_symlink())
        self.assertTrue(elsewhere.is_dir(), "the link's target was deleted")

    # Every path written or deleted, every setting read.

    def test_a_lock_symlink_cannot_clobber_an_unrelated_file(self):
        self.cache.mkdir(parents=True)
        outside = self.scratch / "unrelated.txt"
        outside.write_text("keep these bytes\n")
        (self.cache / ".lock").symlink_to(outside)
        self.build(ADMITTED_BUILDS=str(self.cache))
        self.build()
        self.assertEqual(outside.read_text(), "keep these bytes\n")

    def test_a_symlinked_cache_directory_is_refused(self):
        real = self.scratch / "real"
        real.mkdir()
        self.target.mkdir()
        self.cache.symlink_to(real)
        done = self.build()
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("tools/admitted.sh: the cache path", done.stderr)
        self.assertEqual(list(real.iterdir()), [])

    def test_no_spelling_of_a_cache_path_reaches_through_a_symbolic_link(self):
        # A wire directory with data of its own, outside, reached through links or odd spellings.
        external = self.scratch / "external"
        (external / WIRE).mkdir(parents=True)
        (external / WIRE / "sentinel").write_text("keep\n")
        (self.scratch / "alias").symlink_to(external)
        (self.scratch / "parent-alias").symlink_to(self.scratch)
        real = self.scratch / "real"
        real.mkdir()
        spellings = [
            f"{self.scratch}/alias",
            f"{self.scratch}/alias/",
            f"{self.scratch}/alias//",
            f"{self.scratch}/alias/.",
            f"{self.scratch}/alias/./",
            f"{self.scratch}/alias/../alias",
            f"{self.scratch}/real/../alias",
            f"{self.scratch}/parent-alias/external",
            f"{self.scratch}/parent-alias/new-cache",
            f"{self.scratch}/real/",
            f"{self.scratch}//real",
            f"{self.scratch}/./real",
        ]
        for spelling in spellings:
            with self.subTest(spelling=spelling):
                done = self.build(ADMITTED_BUILDS=spelling)
                self.assertNotEqual(done.returncode, 0)
                self.assertIn("tools/admitted.sh: the cache path", done.stderr)
                self.assertEqual((external / WIRE / "sentinel").read_text(), "keep\n")
                self.assertEqual(list(real.iterdir()), [])
                self.assertFalse((external / "new-cache").exists())
        self.assertEqual(sorted(path.name for path in external.iterdir()), [WIRE])
        self.assertEqual(self.build(ADMITTED_BUILDS=str(real)).returncode, 0, "the plain spelling builds")

    def test_a_default_cache_under_a_symbolic_link_is_refused(self):
        real = self.scratch / "real-target"
        real.mkdir()
        (self.scratch / "linked-target").symlink_to(real)
        done = self.build(FAKE_TARGET=str(self.scratch / "linked-target"))
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("tools/admitted.sh: the cache path", done.stderr)
        self.assertEqual(list(real.iterdir()), [])

    def test_a_malformed_budget_is_refused_before_any_work(self):
        for budget in ["bananas", "", "0", "-5", "1e9", "99999999999999999999"]:
            with self.subTest(budget=budget):
                done = self.build(ADMITTED_MAX_BYTES=budget)
                self.assertNotEqual(done.returncode, 0)
                self.assertIn("ADMITTED_MAX_BYTES must be", done.stderr)
                self.assertFalse(self.cache.exists())

    def test_a_relative_cache_directory_is_refused(self):
        done = self.build(ADMITTED_BUILDS="relative/cache")
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("ADMITTED_BUILDS must be an absolute path", done.stderr)

    def test_a_corpus_naming_no_commit_or_no_wire_is_refused(self):
        manifest = self.repo / "crates/bota_server/tests/admitted" / WIRE / "corpus.txt"
        good = manifest.read_text()
        manifest.write_text("wire x\ncommit --upload-pack=evil\n")
        done = self.build()
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("names no commit", done.stderr)
        manifest.write_text(good)
        stray = self.repo / "crates/bota_server/tests/admitted/not-a-wire"
        stray.mkdir()
        (stray / "corpus.txt").write_text("commit 0123\n")
        done = self.build()
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("is not named by a wire", done.stderr)

    def test_a_failed_build_publishes_nothing_and_leaves_no_work(self):
        self.fake_tool(
            "cargo",
            FAKE_CARGO.replace(
                'if [ "$FAKE_BUILD" = fail ]; then',
                'if [ "$FAKE_BUILD" = fail ]; then\n mkdir -p "$CARGO_TARGET_DIR/release"\n'
                ' printf partial >"$CARGO_TARGET_DIR/release/bota_server"',
            ),
        )
        done = self.build(FAKE_BUILD="fail")
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse((self.cache / WIRE).exists())
        self.assertFalse((self.cache / f"{WIRE}.work").exists())

    def test_a_partial_copy_publishes_nothing(self):
        self.assertEqual(self.build().returncode, 0)
        (self.repo / PROGRAMS[1]).write_text("// force a stale build\n")
        self.fake_tool(
            "cp",
            '#!/usr/bin/env bash\nif [[ "${@: -1}" == */bin/examples/admitted_peer ]]; then '
            'printf partial >"${@: -1}"; exit 74; fi\nexec "$REAL_CP" "$@"\n',
        )
        done = self.build(REAL_CP=shutil.which("cp"))
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("made no examples/admitted_peer", done.stderr)
        self.assertFalse((self.cache / WIRE).exists())
        self.assertFalse((self.cache / f"{WIRE}.work").exists())

    def test_concurrent_runs_on_one_cache_build_one_at_a_time(self):
        guard = self.scratch / "building"
        overlap = self.scratch / "overlap"
        self.fake_tool(
            "cargo",
            FAKE_CARGO.replace(
                'if [ "$FAKE_BUILD" = fail ]; then',
                f'if ! mkdir "{guard}" 2>/dev/null; then touch "{overlap}"; exit 72; fi\n'
                f' trap \'rmdir "{guard}"\' EXIT\n sleep 0.2\n if [ "$FAKE_BUILD" = fail ]; then',
            ),
        )
        # Started together: one builds, the others wait for the cache and find its build.
        results = []
        runs = [threading.Thread(target=lambda: results.append(self.build())) for _ in range(3)]
        for run in runs:
            run.start()
        for run in runs:
            run.join()
        self.assertEqual([done.returncode for done in results], [0, 0, 0], [done.stderr for done in results])
        self.assertFalse(overlap.exists(), "two builds ran at once")


if __name__ == "__main__":
    unittest.main()
