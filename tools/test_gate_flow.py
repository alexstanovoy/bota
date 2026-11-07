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
    if os.environ.get("FLOW_FMT_FAILS"):
        sys.exit("error: expected item, found `}`")
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
        scope = os.environ.get("FLOW_SCOPE", scope)  # the real plan says full when a change selects everything
    print("groups", os.environ.get("FLOW_GROUPS", "core"))
    print("crates", os.environ.get("FLOW_CRATES", ""))
    print("fuzz", os.environ.get("FLOW_FUZZ", "0"))
    print("uncovered", os.environ.get("FLOW_UNCOVERED", ""))
    print("scope", scope)
elif arguments[0] == "available":
    print(os.environ.get("FLOW_AVAILABLE", ""))
elif arguments[0] == "check" and os.environ.get("FLOW_CHECK_FAILS"):
    sys.exit("test-groups: a test in no group")
elif arguments[0] == "run" and os.environ.get("FLOW_MUTATES"):
    Path("mutation.txt").write_text("written while the checks ran")
elif arguments[0] == "run" and os.environ.get("FLOW_FAILS") in arguments:
    Path(arguments[arguments.index("--record") + 1]).write_text(os.environ["FLOW_FAILS"] + "\\n")
    sys.exit(1)
"""


class GateFlow(unittest.TestCase):
    def setUp(self):
        scratch = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], cwd=TOOLS, text=True).strip()) / "temp"
        scratch.mkdir(exist_ok=True)
        directory = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.repo = self.root / "repo"
        for name in ("gate.sh", "precheck.sh", "stamp.sh", "steps.sh"):
            self.write(f"tools/{name}", (TOOLS / name).read_text(), executable=True)
        self.write("tools/test_groups.py", GROUPS)
        self.write("tools/ignored_audit.sh", "#!/bin/sh\n", executable=True)
        self.write("fuzz/fuzz.sh", '#!/bin/sh\necho "fuzz.sh $*" >> "$FLOW_TRACE"\n[ -z "$FLOW_FUZZ_FAILS" ]\n', executable=True)
        self.write("crates/bota_lua/fuzz/fuzz.sh", '#!/bin/sh\necho "lua-fuzz.sh $*" >> "$FLOW_TRACE"\n', executable=True)
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

    def test_a_change_that_selects_everything_is_a_full_check(self):
        self.passes("precheck", FLOW_SCOPE="full")
        self.passes("gate", GATE_FULL="1")

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

    def fuzz_checks(self):
        return [line for line in self.trace.read_text().splitlines() if line.startswith("fuzz.sh")]

    def test_the_precheck_compiles_the_fuzz_targets_when_the_plan_says_so(self):
        self.passes("precheck", FLOW_FUZZ="1")
        self.assertEqual(self.fuzz_checks(), ["fuzz.sh check"])

    def test_the_precheck_leaves_the_fuzz_targets_alone_when_nothing_they_build_on_changed(self):
        self.passes("precheck", FLOW_FUZZ="0")
        self.assertEqual(self.fuzz_checks(), [])

    def test_a_fuzz_target_that_does_not_compile_fails_the_precheck(self):
        self.refused("precheck", 1, "PRECHECK-FAIL fuzz-check", FLOW_FUZZ="1", FLOW_FUZZ_FAILS="1")

    def test_a_change_to_the_fuzz_targets_alone_is_still_checked_by_the_precheck(self):
        (self.repo / "crates/sample/src/extra.rs").unlink()
        self.write("fuzz/fuzz_targets/sample.rs", "fn main() {}\n")
        self.passes("precheck", FLOW_GROUPS="", FLOW_FUZZ="1")
        self.assertEqual(self.fuzz_checks(), ["fuzz.sh check"])

    def test_the_gate_compiles_the_fuzz_targets_when_the_plan_says_so_and_not_otherwise(self):
        self.passes("precheck", FLOW_FUZZ="1")
        self.trace.write_text("")
        self.passes("gate", FLOW_FUZZ="1")
        self.assertEqual(self.fuzz_checks(), ["fuzz.sh check"])
        self.passes("precheck", FLOW_FUZZ="0", PRECHECK_ALL="1")
        self.trace.write_text("")
        self.assertIn("fuzz-check: not affected", self.passes("gate", FLOW_FUZZ="0", GATE_AGAIN="1", GATE_PRECHECKED="1").stdout)
        self.assertEqual(self.fuzz_checks(), [])

    def traced(self, prefix):
        return [line for line in self.trace.read_text().splitlines() if line.startswith(prefix)]

    def test_the_bot_script_host_is_checked_with_its_own_manifest_where_the_plan_names_it(self):
        self.passes("precheck", FLOW_CRATES="bota_core bota_lua")
        self.assertEqual(self.traced("lua-fuzz.sh"), ["lua-fuzz.sh check"])
        self.assertIn("cargo clippy --manifest-path crates/bota_lua/Cargo.toml --all-targets --all-features -- -D warnings",
                      self.traced("cargo clippy"))
        self.assertIn("cargo clippy -p bota_core --all-targets -- -D warnings", self.traced("cargo clippy"))

    def test_the_bot_script_host_is_left_out_of_the_workspace_clippy_run(self):
        self.passes("precheck", FLOW_CRATES="bota_lua")
        self.assertEqual(self.traced("cargo clippy -p"), [])

    def test_the_bot_script_host_is_left_alone_where_the_plan_does_not_name_it(self):
        self.passes("precheck", FLOW_CRATES="bota_core")
        self.assertEqual(self.traced("lua-fuzz.sh"), [])
        self.assertFalse([line for line in self.traced("cargo") if "bota_lua" in line])

    def test_the_gate_denies_and_formats_the_bot_script_host_where_its_checkout_is_and_not_otherwise(self):
        self.passes("precheck", FLOW_AVAILABLE="bota_lua")
        self.passes("gate", FLOW_AVAILABLE="bota_lua")
        self.assertIn("cargo deny --config deny.toml --manifest-path crates/bota_lua/Cargo.toml check", self.traced("cargo deny"))
        self.assertIn("cargo fmt --manifest-path crates/bota_lua/Cargo.toml --all --check", self.traced("cargo fmt"))
        self.trace.write_text("")
        gate = self.passes("gate", GATE_AGAIN="1", GATE_PRECHECKED="1").stdout
        self.assertIn("deny-lua: not affected (no luagee checkout)", gate)
        self.assertIn("fmt-lua: not affected (no luagee checkout)", gate)
        self.assertEqual([line for line in self.traced("cargo") if "bota_lua" in line], [])

    def test_the_precheck_and_the_gate_name_what_is_not_covered_here(self):
        precheck = self.passes("precheck", FLOW_UNCOVERED="bota_lua").stdout
        self.assertIn("PRECHECK-OK (", precheck)
        self.assertRegex(precheck, r"PRECHECK-OK \(\d+ s; not covered: bota_lua\)")
        self.assertIn("precheck: NOT COVERED here: bota_lua", precheck)
        gate = self.passes("gate", FLOW_UNCOVERED="bota_lua").stdout
        self.assertIn("GATE-OK (affected: 1 groups; not covered: bota_lua)", gate)
        self.assertIn("gate: NOT COVERED here: bota_lua", gate)

    def test_nothing_is_said_about_coverage_where_everything_is_covered(self):
        self.assertNotIn("not covered", self.passes("precheck").stdout.lower())

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

    def test_the_groups_that_failed_run_first_next_time(self):
        self.refused("precheck", 1, "PRECHECK-FAIL test", FLOW_GROUPS="core server", FLOW_FAILS="server")
        self.trace.write_text("")
        self.passes("precheck", FLOW_GROUPS="core server")
        runs = [line.split()[-1] for line in self.trace.read_text().splitlines() if line.startswith("test_groups run")]
        self.assertEqual(runs, ["server", "core"])

    def test_a_tree_cargo_fmt_cannot_read_has_no_stamp(self):
        self.refused("precheck", 2, "`cargo fmt --all` failed", FLOW_FMT_FAILS="1")

    def test_without_its_base_the_precheck_fails_closed(self):
        self.git("update-ref", "-d", "refs/remotes/origin/main")  # and there is no main either
        self.refused("precheck", 2, "PRECHECK-FAIL plan")

    def test_a_documents_only_change_passes_the_gate_at_once_and_is_stamped(self):
        # The documents-only path once printed GATE-OK and then died on an unset variable, unstamped.
        (self.repo / "crates/sample/src/extra.rs").unlink()
        self.write("README.md", "bota, a simulator\n")
        self.passes("precheck")
        self.git("-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-qam", "words")
        self.trace.write_text("")
        self.assertIn("GATE-OK (documents only", self.passes("gate", GATE_PRECHECKED="1").stdout)
        self.assertNotIn("test_groups check", self.trace.read_text())
        self.refused("gate", 3, "already passed", GATE_PRECHECKED="1")


if __name__ == "__main__":
    unittest.main()
