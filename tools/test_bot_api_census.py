"""Contracts for the bounded, source-only bot API census."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("bot_api_census", Path(__file__).with_name("bot_api_census.py"))
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)


class Census(unittest.TestCase):
    def test_sf_entry_closure_excludes_other_heroes_and_resolves_hero_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "hero").mkdir()
            (root / "bot_nevermore.lua").write_text('local h=require(GetScriptDirectory().."/hero/"..string.gsub(GetBot():GetUnitName(),"npc_dota_hero_",""))\nfunction Think() GetBot():GetHealth(); MissingGlobal(); end')
            (root / "bot_lina.lua").write_text('OtherHeroOnly()')
            (root / "hero/nevermore.lua").write_text('-- FakeComment()\nlocal s="FakeString()"\nGetBot():MissingMethod()')
            report = census.scan(root, {"global": {"GetBot", "GetScriptDirectory"}, "unit": {"GetHealth", "GetUnitName"}, "string": {"gsub"}})
            self.assertEqual(report["files"], ["bot_nevermore.lua", "hero/nevermore.lua"])
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
            report = census.scan(root, {"global": {"GetBot"}, "unit": {"GetHealth"}})
            self.assertEqual(report["missing_members"], {})
            self.assertNotIn("Deferred", report["missing_globals"])
            self.assertTrue(report["unresolved_dependencies"])
            self.assertFalse(report["closure_complete"])

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
