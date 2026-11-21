"""Exercise the feature fork check with real Cargo graphs and no registry dependencies."""

import contextlib
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import feature_forks


class FeatureForkTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="bota-feature-forks-")
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)

    def package(self, name, manifest=""):
        (self.root / name / "src").mkdir(parents=True)
        (self.root / name / "src/lib.rs").touch()
        (self.root / name / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2024"\n{manifest}')

    def workspace(self, b_dev="", groups='{ crate = "a" }, { crate = "b" }', crate_tables="", a_features=""):
        (self.root / "Cargo.toml").write_text('[workspace]\nresolver = "3"\nmembers = ["a", "b", "bota_lua", "shared"]\n')
        self.package("shared", '\n[features]\nextra = []\nown = []\n')
        self.package("a", f'\n[dependencies]\nshared = {{ path = "../shared" }}\n\n[features]\nown = ["shared/own"]\n{a_features}')
        self.package("b", f'\n[dependencies]\nshared = {{ path = "../shared" }}\n\n[dev-dependencies]\nshared = {{ path = "../shared"{b_dev} }}\n')
        self.package("bota_lua", '\n[features]\nnative = []\n')
        (self.root / "tools").mkdir()
        (self.root / "tools/test_groups.toml").write_text(f'[crate.a]\n[crate.b]\n{crate_tables}\n[[group]]\nname = "g"\ntests = [{groups}]\n')
        subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=self.root, check=True, capture_output=True, text=True, timeout=30)

    def check(self):
        output = io.StringIO()
        with mock.patch.object(feature_forks, "ROOT", self.root):
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
                status = feature_forks.main()
        return status, output.getvalue()

    def test_a_dependency_with_other_features_in_one_test_build_fails_and_is_named(self):
        self.workspace(b_dev=', features = ["extra"]')
        status, output = self.check()
        self.assertEqual(status, 1, output)
        self.assertIn("shared v0.1.0\n", output)
        self.assertIn("feature forks: 1 packages", output)
        self.assertNotIn(str(self.root), output)

    def test_test_builds_that_agree_pass(self):
        self.workspace()
        status, output = self.check()
        self.assertEqual(status, 0, output)
        self.assertIn("feature forks: 0 packages", output)

    def test_a_crate_that_differs_only_in_its_own_feature_passes(self):
        self.workspace(groups='{ crate = "a", features = ["alone"] }, { crate = "b" }', a_features='alone = []\n')
        status, output = self.check()
        self.assertEqual(status, 0, output)
        self.assertIn("feature forks: 0 packages", output)

    def test_a_feature_of_another_package_that_a_crate_turns_on_is_a_fork(self):
        self.workspace(groups='{ crate = "a", features = ["own"] }, { crate = "b" }')
        status, output = self.check()
        self.assertEqual(status, 1, output)   # `shared/own` is not a's own feature: shared forks
        names = [line for line in output.splitlines() if line and not line.startswith(" ")]
        self.assertIn("shared v0.1.0", names)
        self.assertNotIn("a v0.1.0", names, "a itself, built with its own group's feature, is no fork")


if __name__ == "__main__":
    unittest.main()
