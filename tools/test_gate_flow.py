"""The gate's and the precheck's flow (stamps, selection, refusals), run on a scratch repository with cargo and
tools/test_groups.py replaced by stand-ins: no Rust is built."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
INSTALL = ".steam/steam/steamapps/common/dota 2 beta/game/dota/pak01_dir.vpk"

# `cargo fmt --emit stdout` is what tree_key hashes the crates' Rust by; every other call is only traced.
CARGO = """#!/usr/bin/env python3
import os, sys
if sys.argv[1:2] == ["fmt"] and "--emit" in sys.argv:
    print("fn sample() {}")
with open(os.environ["FLOW_TRACE"], "a") as trace:
    print("cargo", *sys.argv[1:], file=trace)
"""

# plan prints what FLOW_GROUPS asks for and a scope made like the real one; check and run succeed unless told otherwise.
GROUPS = """import hashlib, os, subprocess, sys
from pathlib import Path
arguments = sys.argv[1:]
with open(os.environ["FLOW_TRACE"], "a") as trace:
    print("test_groups", *arguments, file=trace)
if arguments[0] == "plan":
    if "--all" in arguments:
        scope = "full"
    else:
        base = arguments[arguments.index("--base") + 1]
        found = subprocess.run(["git", "merge-base", "HEAD", base], capture_output=True, text=True)
        if found.returncode != 0:
            sys.exit(found.stderr)
        groups = os.environ.get("FLOW_GROUPS", "core")
        scope = "affected-" + hashlib.sha256((found.stdout + groups).encode()).hexdigest()[:16]
    print("groups", os.environ.get("FLOW_GROUPS", "core"))
    print("crates")
    print("fuzz 0")
    print("scope", scope)
elif arguments[0] == "check" and os.environ.get("FLOW_CHECK_FAILS"):
    sys.exit("test-groups: a test in no group")
elif arguments[0] == "run" and os.environ.get("FLOW_MUTATES"):
    Path("mutation.txt").write_text("written while the checks ran")
"""


class GateFlow(unittest.TestCase):
    def setUp(self):
        scratch = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], cwd=TOOLS, text=True).strip()) / "temp"
        scratch.mkdir(exist_ok=True)
        directory = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.repo = self.root / "repo"
        for name in ("gate.sh", "precheck.sh", "stamp.sh"):
            self.write(f"tools/{name}", (TOOLS / name).read_text(), executable=True)
        self.write("tools/test_groups.py", GROUPS)
        self.write("tools/ignored_audit.sh", "#!/bin/sh\n", executable=True)
        self.write("tools/import_items/src/main.rs", "fn main() {}\n")
        self.write("crates/sample/src/lib.rs", "fn sample() {}\n")
        self.write(".gitignore", "/temp/\n")
        self.write("README.md", "bota\n")
        (self.root / "bin").mkdir()
        (self.root / "bin/cargo").write_text(CARGO)
        (self.root / "bin/cargo").chmod(0o755)
        (self.root / "home" / INSTALL).parent.mkdir(parents=True)
        (self.root / "home" / INSTALL).touch()
        self.trace = self.root / "trace"
        self.trace.touch()
        clean = {k: v for k, v in os.environ.items() if not k.startswith(("GATE_", "PRECHECK_", "FLOW_"))}
        self.environment = clean | {
            "PATH": f"{self.root / 'bin'}{os.pathsep}{os.environ['PATH']}",
            "HOME": str(self.root / "home"),
            "FLOW_TRACE": str(self.trace),
            "GIT_CONFIG_GLOBAL": os.devnull,
        }
        self.git("init", "-q", "-b", "work")
        self.git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "add", ".")
        self.git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qm", "base")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.write("crates/sample/src/extra.rs", "fn extra() {}\n")  # the change under check

    def write(self, name, text, executable=False):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        if executable:
            path.chmod(0o755)

    def git(self, *arguments):
        return subprocess.check_output(["git", *arguments], cwd=self.repo, env=self.environment, text=True)

    def run_script(self, script, **environment):
        return subprocess.run(["bash", f"tools/{script}.sh"], cwd=self.repo, env=self.environment | environment,
                              capture_output=True, text=True)

    def passes(self, script, **environment):
        result = self.run_script(script, **environment)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def refused(self, script, code, message, **environment):
        result = self.run_script(script, **environment)
        self.assertEqual(result.returncode, code, result.stdout + result.stderr)
        self.assertIn(message, result.stdout + result.stderr)

    def test_the_gate_starts_only_after_a_precheck_and_runs_once_per_tree(self):
        self.refused("gate", 3, "no passing precheck")
        self.passes("precheck")
        self.assertIn("GATE-OK (affected: 1 groups)", self.passes("gate").stdout)
        self.refused("gate", 3, "already passed")

    def test_an_affected_precheck_does_not_admit_a_full_gate(self):
        self.passes("precheck")
        self.refused("gate", 3, "no passing precheck", GATE_FULL="1")

    def test_a_full_precheck_admits_both_gates_and_a_full_pass_stands_for_an_affected_one(self):
        self.passes("precheck", PRECHECK_ALL="1")
        self.assertIn("GATE-OK (full: 1 groups)", self.passes("gate", GATE_FULL="1").stdout)
        self.refused("gate", 3, "already passed")

    def test_a_precheck_of_another_selection_does_not_admit_the_gate(self):
        self.passes("precheck")
        self.refused("gate", 3, "no passing precheck", FLOW_GROUPS="core server")

    def test_a_new_merge_base_needs_a_new_precheck(self):
        self.passes("precheck")
        self.git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "--allow-empty", "-qm", "later")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        self.refused("gate", 3, "no passing precheck")

    def test_a_change_to_rust_outside_the_crates_needs_a_new_precheck(self):
        self.passes("precheck")
        self.write("tools/import_items/src/main.rs", "fn main() { todo!() }\n")
        self.refused("gate", 3, "no passing precheck")

    def test_any_step_left_out_fails_the_gate_and_stamps_nothing(self):
        self.passes("precheck")
        for skip in ("deny", "a-step-that-does-not-exist"):
            self.refused("gate", 1, "GATE-FAIL steps were skipped", GATE_SKIP=skip)
        self.passes("gate")

    def test_a_failing_group_check_fails_the_gate(self):
        self.passes("precheck")
        self.refused("gate", 1, "GATE-FAIL test-groups", FLOW_CHECK_FAILS="1")

    def test_a_tree_changed_while_the_checks_ran_gets_no_stamp(self):
        self.refused("precheck", 1, "PRECHECK-FAIL the tree changed", FLOW_MUTATES="1")
        (self.repo / "mutation.txt").unlink()
        self.passes("precheck")
        self.refused("gate", 1, "GATE-FAIL the tree changed", FLOW_MUTATES="1")
        stamps = (self.repo / ".git/checked-trees").glob("*")
        self.assertFalse(any(" gate " in f" {stamp.read_text()}" for stamp in stamps))

    def test_the_importer_needs_a_dota_install_only_when_its_group_runs(self):
        (self.root / "home" / INSTALL).unlink()
        self.passes("precheck")
        self.passes("gate")
        self.passes("precheck", FLOW_GROUPS="import_items")
        self.refused("gate", 1, "no Dota 2 install", FLOW_GROUPS="import_items")

    def test_without_its_base_the_precheck_fails_closed(self):
        self.git("update-ref", "-d", "refs/remotes/origin/main")  # and there is no main either
        self.refused("precheck", 2, "PRECHECK-FAIL plan")

    def test_a_documents_only_change_passes_the_gate_without_running_checks(self):
        (self.repo / "crates/sample/src/extra.rs").unlink()
        self.write("README.md", "bota, a simulator\n")
        self.passes("precheck")
        self.assertIn("documents only", self.passes("gate").stdout)


if __name__ == "__main__":
    unittest.main()
