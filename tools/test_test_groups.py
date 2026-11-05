"""tools/test_groups.py's routing, selection and guards, on this repository's own groups (tools/test_groups.toml)."""

import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import test_groups  # noqa: E402

CRATES, GROUPS, ROUTES = test_groups.load()
ROUTER = test_groups.Router(CRATES, GROUPS, ROUTES)
SERVER = "crates/bota_server/src"


def groups_of(*paths):
    return test_groups.plan(ROUTER, list(paths))[0]


class Routing(unittest.TestCase):
    def test_a_game_system_runs_its_group_the_pins_and_the_whole_matches_but_not_the_practice_map(self):
        self.assertEqual(groups_of(f"{SERVER}/game/systems/walk.rs"), {"game_movement", "game_determinism", "server_matches"})

    def test_a_change_to_a_game_test_runs_no_whole_match(self):
        self.assertEqual(groups_of(f"{SERVER}/game/systems/tests/pierce.rs"), {"game_combat", "game_determinism"})

    def test_the_practice_map_runs_for_the_server_around_the_game_and_for_what_everything_shares(self):
        self.assertIn("server_practice", groups_of(f"{SERVER}/net/conn.rs"))
        self.assertIn("server_practice", groups_of(f"{SERVER}/game/world.rs"))
        self.assertIn("server_practice", groups_of("crates/bota_core/src/lib.rs"))

    def test_a_game_test_file_runs_its_group_and_the_pins(self):
        self.assertEqual(groups_of(f"{SERVER}/game/tests/runes.rs"), {"game_objectives", "game_determinism"})

    def test_the_world_runs_every_server_group_and_radovid_which_launches_the_server(self):
        expected = {g.name for g in GROUPS if g.name.startswith(("game_", "server_"))} | {"radovid"}
        self.assertEqual(groups_of(f"{SERVER}/game/world.rs"), expected)

    def test_a_proto_change_runs_everything_built_on_it(self):
        self.assertEqual(groups_of("crates/bota_proto/src/lib.rs"), set(ROUTER.names) - {"import_items", "tooling"})

    def test_the_item_ids_run_the_importer(self):
        self.assertIn("import_items", groups_of("crates/bota_proto/registry/ids.txt"))

    def test_a_file_nothing_names_runs_everything(self):
        self.assertEqual(groups_of("somewhere/new.txt"), set(ROUTER.names))

    def test_the_gate_machinery_runs_everything(self):
        self.assertEqual(test_groups.plan(ROUTER, ["tools/gate.sh"]), (set(ROUTER.names), ROUTER.workspace(), True))

    def test_documents_run_nothing_and_lint_nothing(self):
        self.assertEqual(test_groups.plan(ROUTER, ["DEVELOPMENT.md", "crates/bota_server/README.md"]), (set(), set(), False))

    def test_a_server_change_lints_the_server_only(self):
        self.assertEqual(test_groups.plan(ROUTER, [f"{SERVER}/net/conn.rs"])[1], {"bota_server"})

    def test_the_test_certificate_runs_every_group_that_reads_it(self):
        self.assertTrue({"server_net", "server_launch", "server_transport"} <= groups_of("crates/bota_server/tests/tls/cert.pem"))

    def test_waves_and_camps_run_the_practice_map(self):
        self.assertIn("server_practice", groups_of(f"{SERVER}/game/systems/wave.rs"))
        self.assertIn("server_practice", groups_of(f"{SERVER}/game/systems/jungle.rs"))
        self.assertNotIn("server_practice", groups_of(f"{SERVER}/game/systems/walk.rs"))

    def test_globs_match_within_a_directory_with_one_star_and_across_with_two(self):
        self.assertTrue(test_groups.glob_regex("tools/*.sh").match("tools/gate.sh"))
        self.assertFalse(test_groups.glob_regex("tools/*.sh").match("tools/import_items/x.sh"))
        self.assertTrue(test_groups.glob_regex("**.md").match("crates/bota_core/README.md"))


class Selection(unittest.TestCase):
    def test_a_filter_selects_like_libtest_by_substring(self):
        selector = test_groups.Selector(None, "lib", ("game::tests::cast::",), (), (), ())
        self.assertTrue(test_groups.selects(selector, "lib", "game::tests::cast::a_cast_turns_first"))
        self.assertFalse(test_groups.selects(selector, "lib", "game::systems::tests::cast::a_cast_turns_first"))
        self.assertFalse(test_groups.selects(selector, "test:solo", "game::tests::cast::a_cast_turns_first"))

    def test_a_test_is_written_in_the_file_its_module_path_leads_to(self):
        listed = test_groups.Listed({}, {"lib": test_groups.ROOT / SERVER / "lib.rs"})
        written = lambda name: str(test_groups.test_file(listed, "lib", name).relative_to(test_groups.ROOT))  # noqa: E731
        self.assertEqual(written("game::tests::runes::a_rune_spawns"), f"{SERVER}/game/tests/runes.rs")
        self.assertEqual(written("game::world::fingerprint::every_part_moves_it"), f"{SERVER}/game/world.rs")
        self.assertEqual(written("game::systems::tests::raze::damage::x"), f"{SERVER}/game/systems/tests/raze/damage.rs")

    def test_the_scope_of_a_stamp_follows_the_base_and_the_selection(self):
        one = test_groups.scope_of("a" * 40, ["core"], ["bota_core"], True)
        self.assertNotEqual(one, test_groups.scope_of("b" * 40, ["core"], ["bota_core"], True))
        self.assertNotEqual(one, test_groups.scope_of("a" * 40, ["core", "proto"], ["bota_core"], True))
        self.assertEqual(test_groups.scope_of(None, ["core"], [], False), "full")

    def test_a_change_that_selects_everything_is_stamped_as_full(self):
        self.assertEqual(test_groups.plan_report(ROUTER, "a" * 40, ["tools/gate.sh"])[-1], "scope full")
        self.assertTrue(test_groups.plan_report(ROUTER, "a" * 40, [f"{SERVER}/net/conn.rs"])[-1].startswith("scope affected-"))


class Partition(unittest.TestCase):
    """check's verdicts on the tests a build holds (the listing libtest would give, made up here)."""

    BUILD = next(s.build for g in GROUPS if g.name == "game_movement" for s in g.selectors)

    def assess(self, groups, *names):
        listed = test_groups.Listed({"lib": list(names)}, {"lib": test_groups.ROOT / SERVER / "lib.rs"})
        return test_groups.assess(groups, ROUTER, {self.BUILD: listed})

    def group(self, name, *filters):
        return test_groups.Group(name, [], [], [test_groups.Selector(self.BUILD, "lib", filters, (), (), ())], [])

    def test_this_repositorys_movement_test_is_in_its_group(self):
        self.assertEqual(self.assess([self.group("game_movement", "game::tests::movement::")], "game::tests::movement::a_walk"), [])

    def test_a_test_in_two_groups_fails(self):
        groups = [self.group("game_movement", "game::tests::movement::"), self.group("game_combat", "::movement::")]
        problems = self.assess(groups, "game::tests::movement::a_walk")
        self.assertIn("bota_server lib game::tests::movement::a_walk: in 2 groups ['game_combat', 'game_movement'], must be in exactly one", problems)

    def test_a_test_in_no_group_fails(self):
        groups = [self.group("game_movement", "game::tests::movement::")]
        problems = self.assess(groups, "game::tests::movement::a_walk", "game::tests::new_system::a_rule")
        self.assertEqual(problems, ["bota_server lib game::tests::new_system::a_rule: in 0 groups [], must be in exactly one"])

    def test_a_test_whose_file_does_not_run_its_group_fails(self):
        problems = self.assess([self.group("game_lanes", "game::tests::movement::")], "game::tests::movement::a_walk")
        self.assertEqual(problems, [f"{SERVER}/game/tests/movement.rs: holds game::tests::movement::a_walk of group game_lanes, but a change to it does not run that group"])

    def test_a_filter_that_selects_nothing_fails(self):
        groups = [self.group("game_movement", "game::tests::movement::", "game::tests::gone::")]
        self.assertEqual(self.assess(groups, "game::tests::movement::a_walk"), ["group game_movement: the filter game::tests::gone:: selects no test"])

    def test_filters_without_a_target_are_refused(self):
        entry = {"name": "loose", "tests": [{"crate": "bota_server", "filters": ["net::"]}]}
        with self.assertRaisesRegex(test_groups.Failure, "filters bota_server without a target"):
            test_groups.load_group(entry, CRATES)


class Running(unittest.TestCase):
    def pool(self, *runs, jobs=2):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        record = Path(scratch.name) / "failed"
        pool = test_groups.Pool(jobs, record)
        with open(Path(scratch.name) / "out", "w") as out:
            stdout, sys.stdout = sys.stdout, out
            try:
                passed = pool.run(list(runs))
            finally:
                sys.stdout = stdout
        return passed, record.read_text() if record.exists() else "", (Path(scratch.name) / "out").read_text()

    def python(self, group, code, expected=None):
        return test_groups.Invocation(group, [sys.executable, "-c", code], Path.cwd(), {}, expected, {group})

    def test_a_run_of_fewer_tests_than_its_groups_select_fails_and_is_recorded(self):
        result = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out"
        passed, record, out = self.pool(self.python("core", f"print({result!r})", expected=2))
        self.assertFalse(passed)
        self.assertEqual(record, "core\n")
        self.assertIn("it ran 1 tests where its groups select 2", out)

    def test_the_first_failure_stops_the_runs_in_flight_and_the_rest(self):
        began = time.monotonic()
        passed, record, _ = self.pool(
            self.python("slow", "import time; time.sleep(60)"),
            self.python("broken", "raise SystemExit(3)"),
            self.python("never", "import time; time.sleep(60)"),
        )
        self.assertFalse(passed)
        self.assertEqual(record, "broken\n")
        self.assertLess(time.monotonic() - began, 30)

    def test_runs_at_once_pass_together(self):
        passed, _, out = self.pool(*[self.python(f"g{n}", "import time; time.sleep(1)") for n in range(4)], jobs=4)
        self.assertTrue(passed)
        self.assertEqual(out.count("test-groups: 1 s"), 4)


class Guards(unittest.TestCase):
    def in_scratch(self, files):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        for name, text in files.items():
            (Path(scratch.name) / name).parent.mkdir(parents=True, exist_ok=True)
            (Path(scratch.name) / name).write_text(text)
        root = test_groups.ROOT
        test_groups.ROOT = Path(scratch.name)
        self.addCleanup(setattr, test_groups, "ROOT", root)

    def test_a_test_target_with_its_own_harness_is_refused(self):
        self.in_scratch({"Cargo.toml": '[package]\nname = "x"\n[[test]]\nname = "custom"\nharness = false\n'})
        problems = test_groups.check_harnesses(CRATES, GROUPS, ["Cargo.toml"])
        self.assertEqual(problems, ["Cargo.toml: test custom has a harness of its own, which no group can list"])

    def test_a_bench_no_group_runs_is_refused(self):
        without_commands = [test_groups.Group(g.name, g.paths, g.also_paths, g.selectors, []) for g in GROUPS]
        problems = test_groups.check_harnesses(CRATES, without_commands, [])
        self.assertEqual(problems, ["bota_server: bench dummy is run by no group's command (tools/test_groups.toml)"])

    def test_a_test_where_no_group_runs_is_refused(self):
        self.in_scratch({"fuzz/src/lib.rs": "#[cfg(test)]\nmod tests {}\n", "scripts/test_seeds.py": "def test_seeds():\n    pass\n"})
        self.assertEqual(test_groups.check_markers(ROUTER, ["fuzz/src/lib.rs", "scripts/test_seeds.py"]), [
            "fuzz/src/lib.rs: holds tests (#[cfg(test)]) but a change to it runs no group, so none runs them",
            "scripts/test_seeds.py: holds tests (def test_) but a change to it runs no group, so none runs them",
        ])

    def test_this_repository_has_no_test_where_no_group_runs(self):
        self.assertEqual(test_groups.check_markers(ROUTER, test_groups.files()), [])

    def test_this_repository_has_no_unlistable_target(self):
        self.assertEqual(test_groups.check_harnesses(CRATES, GROUPS, test_groups.files()), [])


class Changes(unittest.TestCase):
    def test_a_moved_file_counts_at_both_its_paths(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        repo = Path(scratch.name)
        git = lambda *arguments: subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", *arguments],  # noqa: E731
                                                cwd=repo, check=True, capture_output=True)
        git("init", "-q", "-b", "work")
        (repo / "old.rs").write_text("fn moved() {}\n" * 20)
        git("add", ".")
        git("commit", "-qm", "base")
        git("mv", "old.rs", "new.rs")
        git("commit", "-qm", "move")
        root = test_groups.ROOT
        test_groups.ROOT = repo
        self.addCleanup(setattr, test_groups, "ROOT", root)
        self.assertEqual(test_groups.changed("HEAD~1")[1], ["new.rs", "old.rs"])


if __name__ == "__main__":
    unittest.main()
