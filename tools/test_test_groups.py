"""tools/test_groups.py's routing, selection and guards, on this repository's own groups (tools/test_groups.toml)."""

import io
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import patch
import os
import signal
import subprocess
import sys
import tempfile
import tomllib
import threading
import time
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import test_groups  # noqa: E402

CRATES, GROUPS, ROUTES = test_groups.load()
ROUTER = test_groups.Router(CRATES, GROUPS, ROUTES)
SERVER = "crates/bota_server/src"
# The bot-script host needs the luagee checkout: where it is absent, its group and lint are left out (see BotscriptAbsent).
LUA = {"lua"} if CRATES["bota_lua"].available else set()


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

    def test_the_world_runs_server_groups_and_consumers_with_real_matches(self):
        expected = {g.name for g in GROUPS if g.name.startswith(("game_", "server_"))} | {"radovid"} | LUA
        self.assertEqual(groups_of(f"{SERVER}/game/world.rs"), expected)

    def test_a_proto_change_runs_everything_built_on_it(self):
        self.assertEqual(groups_of("crates/bota_proto/src/lib.rs"), set(ROUTER.names) - {"import_items", "tooling"})

    def test_the_item_ids_run_the_importer(self):
        self.assertIn("import_items", groups_of("crates/bota_proto/registry/ids.txt"))

    def test_a_file_nothing_names_runs_everything(self):
        self.assertEqual(groups_of("somewhere/new.txt"), set(ROUTER.names))

    def test_the_gate_machinery_runs_everything(self):
        self.assertEqual(test_groups.plan(ROUTER, ["tools/gate.sh"]), (set(ROUTER.names), ROUTER.linted(), True))

    def test_documents_run_nothing_and_lint_nothing(self):
        self.assertEqual(test_groups.plan(ROUTER, ["DEVELOPMENT.md", "crates/bota_server/README.md"]), (set(), set(), False))

    def test_a_change_to_a_crate_the_fuzz_targets_build_on_checks_the_fuzz_targets(self):
        for crate in ("bota_proto", "bota_core", "bota_replay", "bota_server", "bota_client_core"):
            self.assertTrue(test_groups.plan(ROUTER, [f"crates/{crate}/src/lib.rs"])[2], crate)

    def test_a_change_to_the_fuzz_targets_checks_the_fuzz_targets(self):
        self.assertTrue(test_groups.plan(ROUTER, ["fuzz/fuzz_targets/server_orders.rs"])[2])

    def test_a_change_the_fuzz_targets_do_not_build_on_leaves_them_alone(self):
        for path in ("DEVELOPMENT.md", "crates/bota_radovid/src/lib.rs"):
            self.assertFalse(test_groups.plan(ROUTER, [path])[2], path)

    def test_a_server_change_lints_the_server_and_the_lua_match_consumer(self):
        self.assertEqual(test_groups.plan(ROUTER, [f"{SERVER}/net/conn.rs"])[1], {"bota_server"} | ({"bota_lua"} if LUA else set()))

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


class BotscriptAbsent(unittest.TestCase):
    """Where the luagee checkout bota_lua needs is absent (patched, so these hold with the checkout too)."""

    def load_without_luagee(self):
        stderr = io.StringIO()
        with patch.object(test_groups, "present", lambda requires: not requires), redirect_stderr(stderr):
            crates, groups, routes = test_groups.load()
        return test_groups.Router(crates, groups, routes), stderr.getvalue()

    def test_the_lua_group_is_left_out_and_said_so(self):
        router, notice = self.load_without_luagee()
        self.assertNotIn("lua", router.names)
        self.assertIn("group lua is not covered here: bota_lua needs ../luagee/crates/luagee/Cargo.toml, which is absent", notice)

    def test_a_change_to_the_server_lints_only_the_workspace(self):
        router, _ = self.load_without_luagee()
        self.assertEqual(test_groups.plan(router, [f"{SERVER}/net/conn.rs"])[1], {"bota_server"})

    def test_a_change_to_bota_lua_runs_and_lints_nothing(self):
        router, _ = self.load_without_luagee()
        self.assertEqual(test_groups.plan(router, ["crates/bota_lua/src/lib.rs"]), (set(), set(), False))

    def test_the_whole_plan_leaves_bota_lua_out(self):
        router, _ = self.load_without_luagee()
        groups, lint, _ = test_groups.plan(router, None)
        self.assertNotIn("lua", groups)
        self.assertNotIn("bota_lua", lint)

    def test_a_change_to_a_file_of_the_absent_crate_is_refused_rather_than_left_unchecked(self):
        router, _ = self.load_without_luagee()
        with self.assertRaises(test_groups.Failure) as raised:
            test_groups.plan_report(router, "a" * 40, ["crates/bota_lua/src/lib.rs"])
        self.assertEqual(str(raised.exception), "crates/bota_lua/src/lib.rs changed, but bota_lua cannot be built here: "
                         "../luagee/crates/luagee/Cargo.toml is missing. Check out that repository beside this one so that the change can be checked")

    def test_a_document_of_the_absent_crate_is_no_code_to_check(self):
        router, _ = self.load_without_luagee()
        self.assertEqual(test_groups.plan_report(router, "a" * 40, ["crates/bota_lua/README.md"])[1:4], ["crates", "fuzz 0", "uncovered bota_lua"])

    def test_the_plan_names_the_absent_crate_and_its_scope_is_never_full(self):
        router, _ = self.load_without_luagee()
        report = test_groups.plan_report(router, "a" * 40, ["tools/gate.sh"])
        self.assertIn("uncovered bota_lua", report)
        self.assertEqual(report[-1], "scope full-without-bota_lua")

    def test_the_tests_of_the_absent_crate_are_no_unrun_tests(self):
        router, _ = self.load_without_luagee()
        self.assertEqual(test_groups.check_markers(router, test_groups.files()), [])

    def test_a_machine_that_must_cover_everything_fails_on_the_missing_checkout(self):
        with patch.object(test_groups, "present", lambda requires: not requires), patch.dict(os.environ, {test_groups.REQUIRE_ALL: "1"}):
            with self.assertRaises(test_groups.Failure) as raised:
                test_groups.load()
        self.assertEqual(str(raised.exception), "crate bota_lua needs ../luagee/crates/luagee/Cargo.toml, which is absent, and TEST_GROUPS_REQUIRE_ALL is set")


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
        self.assertEqual(test_groups.plan_report(ROUTER, "a" * 40, ["tools/gate.sh"])[-1], "scope full" if LUA else "scope full-without-bota_lua")
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


class FeatureBuilds(unittest.TestCase):
    """Which builds a change lists, and the full check's proof that skipping the others is safe."""

    SERVER_CRATES = {"bota_server", "bota_radovid"}

    def listed(self, *paths):
        builds = test_groups.builds_to_list(CRATES, GROUPS, ROUTER, self.SERVER_CRATES, list(paths))
        return {b.features for b in builds if b.crate == "bota_server"}

    def test_a_game_change_lists_only_the_plain_server_build(self):
        self.assertEqual(self.listed(f"{SERVER}/game/systems/walk.rs"), {("websocket",)})

    def test_a_change_to_the_profiler_lists_its_feature_build(self):
        self.assertIn(("phase-profile", "websocket"), self.listed(f"{SERVER}/profile/tests.rs"))

    def test_a_changed_file_that_names_the_feature_lists_its_build(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        (Path(scratch.name) / f"{SERVER}/game/systems").mkdir(parents=True)
        (Path(scratch.name) / f"{SERVER}/game/systems/walk.rs").write_text('#[cfg(feature = "phase-profile")]\n#[test]\nfn timed() {}\n')
        root = test_groups.ROOT
        test_groups.ROOT = Path(scratch.name)
        self.addCleanup(setattr, test_groups, "ROOT", root)
        self.assertIn(("phase-profile", "websocket"), self.listed(f"{SERVER}/game/systems/walk.rs"))

    def test_a_file_only_a_feature_build_compiles_must_run_a_group_of_that_build(self):
        plain = test_groups.plain_build(CRATES["bota_server"])
        profiled = next(s.build for g in GROUPS if g.name == "server_profile" for s in g.selectors)
        listings = {
            plain: test_groups.Listed({}, {}, sources={f"{SERVER}/lib.rs"}),
            profiled: test_groups.Listed({}, {}, sources={f"{SERVER}/lib.rs", f"{SERVER}/profile/record.rs", f"{SERVER}/game/systems/walk.rs"}),
        }
        problems = test_groups.check_feature_files(CRATES, GROUPS, ROUTER, listings)
        self.assertEqual(problems, [f"{SERVER}/game/systems/walk.rs: only the bota_server build with phase-profile,websocket (test-release) "
                                    "compiles it, but a change to it runs no group of that build (server_profile)"])


class Proofs(unittest.TestCase):
    """The feature-build proof fails closed, and runs on a change as it does in full."""

    def scratch_root(self, files):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        for name, text in files.items():
            (Path(scratch.name) / name).parent.mkdir(parents=True, exist_ok=True)
            (Path(scratch.name) / name).write_text(text)
        root = test_groups.ROOT
        test_groups.ROOT = Path(scratch.name)
        self.addCleanup(setattr, test_groups, "ROOT", root)
        return Path(scratch.name)

    def test_dep_info_names_the_crates_files(self):
        root = self.scratch_root({"t.d": f"{SERVER}/../target/x: {SERVER}/lib.rs {SERVER}/net/mod.rs crates/bota_core/src/lib.rs\n"})
        self.assertEqual(test_groups.dep_info(root / "t.d", CRATES["bota_server"]), {f"{SERVER}/lib.rs", f"{SERVER}/net/mod.rs"})

    def test_a_missing_or_malformed_dep_info_fails_the_proof(self):
        root = self.scratch_root({"bad.d": "no sources here\n", "empty.d": ""})
        for name in ("missing.d", "bad.d", "empty.d"):
            with self.assertRaises(test_groups.Failure, msg=name):
                test_groups.dep_info(root / name, CRATES["bota_server"])

    def test_an_extra_build_without_its_plain_build_fails_the_proof(self):
        profiled = next(s.build for g in GROUPS if g.name == "server_profile" for s in g.selectors)
        with self.assertRaisesRegex(test_groups.Failure, "its plain build"):
            test_groups.check_feature_files(CRATES, GROUPS, ROUTER, {profiled: test_groups.Listed({}, {}, sources=set())})

    def test_a_change_the_plain_build_does_not_compile_is_proved_on_the_other_builds(self):
        plain = test_groups.plain_build(CRATES["bota_server"])
        walk = f"{SERVER}/game/systems/walk.rs"
        listed = []

        def fake_listing(crates, build):
            listed.append(build)
            sources = {f"{SERVER}/lib.rs"} | ({walk} if build != plain else set())
            return test_groups.Listed({}, {}, sources=sources)

        original = test_groups.list_build
        test_groups.list_build = fake_listing
        self.addCleanup(setattr, test_groups, "list_build", original)
        problems = test_groups.check_tests(CRATES, GROUPS, ROUTER, {"bota_server"}, [walk])
        self.assertIn(("phase-profile", "websocket"), {b.features for b in listed})
        self.assertTrue(any(p.startswith(f"{walk}: only the bota_server build with phase-profile") for p in problems), problems)

    def test_a_history_of_values_that_are_no_count_is_as_none(self):
        root = self.scratch_root({"h.json": '{"text": {"seconds": "4", "cores": 1}, "less": {"seconds": -1, "cores": 1}, '
                                            '"flag": {"seconds": true, "cores": 1}, "list": [1], "fine": {"seconds": 2.5, "cores": 0.5}}'})
        self.assertEqual(test_groups.read_history(root / "h.json"), {"fine": {"seconds": 2.5, "cores": 0.5}})
        for text in ('{"a": {"seconds": NaN, "cores": 1}}', '{"a": {"seconds": 1, "cores": Infinity}}', "[1, 2", "[1, 2]"):
            (root / "broken.json").write_text(text)
            self.assertEqual(test_groups.read_history(root / "broken.json"), {}, text)

    def test_the_history_is_replaced_whole(self):
        root = self.scratch_root({})
        test_groups.write_history(root / "h.json", {"a": {"seconds": 1.0, "cores": 2.0}})
        self.assertEqual(test_groups.read_history(root / "h.json"), {"a": {"seconds": 1.0, "cores": 2.0}})
        self.assertEqual(sorted(p.name for p in root.iterdir()), ["h.json"])

    def test_a_test_gated_on_a_feature_every_build_turns_on_is_refused(self):
        self.scratch_root({
            f"{SERVER}/net/a.rs": '#[cfg(all(test, not(feature = "websocket")))]\nmod tests {}\n',
            f"{SERVER}/net/b.rs": '#[cfg(not(feature = "websocket"))]\n#[test]\nfn plain() {}\n',
            f"{SERVER}/net/c.rs": '#[cfg(not(feature = "websocket"))]\nfn production() {}\n',
            f"{SERVER}/net/d.rs": '#[cfg(not(feature = "phase-profile"))]\n#[test]\nfn most_builds_run_it() {}\n',
        })
        problems = test_groups.check_negated_features(CRATES, GROUPS, [f"{SERVER}/net/{n}.rs" for n in "abcd"])
        self.assertEqual([p.split(":")[0] for p in problems], [f"{SERVER}/net/a.rs", f"{SERVER}/net/b.rs"])


class Running(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.scratch = Path(scratch.name)
        self.record = self.scratch / "failed"

    def pool(self, share=2):
        return test_groups.Pool(share, self.record, self.scratch / "logs")

    def run_pool(self, *runs, share=2):
        with open(self.scratch / "out", "w") as out:
            stdout, sys.stdout = sys.stdout, out
            try:
                passed = self.pool(share).run(list(runs))
            finally:
                sys.stdout = stdout
        return passed, (self.scratch / "out").read_text()

    def python(self, group, code, expected=None):
        return test_groups.Invocation(group, [sys.executable, "-c", code], self.scratch, {}, expected, {group})

    def recorded(self):
        return self.record.read_text() if self.record.exists() else ""

    def test_a_run_of_fewer_tests_than_its_groups_select_fails_and_is_recorded(self):
        result = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out"
        passed, out = self.run_pool(self.python("core", f"print({result!r})", expected=2))
        self.assertFalse(passed)
        self.assertEqual(self.recorded(), "core\n")
        self.assertIn("it ran 1 tests where its groups select 2", out)

    def test_a_binary_that_cannot_start_fails_its_groups(self):
        missing = test_groups.Invocation("gone", [str(self.scratch / "no-such-binary")], self.scratch, {}, 1, {"core"})
        passed, out = self.run_pool(missing)
        self.assertFalse(passed)
        self.assertEqual(self.recorded(), "core\n")
        self.assertIn("(the runner: FileNotFoundError", out)

    def test_skipped_unittests_fail_and_record_their_group(self):
        for ordinary in (False, True):
            with self.subTest(ordinary=ordinary):
                code = (
                    "import unittest\n"
                    "class Contracts(unittest.TestCase):\n"
                    "    @unittest.skip('required dependency unavailable')\n"
                    "    def test_required(self): self.fail('never executed')\n"
                    + ("    def test_ordinary(self): pass\n" if ordinary else "")
                    + "unittest.main()\n"
                )
                passed, out = self.run_pool(self.python("tooling", code, expected=1 + ordinary))
                self.assertFalse(passed, out)
                self.assertEqual(self.recorded(), "tooling\n")
                self.assertIn("1 skipped", out)

    def test_ignored_or_measured_rust_tests_fail_instead_of_counting_as_executed(self):
        for ignored, measured in ((1, 0), (0, 1)):
            with self.subTest(ignored=ignored, measured=measured):
                summary = f"test result: ok. 0 passed; 0 failed; {ignored} ignored; {measured} measured; 0 filtered out"
                passed, out = self.run_pool(self.python("core", f"print({summary!r})", expected=1))
                self.assertFalse(passed, out)
                self.assertEqual(self.recorded(), "core\n")
                self.assertIn(f"0 failed, {ignored} ignored, {measured} measured tests", out)

    def local_conformance_run(self):
        name = "local_scripts_play_a_complete_match_without_script_failures"
        target_name = "test:local_scripts"
        build = test_groups.Build("bota_lua", test_groups.PROFILE, ("native",))
        listed = test_groups.Listed({target_name: [name, "another_contract"]}, {},
                                    executables={target_name: Path("/test-binary")})
        target = test_groups.Target(build, target_name, (), (), ())
        return test_groups.invocation(CRATES, listed, target, {"lua"}, {})

    def test_optional_local_conformance_is_reported_and_only_its_exact_name_is_excluded(self):
        output = io.StringIO()
        with patch.dict(os.environ, {"BOTA_LUA_LOCAL_PACK": ""}), redirect_stdout(output):
            run = self.local_conformance_run()
        self.assertEqual(run.expected, 1)
        self.assertIn("--skip", run.command)
        self.assertNotIn("another_contract", run.command[run.command.index("--skip") + 1:])
        self.assertIn("optional local conformance not run", output.getvalue())
        self.assertIn("tools/test_lua_local.sh", output.getvalue())

    def test_configured_local_conformance_executes_and_counts_the_ignored_contract(self):
        with patch.dict(os.environ, {"BOTA_LUA_LOCAL_PACK": "/local-sources"}):
            run = self.local_conformance_run()
        self.assertEqual(run.expected, 2)
        self.assertIn("--include-ignored", run.command)
        self.assertNotIn("--skip", run.command)

    def test_manual_recorders_are_exempted_by_name_without_exempting_their_file(self):
        build = Partition.BUILD
        names = ["game::tests::determinism::print_pins", "game::tests::determinism::print_blind_pins",
                 "game::tests::determinism::new_contract"]
        listed = test_groups.Listed({"lib": names}, {"lib": test_groups.ROOT / SERVER / "lib.rs"},
                                    executables={"lib": Path("/test-binary")})
        target = test_groups.Target(build, "lib", (), (), ())
        run = test_groups.invocation(CRATES, listed, target, {"game_determinism"}, {})
        self.assertEqual(run.expected, 1)
        skipped = [run.command[i + 1] for i, word in enumerate(run.command[:-1]) if word == "--skip"]
        self.assertCountEqual(skipped, names[:2])
        self.assertNotIn(names[2], skipped)

    def test_manual_recorders_skip_only_exact_full_names_in_real_libtest(self):
        source = self.scratch / "contracts.rs"
        binary = self.scratch / "contracts"
        source.write_text('''mod game { mod tests { mod determinism {
            #[test] #[ignore] fn print_pins() { panic!("manual recorder executed"); }
            #[test] #[ignore] fn print_blind_pins() { panic!("manual recorder executed"); }
            #[test] fn print_pins_preserves_the_purchase_contract() {}
            #[test] fn ordinary_contract() {}
        } } }''')
        subprocess.run(["rustc", "--edition", "2024", "--test", "-O", str(source), "-o", str(binary)],
                       capture_output=True, text=True, check=True)
        names = ["game::tests::determinism::" + name for name in (
            "print_pins", "print_blind_pins", "print_pins_preserves_the_purchase_contract", "ordinary_contract")]
        listed = test_groups.Listed({"lib": names}, {"lib": source}, executables={"lib": binary})
        target = test_groups.Target(Partition.BUILD, "lib", ("game::tests::determinism::",), (), ())
        run = test_groups.invocation(CRATES, listed, target, {"game_determinism"}, {})
        passed, out = self.run_pool(run)
        self.assertTrue(passed, out)
        self.assertIn("2 passed; 0 failed; 0 ignored", out)

    def test_output_that_is_not_utf8_is_told_and_counted(self):
        code = "import sys; sys.stdout.buffer.write(b'\\xff\\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured\\n')"
        passed, out = self.run_pool(self.python("core", code, expected=1))
        self.assertTrue(passed, out)

    def test_runs_at_once_meet_at_a_barrier(self):
        # each run marks its arrival and waits for the others: only runs in flight together can all pass
        wait = (
            "import pathlib, sys, time\n"
            "here = pathlib.Path(sys.argv[1]); (here / sys.argv[2]).touch()\n"
            "deadline = time.monotonic() + 120\n"
            "while len(list(here.glob('arrived-*'))) < 3:\n"
            "    if time.monotonic() > deadline: sys.exit('the others never came')\n"
            "    time.sleep(0.05)\n"
        )
        runs = [test_groups.Invocation(f"g{n}", [sys.executable, "-c", wait, str(self.scratch), f"arrived-{n}"], self.scratch, {}, None, {f"g{n}"})
                for n in range(3)]
        passed, out = self.run_pool(*runs, share=3)
        self.assertTrue(passed, out)

    def test_the_first_failure_stops_the_runs_in_flight_and_the_rest(self):
        finished = self.scratch / "finished"
        slow = f"import time; time.sleep(60); open({str(finished)!r}, 'w')"
        passed, _ = self.run_pool(self.python("slow", slow), self.python("broken", "raise SystemExit(3)"), self.python("never", slow))
        self.assertFalse(passed)
        self.assertEqual(self.recorded(), "broken\n")
        self.assertFalse(finished.exists())

    def test_the_first_failure_is_told_last(self):
        passed, out = self.run_pool(self.python("fine", "pass"), self.python("broken", "raise SystemExit(3)"), share=1)
        self.assertFalse(passed)
        self.assertTrue(out.rstrip().splitlines()[-1].startswith("test-groups: FAILED after"), out)
        self.assertIn("groups broken", out.rstrip().splitlines()[-1])

    def test_only_the_first_failure_is_recorded(self):
        pool = self.pool()
        with open(self.scratch / "out", "w") as out:
            stdout, sys.stdout = sys.stdout, out
            try:
                with pool.lock:
                    pool.fail("first", {"core"})
                    pool.fail("second", {"server_net"})
            finally:
                sys.stdout = stdout
        self.assertEqual((pool.failure, self.recorded()), ("first", "core\n"))

    def test_a_run_that_would_start_after_a_failure_does_not_start(self):
        started = self.scratch / "started"
        pool = self.pool()
        pool.failure = "an earlier failure"
        pool.take(1)
        pool.execute(self.python("late", f"open({str(started)!r}, 'w')"), self.scratch / "late.log")
        self.assertFalse(started.exists())

    def test_sigterm_stops_every_binary_and_what_it_started(self):
        child = self.scratch / "child"
        code = (
            "import subprocess, sys, time\n"
            "server = subprocess.Popen(['sleep', '300'])\n"
            f"open({str(child)!r}, 'w').write(str(server.pid))\n"
            "time.sleep(300)\n"
        )

        def terminate_once_started():
            deadline = time.monotonic() + 120
            while not (child.exists() and child.read_text()) and time.monotonic() < deadline:
                time.sleep(0.05)
            os.kill(os.getpid(), signal.SIGTERM)

        threading.Thread(target=terminate_once_started, daemon=True).start()
        with self.assertRaises(SystemExit) as stopped:
            self.run_pool(self.python("server", code))
        self.assertEqual(stopped.exception.code, 128 + signal.SIGTERM)
        self.assertEqual(self.recorded(), "")
        deadline = time.monotonic() + 30  # a hang guard: the kill is sent before run returns
        while Path(f"/proc/{child.read_text()}").exists() and "Z" not in Path(f"/proc/{child.read_text()}/stat").read_text().split()[2]:
            self.assertLess(time.monotonic(), deadline, "the server a binary started outlived the run")
            time.sleep(0.05)


class Share(unittest.TestCase):
    def test_runs_wait_while_the_share_has_no_room_for_their_threads(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        runs = [test_groups.Invocation(f"g{n}", [sys.executable, "-c", "pass"], Path(scratch.name), {}, None, {f"g{n}"}, cost=2)
                for n in range(5)]
        pool = test_groups.Pool(4, None, Path(scratch.name) / "logs")
        stdout, sys.stdout = sys.stdout, io.StringIO()
        try:
            passed = pool.run(runs)
        finally:
            sys.stdout = stdout
        self.assertTrue(passed)
        self.assertLessEqual(pool.most, 4)

    def test_a_run_holds_the_cpus_it_used_last_time_within_its_threads(self):
        self.assertEqual(test_groups.cost_of(None, 12), 12)
        self.assertEqual(test_groups.cost_of({"seconds": 50.0, "cores": 0.2}, 12), 1)
        self.assertEqual(test_groups.cost_of({"seconds": 30.0, "cores": 9.3}, 12), 10)
        self.assertEqual(test_groups.cost_of({"seconds": 30.0, "cores": 31.0}, 12), 12)

    def test_a_run_larger_than_the_share_runs_alone(self):
        pool = test_groups.Pool(4, None, Path(tempfile.gettempdir()))
        pool.take(16)
        self.assertEqual(pool.used, 16)  # it did not wait: nothing else ran
        pool.give(16)

    def test_heavy_cpus_bounds_the_share(self):
        environment = os.environ.get("HEAVY_CPUS")
        self.addCleanup(lambda: os.environ.__setitem__("HEAVY_CPUS", environment) if environment else os.environ.pop("HEAVY_CPUS", None))
        os.environ["HEAVY_CPUS"] = "2"
        self.assertLessEqual(test_groups.cpu_share(), 2)

    def test_a_share_that_is_no_number_is_refused(self):
        result = subprocess.run([sys.executable, "-B", str(test_groups.ROOT / "tools/test_groups.py"), "run", "core"],
                                env=os.environ | {"TEST_GROUPS_CPUS": "four"}, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("TEST_GROUPS_CPUS 'four': a whole number of at least one", result.stderr)


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

    def test_bota_lua_repeats_the_workspace_settings_it_cannot_inherit(self):
        root = tomllib.loads((test_groups.ROOT / "Cargo.toml").read_text())
        lua = tomllib.loads((test_groups.ROOT / "crates/bota_lua/Cargo.toml").read_text())
        self.assertEqual(lua["profile"], root["profile"])
        self.assertEqual(lua["workspace"]["lints"], root["workspace"]["lints"])
        for key, value in lua["workspace"]["package"].items():
            self.assertEqual(value, root["workspace"]["package"][key], f"workspace.package.{key}")

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
