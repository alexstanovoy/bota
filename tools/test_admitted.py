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
        )

    def test_a_verified_build_keeps_its_executables_wire_and_stamp_and_drops_the_rest(self):
        done = self.build()
        self.assertEqual(done.returncode, 0, done.stderr)
        entry = self.cache / WIRE
        self.assertEqual(sorted(path.name for path in entry.iterdir()), ["bin", "stamp", "wire"])
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


if __name__ == "__main__":
    unittest.main()
