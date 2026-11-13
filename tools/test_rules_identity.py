"""Build real rule consumers across independent branches and editorial changes."""

import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class RulesIdentityBuildTests(unittest.TestCase):
    def test_independent_rules_merge_and_editorial_changes_keep_the_identity(self):
        scratch = ROOT / "temp"
        scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="rules_identity_", dir=scratch) as name:
            tree = Path(name)
            self.make_tree(tree)
            initial = self.identity(tree)
            self.command(tree, "cargo", "fmt", "--all")
            self.assertEqual(self.identity(tree), initial, "cargo fmt must preserve the rules identity")
            self.git(tree, "init", "-q", "-b", "base")
            self.git(tree, "add", ".")
            self.git(tree, "commit", "-qm", "Base rules")

            server = tree / "crates/bota_server/src/game/systems/illusion.rs"
            original = server.read_text()
            server.write_text(original + "\n// A comment, not a rule.\n")
            self.assertEqual(self.identity(tree), initial, "comments must not move RULES")
            server.write_text(original)

            plumbing = tree / "crates/bota_server/src/chat.rs"
            plumbing_source = plumbing.read_text()
            plumbing.write_text(plumbing_source + "\nfn identity_transport_probe() {}\n")
            self.assertEqual(self.identity(tree), initial, "service plumbing is outside simulation identity")
            plumbing.write_text(plumbing_source)
            wire_limits = tree / "crates/bota_proto/src/limits.rs"
            wire_source = wire_limits.read_text()
            wire_limits.write_text(wire_source.replace("MAX_CHAT_BYTES: usize = 512;", "MAX_CHAT_BYTES: usize = 513;"))
            self.assertEqual(self.identity(tree), initial, "wire-only bounds have their own wire fingerprint")
            wire_limits.write_text(wire_source)

            data = tree / "crates/bota_core/src/data/items.rs"
            original_data = data.read_text()
            price = re.compile(r"(const ITEM_BOTTLE:.*?cost: )(\d+)", re.S)
            changed, count = price.subn(lambda match: match[1] + str(int(match[2]) + 1), original_data, count=1)
            self.assertEqual(count, 1, "a Bottle price exists in imported data")
            data.write_text(changed)
            item_change = self.identity(tree)
            self.assertEqual(item_change[0], initial[0], "item data uses catalogue agreement")
            self.assertNotEqual(item_change[1], initial[1], "an imported rule must change its catalogue identity")
            data.write_text(original_data)

            identifiers = tree / "crates/bota_proto/src/ids.rs"
            original_identifiers = identifiers.read_text()
            identifiers.write_text(original_identifiers.replace("pub const BOOTS: ItemId = ItemId(0);", "pub const BOOTS: ItemId = ItemId(1);"))
            binding_change = self.identity(tree)
            self.assertEqual(binding_change[0], initial[0], "item bindings use catalogue agreement")
            self.assertNotEqual(binding_change[1], initial[1], "changing an existing item binding must move its catalogue identity")
            identifiers.write_text(original_identifiers.replace("impl ItemId {", "impl ItemId {\n pub const FUTURE_ITEM: ItemId = ItemId(65535);", 1))
            self.assertEqual(self.identity(tree), initial, "an appended id must preserve older catalogue prefixes")
            identifiers.write_text(original_identifiers)

            original_mechanics = (tree / "crates/bota_core/src/data/item_mechanics.rs").read_text()
            original_registry = (tree / "crates/bota_proto/registry/ids.txt").read_text()
            self.append_item(tree, int(initial[2]))
            appended = self.identity(tree, int(initial[2]))
            self.assertEqual(appended[:2], initial[:2], "a real item, including its private helper, must preserve RULES and the older catalogue")
            self.assertEqual(int(appended[2]), int(initial[2]) + 1, "the appended row is in the compiled catalogue")
            self.assertNotEqual(self.identity(tree)[1], initial[1], "the full catalogue includes the appended item")
            data.write_text(original_data)
            identifiers.write_text(original_identifiers)
            (tree / "crates/bota_core/src/data/item_mechanics.rs").write_text(original_mechanics)
            (tree / "crates/bota_proto/registry/ids.txt").write_text(original_registry)

            self.git(tree, "switch", "-qc", "formation")
            server.write_text(original.replace("ILLUSION_RUNE_COUNT as i32 + 2", "ILLUSION_RUNE_COUNT as i32 + 3"))
            formation = self.identity(tree)
            self.assertNotEqual(formation, initial, "a server rule must automatically move RULES")
            self.git(tree, "add", ".")
            self.git(tree, "commit", "-qm", "Change illusion formation")

            self.git(tree, "switch", "-qc", "duration", "base")
            map_source = tree / "crates/bota_core/src/map.rs"
            map_source.write_text(map_source.read_text().replace("20 * 60 * rules::TICKS_PER_SECOND", "21 * 60 * rules::TICKS_PER_SECOND"))
            duration = self.identity(tree)
            self.assertNotEqual(duration, initial, "a core rule must automatically move RULES")
            self.assertNotEqual(duration, formation, "independent rules must have different identities")
            self.git(tree, "add", ".")
            self.git(tree, "commit", "-qm", "Change match duration")
            self.git(tree, "merge", "-q", "--no-edit", "formation")
            merged = self.identity(tree)
            self.assertNotIn(merged, (initial, formation, duration), "the merge must derive its own identity")
            self.assertEqual(self.git(tree, "status", "--porcelain"), "", "no version or pin edits after the merge")

            with map_source.open("a") as output:
                output.write("\npub const BROKEN_SOURCE: u8 = ;\n")
            result = subprocess.run(
                ["cargo", "check", "--release", "--offline", "-p", "identity_probe"],
                cwd=tree, capture_output=True, text=True,
                env={**os.environ, "CARGO_TARGET_DIR": str(tree / "target")}, timeout=300,
            )
            self.assertNotEqual(result.returncode, 0, "rustc must reject invalid production source")
            self.assertIn("expected expression", result.stderr)
            self.assertIn("map.rs", result.stderr)
            self.assertNotIn("failed to run custom build command", result.stderr)

    def append_item(self, tree, item_id):
        data = tree / "crates/bota_core/src/data/items.rs"
        source = data.read_text()
        row = re.search(r"pub\(crate\) const ITEM_BOOTS: ItemData = ItemData \{.*?^\};", source, re.M | re.S)
        self.assertIsNotNone(row, "a simple imported row exists")
        appended = row[0].replace("ITEM_BOOTS", "ITEM_RULES_TEST").replace("ItemId(0)", f"ItemId({item_id})").replace('"item_boots"', '"item_rules_test"')
        data.write_text(source + "\n" + appended + "\n")
        mechanics = tree / "crates/bota_core/src/data/item_mechanics.rs"
        source = mechanics.read_text()
        declaration = re.search(r"pub\(crate\)\s+(?:const|static)\s+ITEM_ROWS\s*:", source)
        self.assertIsNotNone(declaration, "catalogue row storage exists")
        end = source.index("\n];", declaration.start())
        source = source[:end] + "\n    rules_test_row(&ITEM_RULES_TEST)," + source[end:]
        source += "\nconst fn rules_test_row(data: &ItemData) -> ItemDef { from(data) }\n"
        mechanics.write_text(source)
        identifiers = tree / "crates/bota_proto/src/ids.rs"
        identifiers.write_text(identifiers.read_text().replace("impl ItemId {", f"impl ItemId {{\n pub const RULES_TEST: ItemId = ItemId({item_id});", 1))
        registry = tree / "crates/bota_proto/registry/ids.txt"
        registry.write_text(registry.read_text().rstrip() + f"\nitem {item_id} live item_rules_test\n")

    def make_tree(self, tree):
        for crate in ("bota_core", "bota_proto"):
            shutil.copytree(ROOT / "crates" / crate, tree / "crates" / crate)
        shutil.copytree(ROOT / "crates/bota_server/src", tree / "crates/bota_server/src")
        # These probes change catalogue rows on purpose; an admitted earlier wire pins its catalogue at build time
        # (bota_core::ADMITTED_WIRES, "an admitted build's items are this build's first ones, unchanged") and would
        # refuse the build. Admission has tests of its own (bota_server --test admitted): the probes run on a tree
        # admitting none, which is a valid build.
        fingerprint = tree / "crates/bota_core/src/fingerprint.rs"
        source = fingerprint.read_text()
        start = source.index("pub const ADMITTED_WIRES: &[AdmittedWire] = &[")
        end = source.index("\n];", start)
        fingerprint.write_text(source[:start] + "pub const ADMITTED_WIRES: &[AdmittedWire] = &[" + source[end:])
        map_source = tree / "crates/bota_core/src/map.rs"
        with map_source.open("a") as output:
            output.write(
                '\n#[allow(dead_code)] fn identity_format_probe ( x:i32 )->(i32,){'
                'match x {0=>{(1,)},_ => (x,),}}\n'
            )
        manifest = (ROOT / "Cargo.toml").read_text()
        rest = manifest[manifest.index("[workspace.package]"):]
        (tree / "Cargo.toml").write_text(
            '[workspace]\nresolver = "3"\nmembers = ["crates/bota_core", "crates/bota_proto", "probe"]\n' + rest
        )
        shutil.copyfile(ROOT / "Cargo.lock", tree / "Cargo.lock")
        (tree / ".gitignore").write_text("/target/\n")
        (tree / "probe/src").mkdir(parents=True)
        (tree / "probe/Cargo.toml").write_text(
            '[package]\nname = "identity_probe"\nversion = "0.1.0"\nedition = "2024"\n'
            '[dependencies]\nbota_core = { path = "../crates/bota_core" }\n'
        )
        (tree / "probe/src/main.rs").write_text(
            'fn main() { let catalogue = std::env::args().nth(1).map(|n| bota_core::Catalogue::first(n.parse().unwrap()).unwrap()).unwrap_or(bota_core::Catalogue::FULL); '
            'println!("{:016x} {:016x} {}", bota_core::RULES_IDENTITY, catalogue.prefix().digest, bota_core::Catalogue::FULL.items()); }\n'
        )

    def identity(self, tree, prefix=None):
        arguments = ("--", str(prefix)) if prefix is not None else ()
        return tuple(self.command(tree, "cargo", "run", "--release", "--offline", "--quiet", "-p", "identity_probe", *arguments).split())

    def git(self, tree, *arguments):
        return self.command(tree, "git", "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "core.hooksPath=/dev/null", *arguments)

    def command(self, tree, *arguments):
        result = subprocess.run(
            arguments, cwd=tree, capture_output=True, text=True,
            env={**os.environ, "CARGO_TARGET_DIR": str(tree / "target")}, timeout=300,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout.strip()
