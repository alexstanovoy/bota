"""Contracts for the bounded, source-only bot API census."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("bot_api_census", Path(__file__).with_name("bot_api_census.py"))
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)


class Census(unittest.TestCase):
    def test_source_fingerprint_uses_exact_bytes_and_includes_unreached_modules(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'bot_generic.lua').write_text('function Think() end')
            extra = root / 'extra.lua'
            extra.write_bytes(b'--\xff')
            before = census.scan(root, {})['source_fingerprint']
            extra.write_bytes(b'--\xfe')
            after = census.scan(root, {})['source_fingerprint']
            self.assertNotEqual(before, after)
            self.assertEqual(after, census.scan(root, {})['source_fingerprint'])

    def test_ruleset_unavailable_is_distinct_from_missing_bindings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'bot_generic.lua').write_text('GetBot():HasBuyback(); GetBot():MissingMethod()')
            report = census.scan(root, {'global': {'GetBot'}, 'unit': {'HasBuyback'}, 'ruleset_unavailable_unit': {'HasBuyback', 'MissingMethod'}})
            self.assertEqual(report['entrypoints'], ['bots/bot_generic.lua'])
            self.assertEqual(report['ruleset_unavailable_api_names'], 1)
            self.assertIn('HasBuyback', report['ruleset_unavailable']['member'])
            self.assertNotIn('HasBuyback', report['missing_members'])
            self.assertIn('MissingMethod', report['missing_members'])

    def test_block_local_helpers_do_not_hide_calls_after_their_scope(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'helper.lua').write_text('return {}')
            (root / 'bot_generic.lua').write_text('do local bot=require("helper") end; local bot=GetBot(); bot:MissingMethod(); do local MissingGlobal=function() end end; MissingGlobal()')
            report = census.scan(root, {'global': {'GetBot'}})
            self.assertIn('MissingMethod', report['missing_members'])
            self.assertIn('MissingGlobal', report['missing_globals'])

    def test_computed_environment_dispatch_is_reported_without_a_completeness_claim(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'bot_generic.lua').write_text('_G["req".."uire"]("hidden")')
            report = census.scan(root, {})
            self.assertNotIn('closure_complete', report)
            self.assertFalse(report['imports_resolved'])

    def test_hero_substitutions_have_a_small_identifier_bound(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, 'short ASCII key'):
                census.scan(Path(directory), {}, hero='a' * 10000)

    def test_catalogue_fifo_and_malformed_text_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'catalogue'
            os.mkfifo(path)
            with self.assertRaisesRegex(ValueError, 'nonregular'):
                census.read_catalogue(path)
            path.unlink()
            path.write_bytes(b'global\tGetBot\tbad\n')
            with self.assertRaises(ValueError):
                census.read_catalogue(path)
            path.write_bytes(b'\xff')
            with self.assertRaises(UnicodeError):
                census.read_catalogue(path)

    def test_import_work_and_token_budgets_are_enforced(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'bot_generic.lua').write_text('require(GetScriptDirectory().."/helper")')
            with patch.object(census, 'MAX_ANALYSIS_WORK', 8):
                with self.assertRaisesRegex(ValueError, 'work limit'):
                    census.scan(root, {})
            with patch.object(census, 'MAX_TOKENS', 4):
                with self.assertRaisesRegex(ValueError, 'tokens limit'):
                    census.scan(root, {})
    def test_strings_are_not_syntax_and_escaped_imports_are_resolved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bot_generic.lua").write_text(r'local s="function" MissingGlobal(); require("he\108per"); require(")")')
            (root / "helper.lua").write_text('MissingHelperApi()')
            report = census.scan(root, {})
            self.assertIn("MissingGlobal", report["missing_globals"])
            self.assertIn("MissingHelperApi", report["missing_globals"])
            self.assertEqual(len(report["unresolved_dependencies"]), 1)

    def test_nested_import_analysis_is_bounded_before_quadratic_work(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bot_generic.lua").write_text('require(' * 1000 + '"helper"' + ')' * 1000)
            with self.assertRaisesRegex(ValueError, "import expression limit"):
                census.scan(root, {})

    def test_pack_and_runtime_share_the_source_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "pack"
            runtime = Path(directory) / "runtime"
            root.mkdir()
            runtime.mkdir()
            (root / "bot_generic.lua").write_text('123456')
            (runtime / "shared.lua").write_text('123456')
            with patch.object(census, 'MAX_TOTAL', 8):
                with self.assertRaisesRegex(ValueError, "source size"):
                    census.scan(root, {}, runtime=runtime)

    def test_loader_alias_prevents_a_false_complete_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bot_generic.lua").write_text('local import=require; import("hidden")')
            (root / "hidden.lua").write_text('MissingHiddenApi()')
            report = census.scan(root, {})
            self.assertFalse(report["imports_resolved"])
            self.assertTrue(report["unresolved_dependencies"])

    def test_helper_definition_cannot_hide_an_unrelated_engine_member_gap(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bot_generic.lua").write_text('local M=require("helper"); GetBot():MissingMethod()')
            (root / "helper.lua").write_text('local M={} function M.MissingMethod() end return M')
            report = census.scan(root, {"global": {"GetBot"}})
            self.assertIn("MissingMethod", report["missing_members"])

    def test_runtime_and_pack_modules_keep_distinct_namespaces(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "pack"
            runtime = Path(directory) / "runtime"
            (root / "game").mkdir(parents=True)
            runtime.mkdir()
            (root / "bot_generic.lua").write_text('require("bots/game/shared"); require("game/shared")')
            (root / "game/shared.lua").write_text('MissingPackApi()')
            (runtime / "shared.lua").write_text('GetBot()')
            report = census.scan(root, {"global": {"GetBot"}}, runtime=runtime)
            self.assertIn("MissingPackApi", report["missing_globals"])
            self.assertIn("bots/game/shared.lua", report["files"])
            self.assertIn("game/shared.lua", report["files"])

    def test_sf_entry_closure_excludes_other_heroes_and_resolves_hero_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "hero").mkdir()
            (root / "bot_nevermore.lua").write_text('local h=require(GetScriptDirectory().."/hero/"..string.gsub(GetBot():GetUnitName(),"npc_dota_hero_",""))\nfunction Think() GetBot():GetHealth(); MissingGlobal(); end')
            (root / "bot_lina.lua").write_text('OtherHeroOnly()')
            (root / "hero/nevermore.lua").write_text('-- FakeComment()\nlocal s="FakeString()"\nGetBot():MissingMethod()')
            report = census.scan(root, {"global": {"GetBot", "GetScriptDirectory"}, "unit": {"GetHealth", "GetUnitName"}, "string": {"gsub"}})
            self.assertEqual(report["files"], ["bots/bot_nevermore.lua", "bots/hero/nevermore.lua"])
            self.assertEqual(set(report["missing_globals"]), {"MissingGlobal"})
            self.assertEqual(set(report["missing_members"]), {"MissingMethod"})
            self.assertFalse(report["unresolved_dependencies"])
            self.assertGreater(report["api_coverage_percent"], 0)
            self.assertLess(report["api_coverage_percent"], 100)

    def test_mode_hook_fallbacks_are_distinct_from_api_presence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "mode_laning_generic.lua").write_text('function GetDesire() return .4 end')
            (root / "mode_attack_generic.lua").write_text('function GetDesire() return nil end\nfunction Think() GetBot():Action_AttackUnit(nil,false) end')
            report = census.scan(root, {"global": {"GetBot"}, "unit": {"Action_AttackUnit"}})
            self.assertEqual(report["api_coverage_percent"], 100)
            self.assertIn("Think", report["default_hooks"]["laning"])
            self.assertIn("GetDesire", report["default_hooks"]["attack"])
            self.assertNotIn("Think", report["default_hooks"]["attack"])

    def test_dynamic_imports_and_script_helpers_are_not_silently_counted_as_supported_api(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bot_generic.lua").write_text('local Util=require("bots/helper")\nfunction Think() Util:GetSomething(); require(prefix..name); GetBot():GetHealth() end')
            (root / "helper.lua").write_text('local M={} function M:GetSomething() end local Deferred=Factory(function() end) function M.Run() Deferred() end return M')
            report = census.scan(root, {"global": {"GetBot"}, "unit": {"GetHealth"}}, reference={"global": {"GetBot"}, "unit": {"GetHealth"}})
            self.assertEqual(report["missing_members"], {})
            self.assertNotIn("Deferred", report["missing_globals"])
            self.assertIn("Deferred", report["unclassified_calls"]["global"])
            self.assertTrue(report["unresolved_dependencies"])
            self.assertFalse(report["imports_resolved"])

    def test_symlink_and_oversized_sources_are_refused_before_reading(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bot_generic.lua").symlink_to("/dev/zero")
            with self.assertRaisesRegex(ValueError, "symbolic"):
                census.scan(root, {})
            (root / "bot_generic.lua").unlink()
            with (root / "bot_generic.lua").open("wb") as source:
                source.truncate(census.MAX_SOURCE + 1)
            with self.assertRaisesRegex(ValueError, "source size"):
                census.scan(root, {})
