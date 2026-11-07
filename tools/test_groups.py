#!/usr/bin/env python3
"""The test groups of this repository (tools/test_groups.toml): which tests a change runs, and the proof that none is left out.

    tools/test_groups.py check [--base REV | --all]   every test in exactly one group, every file routed, at most 30 groups
    tools/test_groups.py plan [--base REV | --all]    the groups and crates the change since the merge-base with REV affects
    tools/test_groups.py available                    the crates of their own whose required checkouts are here (see [crate.*] requires)
    tools/test_groups.py run [--record FILE] [--cpus N] GROUP...
                                                      the groups' tests and commands side by side, their threads within
                                                      N CPUs (TEST_GROUPS_CPUS; default: the CPUs the run may use),
                                                      stopping at the first failure

REV defaults to origin/main (main where there is no origin). `check` lists the tests of the crates the change can touch
(every crate with --all) by building their test targets in each build the groups use and with all their features; a crate
outside the change keeps the tests and the groups it had at the merge-base, which passed, and so does a build beyond a
crate's plain one that no running group uses, no changed file names a feature of and no changed Rust file needs (see
check_tests, builds_to_list, beyond_plain and check_feature_files). `run` runs the test binaries
the listing built directly, side by side and the longest of the last run first (target/test-groups-history.json), and
checks that each ran exactly as many tests as its groups select, so this selection and libtest's own cannot drift apart.
`plan` prints five lines: `groups NAME...`, `crates NAME...` (the crates to lint: the workspace's, and bota_lua where its
luagee checkout is), `fuzz 0|1` (whether the fuzz targets are to be compiled: a crate they build on or the fuzz crate itself
changed), `uncovered NAME...` (the crates that cannot be built here, so nothing covers them; a change to one of their files
is refused) and `scope` (what a stamp of the check names besides the tree).
"""

import argparse
import hashlib
import json
import math
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip())
CONFIG = ROOT / "tools/test_groups.toml"
FUZZ_MANIFEST = ROOT / "fuzz/Cargo.toml"
PROFILE = "test-release"
# Set (to anything) to make a crate whose `requires` files are absent a failure instead of a noted omission: for the machines
# that merge: they must cover everything.
REQUIRE_ALL = "TEST_GROUPS_REQUIRE_ALL"
# A unittest suite is a build of its own: its directory stands where a crate's name does.
UNITTEST = "unittest"
# At most this many groups: few enough to know each by name, enough to follow the code's crates and subsystems.
MAX_GROUPS = 30
# These write reference data by hand; they are not regression contracts and never run in a gate.
MANUAL_RECORDERS = {
    ("bota_server", "lib"): frozenset({
        "game::tests::determinism::print_pins",
        "game::tests::determinism::print_blind_pins",
    }),
}


# Local sources are explicitly optional and never fetched by the public gate. Only
# these exact contracts may be omitted; ordinary ignored tests still fail the run.
LOCAL_CONFORMANCE = {
    ("bota_lua", "test:local_scripts", "local_scripts_play_a_complete_match_without_script_failures"):
        ("BOTA_LUA_LOCAL_PACK", "tools/test_lua_local.sh"),
}


def local_conformance(crate: str, target: str, selected: list[str]) -> tuple[list[str], list[str]]:
    omitted, arguments = [], []
    for name in selected:
        requirement = LOCAL_CONFORMANCE.get((crate, target, name))
        if requirement is None:
            continue
        variable, command = requirement
        if os.environ.get(variable):
            arguments = ["--include-ignored"]
        else:
            omitted.append(name)
            print(f"test-groups: optional local conformance not run: {crate} {name}; "
                  f"set {variable} and run {command} (public contracts only)")
    return omitted, arguments


class Failure(Exception):
    """A problem to report to the user, one per line."""


@dataclass(frozen=True)
class Build:
    """How a crate's tests are built: the cargo flags that change the artifacts."""

    crate: str
    profile: str
    features: tuple[str, ...]


@dataclass(frozen=True)
class Selector:
    build: Build
    target: str | None  # None: every test target of the crate
    filters: tuple[str, ...]
    args: tuple[str, ...]
    env: tuple[tuple[str, str], ...]
    before: tuple[tuple[str, ...], ...]


@dataclass
class Group:
    name: str
    paths: list[str]
    also_paths: list[str]
    selectors: list[Selector]
    commands: list[list[str]]


@dataclass
class Crate:
    name: str
    directory: str  # relative to ROOT
    manifest: str | None  # outside the workspace: its own manifest
    requires: list[str]  # files (relative to ROOT) it cannot build without: other repositories checked out beside this one
    lint: bool  # outside the workspace: the gate lints it too (clippy and the wasm check with its own manifest)
    routed: bool
    features: list[str]
    profile: str
    all_features: tuple[str, ...]  # every feature but the wasm-only ones
    defaults: list[str]
    feature_table: dict[str, list[str]]
    dependents: set[str] = field(default_factory=set)  # direct, by Cargo (development dependencies too)
    launchers: set[str] = field(default_factory=set)  # direct, declared: crates whose tests run this one's binary
    harnessless: set[str] = field(default_factory=set)  # test targets with `harness = false`
    benches: set[str] = field(default_factory=set)
    available: bool = True  # False: its `requires` were absent when it was loaded, so nothing of it can be built here


@dataclass
class Listed:
    """The tests a build of a crate holds: per target, its names and the file the target starts at."""

    tests: dict[str, list[str]]
    roots: dict[str, Path]
    executables: dict[str, Path] = field(default_factory=dict)
    sources: set[str] = field(default_factory=set)  # the crate's files its test binaries are built from (rustc's dep-info)


def present(requires: list[str]) -> bool:
    return all((ROOT / path).exists() for path in requires)


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout


def glob_regex(pattern: str) -> re.Pattern[str]:
    """`**` crosses directories, `*` and `?` do not."""
    out = ""
    for part in re.split(r"(\*\*|\*|\?)", pattern):
        out += {"**": ".*", "*": "[^/]*", "?": "[^/]"}.get(part, re.escape(part))
    return re.compile(out + r"\Z")


def metadata(manifest: str | None) -> dict:
    command = ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"]
    if manifest:
        command += ["--manifest-path", str(ROOT / manifest)]
    return json.loads(subprocess.run(command, cwd=ROOT, capture_output=True, text=True, check=True).stdout)


def feature_closure(table: dict[str, list[str]], wanted: list[str]) -> tuple[str, ...]:
    """The crate's own features `wanted` turns on (dependency features and `dep:` entries left out)."""
    seen: set[str] = set()
    todo = list(wanted)
    while todo:
        feature = todo.pop()
        if feature in table and feature not in seen:
            seen.add(feature)
            todo += table[feature]
    seen.discard("default")
    return tuple(sorted(seen))


def load_crates(config: dict) -> dict[str, Crate]:
    crates: dict[str, Crate] = {}
    outside: dict[str, dict] = {}  # the packages of the crates with a manifest of their own, which also depend on workspace crates
    workspace = metadata(None)
    for name, settings in config["crate"].items():
        manifest = settings.get("manifest")
        requires = settings.get("requires", [])
        if requires and not manifest:
            raise Failure(f"test_groups.toml: crate {name} requires files, so it must be outside the workspace (name its manifest)")
        if not present(requires):
            if os.environ.get(REQUIRE_ALL):
                missing = [path for path in requires if not present([path])]
                raise Failure(f"crate {name} needs {' '.join(missing)}, which is absent, and {REQUIRE_ALL} is set")
            crates[name] = absent_crate(name, settings, manifest)
            continue
        packages = metadata(manifest)["packages"] if manifest else workspace["packages"]
        package = next((p for p in packages if p["name"] == name), None)
        if package is None:
            raise Failure(f"test_groups.toml: crate {name} is no package of the repository")
        table = package["features"]
        wasm_only = set(settings.get("wasm_only_features", []))
        manifest_toml = tomllib.loads(Path(package["manifest_path"]).read_text())
        if manifest:
            outside[name] = package
        crates[name] = Crate(
            name=name,
            directory=str(Path(package["manifest_path"]).parent.relative_to(ROOT)),
            manifest=manifest,
            requires=requires,
            lint=settings.get("lint", manifest is None),
            routed=settings.get("routed", False),
            features=settings.get("features", []),
            profile=settings.get("profile", PROFILE),
            all_features=feature_closure(table, [f for f in table if f not in wasm_only] + ["default"]),
            defaults=table.get("default", []),
            feature_table=table,
            launchers=set(settings.get("dependents", [])),
            harnessless={t["name"] for t in manifest_toml.get("test", []) if t.get("harness") is False},
            benches={t["name"] for t in package["targets"] if "bench" in t["kind"]},
        )
    members = {p["name"]: p for p in workspace["packages"]}
    for name, package in members.items():
        if name not in crates:
            raise Failure(f"test_groups.toml: workspace crate {name} has no [crate.{name}] table")
    for name, package in {**members, **outside}.items():
        for dependency in package["dependencies"]:
            if dependency.get("path") and dependency["name"] in crates:
                crates[dependency["name"]].dependents.add(name)
    return crates


def absent_crate(name: str, settings: dict, manifest: str) -> Crate:
    """A crate whose `requires` are missing: it is routed (its files belong to it) but nothing of it can be built, listed or
    run, so it has no features, no groups (load drops them) and nothing depends on it."""
    return Crate(name, str(Path(manifest).parent), manifest, settings["requires"], settings.get("lint", False), settings.get("routed", False),
                 [], settings.get("profile", PROFILE), (), [], {}, available=False)


def load() -> tuple[dict[str, Crate], list[Group], list[tuple[list[str], list[str]]]]:
    config = tomllib.loads(CONFIG.read_text())
    crates = load_crates(config)
    groups = [load_group(entry, crates) for entry in config.get("group", [])]
    groups = [g for g in groups if not left_out(g, crates)]
    routes = [(route["paths"], route["groups"]) for route in config.get("route", [])]
    return crates, groups, routes


def available_optional(config: dict) -> list[str]:
    """The crates with `requires` whose files are all there (read from the config alone: no cargo, so cheap for scripts)."""
    return [name for name, settings in config["crate"].items()
            if settings.get("requires") and present(settings["requires"])]


def left_out(group: Group, crates: dict[str, Crate]) -> bool:
    """A group of tests of crates that cannot be built here (their `requires` are absent) is left out, said once on stderr."""
    absent = {s.build.crate for s in group.selectors if s.build.profile != UNITTEST and not crates[s.build.crate].available}
    if not absent:
        return False
    print(f"test-groups: group {group.name} is not covered here: {', '.join(sorted(absent))} needs "
          f"{' '.join(p for c in sorted(absent) for p in crates[c].requires)}, which is absent", file=sys.stderr)
    return True


def load_group(entry: dict, crates: dict[str, Crate]) -> Group:
    selectors = []
    for test in entry.get("tests", []):
        if "unittest" in test:  # Python tests: every test_*.py under the directory
            build = Build(test["unittest"], UNITTEST, ())
            selectors.append(Selector(build, UNITTEST, (), (), (), ()))
            continue
        if test.get("filters") and not test.get("target"):
            raise Failure(f"test_groups.toml: group {entry['name']} filters {test['crate']} without a target: name the target the filters are for")
        crate = crates.get(test["crate"])
        if crate is None:
            raise Failure(f"test_groups.toml: group {entry['name']} names crate {test['crate']}, which has no [crate] table")
        wanted = test.get("features", []) + crate.features + crate.defaults
        build = Build(crate.name, test.get("profile", crate.profile), feature_closure(crate.feature_table, wanted))
        selectors.append(
            Selector(
                build=build,
                target=test.get("target"),
                filters=tuple(test.get("filters", [])),
                args=tuple(test.get("args", [])),
                env=tuple(sorted(test.get("env", {}).items())),
                before=tuple(tuple(command) for command in test.get("before", [])),
            )
        )
    return Group(entry["name"], entry.get("paths", []), entry.get("also_paths", []), selectors, entry.get("commands", []))


class Router:
    """Which groups a change to a file runs."""

    def __init__(self, crates: dict[str, Crate], groups: list[Group], routes: list[tuple[list[str], list[str]]]):
        self.crates = crates
        self.groups = groups
        self.names = [group.name for group in groups]
        self.route_paths = routes
        self.routes = [([glob_regex(p) for p in paths], targets) for paths, targets in routes]
        self.owned = [([glob_regex(p) for p in group.paths], group.name) for group in groups]
        self.also = [
            ([glob_regex(p) for p in group.also_paths if not p.startswith("!")],
             [glob_regex(p[1:]) for p in group.also_paths if p.startswith("!")], group.name)
            for group in groups
        ]

    def above(self, crate: str, launched: bool = True) -> set[str]:
        """The crate and every crate depending on it: by Cargo, and with `launched` by a declared `dependents` too."""
        seen, todo = {crate}, [crate]
        while todo:
            current = self.crates[todo.pop()]
            for dependent in current.dependents | (current.launchers if launched else set()):
                if dependent not in seen:
                    seen.add(dependent)
                    todo.append(dependent)
        return seen

    def crate_wide(self, crate: str) -> set[str]:
        crates = self.above(crate)
        return {g.name for g in self.groups if any(s.build.crate in crates for s in g.selectors)}

    def expand(self, token: str) -> set[str]:
        if token == "*":
            return set(self.names)
        if token.startswith("@"):
            return self.crate_wide(token[1:])
        if token.endswith("*"):
            return {name for name in self.names if name.startswith(token[:-1])}
        return {token} if token in self.names else set()

    def workspace(self) -> set[str]:
        return {name for name, crate in self.crates.items() if crate.manifest is None}

    def linted(self) -> set[str]:
        """The crates a plan lints: the workspace and the crates of their own that are linted too and can be built here."""
        return self.workspace() | {name for name, crate in self.crates.items() if crate.lint and crate.available}

    def uncovered(self) -> list[str]:
        """The crates that cannot be built here because a checkout they require is absent: no group, lint or step covers them."""
        return sorted(name for name, crate in self.crates.items() if not crate.available)

    def crate_of(self, path: str) -> Crate | None:
        inside = [c for c in self.crates.values() if path.startswith(c.directory + "/")]
        return max(inside, key=lambda c: len(c.directory), default=None)

    def route(self, path: str) -> tuple[set[str], bool] | None:
        """The groups a change to `path` runs and whether it runs everything; None when nothing names the file."""
        groups: set[str] = set()
        named = False
        everything = False
        for patterns, name in self.owned:
            if any(p.match(path) for p in patterns):
                groups.add(name)
                named = True
        for patterns, targets in self.routes:
            if any(p.match(path) for p in patterns):
                named = True
                everything |= "*" in targets
                for token in targets:
                    groups |= self.expand(token)
        if named and not groups and not everything:
            return set(), False  # a route that runs nothing (documents), in a crate too
        crate = self.crate_of(path)
        if crate is not None and not crate.routed:
            groups |= self.crate_wide(crate.name)
            named = True
        if not named:
            return None
        groups |= {name for take, leave, name in self.also if any(p.match(path) for p in take) and not any(p.match(path) for p in leave)}
        return groups, everything


def files() -> list[str]:
    return sorted(set(git("ls-files").splitlines()) | set(git("ls-files", "--others", "--exclude-standard").splitlines()))


def changed(base: str) -> tuple[str, list[str]]:
    """The merge-base with `base` and the paths changed since, committed or not; a moved file counts at both its paths."""
    found = subprocess.run(["git", "merge-base", "HEAD", base], cwd=ROOT, capture_output=True, text=True)
    if found.returncode != 0:
        raise Failure(f"no merge-base of HEAD and {base}: {found.stderr.strip()}")
    merge_base = found.stdout.strip()
    diff = git("diff", "--name-only", "--no-renames", merge_base).splitlines()
    return merge_base, sorted(set(diff) | set(git("ls-files", "--others", "--exclude-standard").splitlines()))


def default_base() -> str:
    found = subprocess.run(["git", "rev-parse", "-q", "--verify", "origin/main"], cwd=ROOT, capture_output=True)
    return "origin/main" if found.returncode == 0 else "main"


def fuzz_dependencies() -> set[str]:
    """The crates the fuzz targets build on directly (the fuzz crate is outside the workspace, so Cargo's graph of it is not the router's)."""
    manifest = tomllib.loads(FUZZ_MANIFEST.read_text())
    return {name for name, spec in manifest["dependencies"].items() if isinstance(spec, dict) and "path" in spec}


def plan(router: Router, paths: list[str] | None) -> tuple[set[str], set[str], bool]:
    """Groups to run, workspace crates to lint and whether the fuzz targets are to be checked; `paths` None: everything."""
    workspace = router.linted()
    if paths is None:
        return set(router.names), workspace, True
    groups: set[str] = set()
    touched: set[str] = set()
    for path in paths:
        routed = router.route(path)
        if routed is None or routed[1]:  # unknown or the build itself: run everything
            return set(router.names), workspace, True
        groups |= routed[0]
        crate = router.crate_of(path)
        if routed[0] and crate is not None and crate.name in workspace:
            touched.add(crate.name)
    crates: set[str] = set()
    for crate in touched:
        crates |= router.above(crate, launched=False) & workspace
    # `crates` holds every crate above a changed one, so it meets the fuzz targets' crates exactly when one of them is affected
    return groups, crates, bool(crates & fuzz_dependencies()) or any(p.startswith("fuzz/") for p in paths)


def plan_report(router: Router, merge_base: str | None, paths: list[str] | None) -> list[str]:
    """`plan`'s lines: the groups, the crates to lint, whether to check the fuzz targets, and the scope of the stamps."""
    absent = router.uncovered()
    if paths is not None:
        check_absent_untouched(router, paths)
    chosen, lint, fuzz = plan(router, paths)
    ordered = [n for n in router.names if n in chosen]
    everything = (set(router.names), router.linted(), True) == (chosen, lint, fuzz)
    scope = scope_of(None if everything else merge_base, ordered, sorted(lint), fuzz, absent)
    return [" ".join(["groups", *ordered]), " ".join(["crates", *sorted(lint)]), f"fuzz {int(fuzz)}",
            " ".join(["uncovered", *absent]), f"scope {scope}"]


def check_absent_untouched(router: Router, paths: list[str]) -> None:
    """A change to a file of a crate that cannot be built here cannot be checked here: refuse it rather than let it look
    green. Documents are no code to check."""
    for path in paths:
        crate = router.crate_of(path)
        if crate is not None and not crate.available and not path.endswith(".md"):
            raise Failure(f"{path} changed, but {crate.name} cannot be built here: {' '.join(crate.requires)} is missing. Check "
                          f"out that repository beside this one so that the change can be checked")


def scope_of(merge_base: str | None, groups: list[str], crates: list[str], fuzz: bool, uncovered: list[str] = ()) -> str:
    """What a stamp of a check covers besides the tree: `full` (everything, also when a change selects it all), or what was
    compared with and what that selected; with crates that cannot be built here (`uncovered`) never `full`, so a stamp of a
    tree that was not fully checked is not taken for one that was."""
    if merge_base is None:
        return "full" if not uncovered else "full-without-" + "-".join(uncovered)
    selection = json.dumps([merge_base, groups, crates, fuzz, list(uncovered)])
    return "affected-" + hashlib.sha256(selection.encode()).hexdigest()[:16]


def cargo_test(crates: dict[str, Crate], build: Build) -> list[str]:
    command = ["cargo", "test"]
    manifest = crates[build.crate].manifest
    if manifest:
        command += ["--manifest-path", manifest]
    command += ["-p", build.crate, "--profile", build.profile]
    if build.features:
        command += ["--features", ",".join(build.features)]
    return command


def target_name(target: dict) -> str:
    kind = target["kind"][0]
    if kind in ("lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"):
        return "lib"
    return f"{kind}:{target['name']}"


def list_build(crates: dict[str, Crate], build: Build) -> Listed:
    """Builds the test targets of `build` and asks each for its tests (libtest's `--list`), the doc tests too."""
    if build.profile == UNITTEST:
        return list_unittest(build.crate)
    crate = crates[build.crate]
    command = cargo_test(crates, build) + ["--no-run", "--message-format=json-render-diagnostics"]
    built = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, text=True)
    if built.returncode != 0:
        raise Failure(f"building the tests of {build} failed: {' '.join(command)}")
    listed = Listed({}, {})
    has_doc = False
    for line in built.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact":
            continue
        target = message["target"]
        if not Path(target["src_path"]).is_relative_to(ROOT / crate.directory):
            continue  # a crate it builds on
        has_doc |= target_name(target) == "lib" and target.get("doctest", False)
        if not message["profile"]["test"] or not message.get("executable"):
            continue
        name = target_name(target)
        if name.startswith("test:") and target["name"] in crate.harnessless:
            raise Failure(f"{build.crate}: test target {target['name']} has `harness = false`; it cannot be listed, so no group can hold it")
        listing = subprocess.run([message["executable"], "--list", "--format", "terse"], cwd=ROOT / crate.directory, capture_output=True, text=True, check=True)
        listed.tests[name] = tests_of(listing.stdout)
        listed.roots[name] = Path(target["src_path"])
        listed.executables[name] = Path(message["executable"])
        listed.sources |= dep_info(Path(message["executable"]).with_suffix(".d"), crate)
    if has_doc:
        doc = subprocess.run(cargo_test(crates, build) + ["--doc", "--", "--list", "--format", "terse"], cwd=ROOT, capture_output=True, text=True)
        if doc.returncode != 0:
            raise Failure(f"listing the doc tests of {build} failed:\n{doc.stderr[-2000:]}")
        listed.tests["doc"] = tests_of(doc.stdout)
    return listed


def dep_info(path: Path, crate: Crate) -> set[str]:
    """The files of `crate` a binary was built from, as rustc's dep-info (the `.d` beside it) names them; the proof that
    rests on them fails rather than pass on nothing when the `.d` cannot be read."""
    try:
        first = path.read_text().splitlines()[0]
    except (OSError, IndexError) as error:
        raise Failure(f"{path}: rustc's dep-info cannot be read ({error!r}), so which files the build holds is unknown") from error
    if ": " not in first:
        raise Failure(f"{path}: not rustc's dep-info (no `target: sources` line)")
    names = first.split(": ", 1)[1].replace("\\ ", "\0").split()
    found = {(ROOT / name.replace("\0", " ")).resolve() for name in names}
    return {str(p.relative_to(ROOT)) for p in found if p.is_relative_to(ROOT / crate.directory)}


def unittest_command(directory: str) -> list[str]:
    return [sys.executable, "-B", "-m", "unittest", "discover", "-s", directory, "-t", directory, "-p", "test_*.py"]


def list_unittest(directory: str) -> Listed:
    """The ids of the unittest tests under `directory`, as `python3 -m unittest discover` finds them."""
    walk = (
        "import sys, unittest\n"
        "def walk(suite):\n"
        "    for test in suite:\n"
        "        yield from walk(test) if isinstance(test, unittest.TestSuite) else [test]\n"
        "for test in walk(unittest.defaultTestLoader.discover(sys.argv[1], 'test_*.py', sys.argv[1])):\n"
        "    print(test.id())\n"
    )
    found = subprocess.run([sys.executable, "-B", "-c", walk, directory], cwd=ROOT, capture_output=True, text=True)
    names = found.stdout.split()
    broken = [n for n in names if n.startswith("unittest.loader.")]
    if found.returncode != 0 or broken:
        raise Failure(f"discovering the Python tests under {directory} failed: {broken or found.stderr[-2000:]}")
    return Listed({UNITTEST: names}, {UNITTEST: ROOT / directory})


def tests_of(listing: str) -> list[str]:
    return [line[: -len(": test")] for line in listing.splitlines() if line.endswith(": test")]


def selects(selector: Selector, target: str, name: str) -> bool:
    """libtest's own rule: a test runs when its name contains any of the filters."""
    if selector.target is not None and selector.target != target:
        return False
    return not selector.filters or any(f in name for f in selector.filters)


def test_file(listed: Listed, target: str, name: str) -> Path:
    """The file a test is written in: its module path walked down from the target's root, as rustc finds module files."""
    if target == "doc":
        return ROOT / name.split(" - ", 1)[0]
    if target == UNITTEST:
        return listed.roots[target] / (name.split(".", 1)[0] + ".py")
    file = listed.roots[target]
    directory = file.parent
    for segment in name.split("::")[:-1]:
        found = next((c for c in (directory / f"{segment}.rs", directory / segment / "mod.rs") if c.is_file()), None)
        if found is None:
            break
        file, directory = found, directory / segment
    return file


def check(crates: dict[str, Crate], groups: list[Group], router: Router, paths: list[str] | None) -> list[str]:
    """The checks of the module header; `paths` None: everything, else the change's paths."""
    problems = check_names(groups, router)
    every_file = files()
    problems += check_patterns(groups, router, every_file)
    problems += [f"{path}: no group and no route names it (tools/test_groups.toml)" for path in every_file if router.route(path) is None]
    problems += check_harnesses(crates, groups, every_file)
    problems += check_markers(router, every_file)
    problems += check_negated_features(crates, groups, every_file)
    scope = set(crates) if paths is None else set()
    for path in paths or []:
        routed = router.route(path)
        crate = router.crate_of(path)
        if routed is None or routed[1]:
            scope = set(crates)
            break
        if crate is not None:
            scope |= router.above(crate.name)
    scope = {name for name in scope if crates[name].available}
    return problems + check_tests(crates, groups, router, scope, paths)


# What a test is written with, in Rust and in Python: a file holding one must run some group.
TEST_MARKERS = {".rs": ("#[test]", "#[cfg(test)]"), ".py": ("def test_", "unittest.TestCase")}


def check_markers(router: Router, every_file: list[str]) -> list[str]:
    """A file a route sends to no group (fuzz/, scripts/) may hold no test: no group would run it. The files of a crate that
    cannot be built here (router.uncovered) are left out: their groups are, and every plan names them."""
    problems = []
    absent = router.uncovered()
    for path in every_file:
        markers = TEST_MARKERS.get(Path(path).suffix)
        routed = router.route(path)
        if not markers or routed is None or routed[0] or not (ROOT / path).is_file():
            continue
        if (crate := router.crate_of(path)) is not None and crate.name in absent:
            continue
        found = [m for m in markers if m in (ROOT / path).read_text(errors="replace")]
        if found:
            problems.append(f"{path}: holds tests ({', '.join(found)}) but a change to it runs no group, so none runs them")
    return problems


NEGATED_FEATURE = re.compile(r'#\[cfg\(.*not\(\s*feature\s*=\s*"([^"]+)"\s*\)')


def check_negated_features(crates: dict[str, Crate], groups: list[Group], every_file: list[str]) -> list[str]:
    """A test gated on a feature being off, where every build the groups use turns it on, is built by none of them: such a
    gate (on the test itself, or on a test module) is refused."""
    problems = []
    for crate in crates.values():
        builds = [s.build for g in groups for s in g.selectors if s.build.crate == crate.name]
        always = set.intersection(*(set(b.features) for b in builds)) if builds else set()
        for path in (f for f in every_file if f.startswith(crate.directory + "/") and f.endswith(".rs")):
            lines = (ROOT / path).read_text(errors="replace").splitlines() if (ROOT / path).is_file() else []
            for number, line in enumerate(lines):
                found = NEGATED_FEATURE.search(line)
                if not found or found.group(1) not in always:
                    continue
                gated = "test" in line.split("not(", 1)[0] or any("#[test]" in after for after in lines[number + 1 : number + 4])
                if gated:
                    problems.append(f"{path}:{number + 1}: a test gated on feature {found.group(1)} being off, which every "
                                    f"build of {crate.name} the groups use turns on: none runs it")
    return problems


def check_harnesses(crates: dict[str, Crate], groups: list[Group], every_file: list[str]) -> list[str]:
    """libtest lists what the groups hold; a target with a harness of its own cannot be listed. A bench (always its own
    harness) must be run by a group's command, any other such target is refused."""
    problems = []
    for path in (f for f in every_file if f.endswith("Cargo.toml") and (ROOT / f).is_file()):
        manifest = tomllib.loads((ROOT / path).read_text())
        for kind in ("lib", "bin", "test", "example"):
            entries = manifest.get(kind, [])
            for target in [entries] if isinstance(entries, dict) else entries:
                if target.get("harness") is False and target.get("test", kind == "test") is not False:
                    problems.append(f"{path}: {kind} {target.get('name', '')} has a harness of its own, which no group can list")
    benched = set()
    for group in groups:
        for command in group.commands:
            if "--bench" in command and "-p" in command:
                benched.add((command[command.index("-p") + 1], command[command.index("--bench") + 1]))
    for crate in (c for c in crates.values() if c.manifest is None):
        for bench in crate.benches - {b for c, b in benched if c == crate.name}:
            problems.append(f"{crate.name}: bench {bench} is run by no group's command (tools/test_groups.toml)")
    return problems


def check_names(groups: list[Group], router: Router) -> list[str]:
    problems = []
    if len(groups) > MAX_GROUPS:
        problems.append(f"{len(groups)} groups, more than {MAX_GROUPS}")
    names = [group.name for group in groups]
    problems += [f"group {name} is defined twice" for name in sorted({n for n in names if names.count(n) > 1})]
    problems += [f"group {group.name} selects no tests" for group in groups if not group.selectors]
    for _, targets in router.routes:
        problems += [f"a route names {token}, which is no group" for token in targets if not router.expand(token)]
    return problems


def check_patterns(groups: list[Group], router: Router, every_file: list[str]) -> list[str]:
    owners = [(group.paths + [p.removeprefix("!") for p in group.also_paths], f"group {group.name}") for group in groups]
    owners += [(paths, "a route") for paths, _ in router.route_paths]
    problems = []
    for paths, owner in owners:
        for pattern in paths:
            regex = glob_regex(pattern)
            if not any(regex.match(path) for path in every_file):
                problems.append(f"{owner}: the pattern {pattern} names no file")
    return problems


def plain_build(crate: Crate) -> Build:
    """The build a crate's groups use unless they ask for more: its profile, its group features and its defaults."""
    return Build(crate.name, crate.profile, feature_closure(crate.feature_table, crate.features + crate.defaults))


def check_tests(crates: dict[str, Crate], groups: list[Group], router: Router, scope: set[str], paths: list[str] | None) -> list[str]:
    """Every test of the crates in `scope`, in every build a group uses and with all features, is in exactly one group, and a
    change to the file it is written in runs that group.

    A change (`paths`) lists a crate's other builds (more features, another profile) only when they can hold new tests: a
    group that uses one runs, a changed file names one of its extra features, or a changed Rust file is not in the plain
    build (builds_to_list, beyond_plain). That is enough because of what check_feature_files proves on every listed build:
    every file only such a build compiles routes to a group that uses it, and a test only that build holds is either in
    such a file or gated in its own file, which then names the feature."""
    candidates = builds_to_list(crates, groups, router, scope, None)
    ordered = sorted(builds_to_list(crates, groups, router, scope, paths), key=str)
    listings = {build: list_build(crates, build) for build in ordered}
    if paths is not None:
        outside = beyond_plain(crates, listings, paths)
        listings |= {b: list_build(crates, b) for b in sorted(candidates - set(listings), key=str) if b.crate in outside}
    return assess(groups, router, listings) + check_feature_files(crates, groups, router, listings)


def beyond_plain(crates: dict[str, Crate], listings: dict[Build, Listed], paths: list[str]) -> set[str]:
    """The crates with a changed Rust file their plain build does not compile: only another build can say what it holds."""
    found = set()
    for build, listed in listings.items():
        crate = crates.get(build.crate)
        if crate is None or build != plain_build(crate):
            continue
        inside = (p for p in paths if p.startswith(crate.directory + "/") and p.endswith(".rs") and (ROOT / p).is_file())
        if any(path not in listed.sources for path in inside):
            found.add(crate.name)
    return found


def builds_to_list(crates: dict[str, Crate], groups: list[Group], router: Router, scope: set[str], paths: list[str] | None) -> set[Build]:
    builds = {s.build for g in groups for s in g.selectors if s.build.crate in scope or s.build.profile == UNITTEST}
    builds |= {Build(c, crates[c].profile, crates[c].all_features) for c in scope}
    if paths is None:
        return builds
    chosen = plan(router, paths)[0]
    used = {s.build for g in groups if g.name in chosen for s in g.selectors}
    return {b for b in builds if b.profile == UNITTEST or b == plain_build(crates[b.crate]) or b in used
            or names_a_feature(crates[b.crate], b, paths)}


def names_a_feature(crate: Crate, build: Build, paths: list[str]) -> bool:
    """Whether a changed file of the crate names a feature `build` has beyond the crate's plain build."""
    extra = set(build.features) - set(plain_build(crate).features)
    for path in paths:
        if not extra or not path.startswith(crate.directory + "/") or not (ROOT / path).is_file():
            continue
        text = (ROOT / path).read_text(errors="replace")
        if any(f'"{feature}"' in text for feature in extra):
            return True
    return False


def check_feature_files(crates: dict[str, Crate], groups: list[Group], router: Router, listings: dict[Build, Listed]) -> list[str]:
    """What lets a change skip a crate's other builds: every file only such a build compiles routes to a group using it."""
    problems = []
    for build, listed in listings.items():
        if build.profile == UNITTEST or build.crate not in crates:
            continue
        plain = plain_build(crates[build.crate])
        if plain not in listings:
            raise Failure(f"{build.crate}: its plain build ({','.join(plain.features)}, {plain.profile}) was not listed, so "
                          f"what only {','.join(build.features)} ({build.profile}) compiles is unknown")
        only = listed.sources - listings[plain].sources
        users = {g.name for g in groups if any(s.build == build for s in g.selectors)}
        for path in sorted(only):
            routed = router.route(path)
            if routed is None or users.isdisjoint(routed[0]):
                problems.append(f"{path}: only the {build.crate} build with {','.join(build.features)} ({build.profile}) compiles it, "
                                f"but a change to it runs no group of that build ({', '.join(sorted(users)) or 'none uses it'})")
    return problems


def assess(groups: list[Group], router: Router, listings: dict[Build, Listed]) -> list[str]:
    """check_tests on the tests each build holds: each in exactly one group, its file routed to that group, every filter used."""
    owners: dict[tuple[str, str, str], set[str]] = {}
    files_of: dict[tuple[str, str, str], Path] = {}
    problems = []
    for build, listed in listings.items():
        for target, names in listed.tests.items():
            for name in names:
                key = (build.crate, target, name)
                owners.setdefault(key, set())
                files_of[key] = test_file(listed, target, name)
        for group in groups:
            for selector in (s for s in group.selectors if s.build == build):
                hits = [(t, n) for t, names in listed.tests.items() for n in names if selects(selector, t, n)]
                problems += unused_filters(group, selector, hits)
                for target, name in hits:
                    owners[(build.crate, target, name)].add(group.name)
    for key, names in sorted(owners.items()):
        crate, target, name = key
        if len(names) != 1:
            problems.append(f"{crate} {target} {name}: in {len(names)} groups {sorted(names)}, must be in exactly one")
            continue
        path = str(files_of[key].relative_to(ROOT))
        routed = router.route(path)
        if routed is None or names.isdisjoint(routed[0]):
            problems.append(f"{path}: holds {name} of group {next(iter(names))}, but a change to it does not run that group")
    return problems


def unused_filters(group: Group, selector: Selector, hits: list[tuple[str, str]]) -> list[str]:
    if not hits:
        return [f"group {group.name}: {selector.build.crate} {selector.target or 'all targets'} selects no test"]
    return [f"group {group.name}: the filter {f} selects no test" for f in selector.filters if not any(f in n for _, n in hits)]


# A run keeps the CPUs its binaries use within its CPU share (its CPUs, under the cgroup's quota and heavy.sh's
# HEAVY_CPUS; TEST_GROUPS_CPUS or --cpus sets it). A binary gets as many test threads as it has tests, up to an inherited
# RUST_TEST_THREADS (heavy.sh caps it: a test thread may play a game, and games take memory), and starts when the share
# has room for the CPUs it used last time (its CPU time over its wall time, in the run's history; its threads before it
# has one). So the one large binary (bota_server's unit tests) takes most of the share, and the many small ones that
# mostly wait on sockets and clocks run beside it. The share is what the run may use, not what the machine has free: other
# jobs beside it are not counted (under heavy.sh without HEAVY_CPUS there is no quota), and a binary may use more than it
# used last time, up to its threads, until the next run's history says so.
# A hang guard: a test binary or command still running after this long is stopped and fails its groups.
TIMEOUT_SECONDS = 1800


def cpu_share() -> int:
    """The CPUs a run may use: its affinity, under the cgroup's cpu.max quota and HEAVY_CPUS."""
    share = len(os.sched_getaffinity(0))
    try:
        cgroup = Path("/proc/self/cgroup").read_text().splitlines()[0].split(":", 2)[2]
        quota, period = (Path("/sys/fs/cgroup") / cgroup.lstrip("/") / "cpu.max").read_text().split()
        if quota != "max":
            share = min(share, max(1, -(-int(quota) // int(period))))
    except (OSError, ValueError, IndexError):
        pass  # no cgroup v2 here: the affinity stands
    heavy = os.environ.get("HEAVY_CPUS", "")
    if heavy.isdigit() and int(heavy) > 0:
        share = min(share, int(heavy))
    return share


@dataclass
class Invocation:
    """One process of a run: a prebuilt test binary with its filters, or a cargo or unittest command."""

    key: str  # what the run's timing history knows it by
    command: list[str]
    cwd: Path
    env: dict[str, str]
    expected: int | None  # the tests it must run (None: a command that runs no libtest tests)
    groups: set[str]
    cost: int = 1  # the CPUs of the share it holds while it runs


@dataclass(frozen=True)
class Target:
    """A target of one build, run with the union of its groups' filters, and what it is given."""

    build: Build
    name: str
    filters: tuple[str, ...]
    args: tuple[str, ...]
    env: tuple[tuple[str, str], ...]


def invocations(crates: dict[str, Crate], chosen: list[Group], places: dict[str, str]) -> list[Invocation]:
    """The processes of the chosen groups: per build, environment and extra arguments, each test binary once with the union of
    its groups' filters (none when a group takes it whole), the doc tests through cargo, the unittest suites through Python."""
    keyed: dict[tuple, dict[str, tuple[set[str] | None, set[str]]]] = {}
    for group in chosen:
        for selector in group.selectors:
            targets = keyed.setdefault((selector.build, selector.args, selector.env), {})
            for target in [selector.target] if selector.target else ["*"]:
                filters, owners = targets.get(target, (set(), set()))
                merged = None if filters is None or not selector.filters else filters | set(selector.filters)
                targets[target] = (merged, owners | {group.name})
    runs = []
    for (build, args, env), targets in keyed.items():
        listed = list_build(crates, build)
        if build.profile == UNITTEST:
            owners = {o for _, owned in targets.values() for o in owned}
            runs.append(Invocation(f"unittest {build.crate}", unittest_command(build.crate), ROOT, {}, len(listed.tests[UNITTEST]), owners))
            continue
        if "*" in targets:  # the whole crate
            filters, owners = targets.pop("*")
            if filters is not None:
                raise Failure("filters without a target reached the runner (load_group refuses them)")
            targets = {t: (None, owners | targets.get(t, (None, set()))[1]) for t in listed.tests}
        for name, (filters, owners) in sorted(targets.items()):
            target = Target(build, name, tuple(sorted(filters or ())), args, env)
            runs.append(invocation(crates, listed, target, owners, places))
    return runs


def invocation(crates: dict[str, Crate], listed: Listed, target: Target, owners: set[str], places: dict[str, str]) -> Invocation:
    selector = Selector(target.build, None, target.filters, (), (), ())
    build = target.build
    selected = [n for n in listed.tests.get(target.name, []) if selects(selector, target.name, n)]
    recorders = sorted(set(selected) & MANUAL_RECORDERS.get((build.crate, target.name), frozenset()))
    omitted, local_args = local_conformance(build.crate, target.name, selected)
    expected = len(selected) - len(recorders) - len(omitted)
    skips = [argument for name in recorders + omitted for argument in ("--skip", name)]
    key = " ".join([build.crate, build.profile, ",".join(build.features), target.name, *target.filters, *target.args])
    env = {k: v.format(**places) for k, v in target.env}
    if target.name == "doc":  # rustdoc builds and runs them: only cargo can
        command = cargo_test(crates, build) + ["--doc", "--", *target.filters, *target.args]
        return Invocation(key, command, ROOT, env, expected, owners)
    crate = crates[build.crate]
    # as `cargo test` runs a test binary: from its crate's directory, told where that is
    env |= {"CARGO_MANIFEST_DIR": str(ROOT / crate.directory), "CARGO_PKG_NAME": crate.name}
    # Libtest applies --exact to both filters and --skip. Expand the already selected
    # names so substring group filters retain their meaning without suppressing name prefixes.
    command = [str(listed.executables[target.name]), "--exact", *selected, *target.args, *local_args, *skips]
    return Invocation(key, command, ROOT / crate.directory, env, expected, owners)


def target_directory() -> str:
    return metadata(None)["target_directory"]


def run(crates: dict[str, Crate], chosen: list[Group], record: Path | None, share: int | None) -> bool:
    """Runs the chosen groups: their `before` commands one after another, then their test binaries, doc tests and commands
    side by side within the CPU share (`share`, else cpu_share()), the longest of the last run first, stopping at the first
    failure; `record` gets the groups of the first failure. Its output files are its own and go with it."""
    places = {"target_dir": target_directory()}
    (Path(places["target_dir"]) / "test-groups-logs").mkdir(parents=True, exist_ok=True)
    logs = Path(tempfile.mkdtemp(prefix=f"{os.getpid()}-", dir=Path(places["target_dir"]) / "test-groups-logs"))
    try:
        return run_logged(crates, chosen, record, share, places, logs)
    finally:
        shutil.rmtree(logs, ignore_errors=True)


def run_logged(crates: dict[str, Crate], chosen: list[Group], record: Path | None, share: int | None,
               places: dict[str, str], logs: Path) -> bool:
    began = time.monotonic()
    share = share or cpu_share()
    inherited = os.environ.get("RUST_TEST_THREADS", "")
    cap = min(share, int(inherited)) if inherited.isdigit() and int(inherited) > 0 else share
    befores: dict[tuple[str, ...], str] = {}  # each command once, for the first group that asks for it
    for group in chosen:
        for selector in group.selectors:
            for command in selector.before:
                befores.setdefault(command, group.name)
    for command, name in befores.items():
        if not Pool(share, record, logs).run([Invocation(" ".join(command), list(command), ROOT, {}, None, {name}, share)]):
            return False
    history_file = Path(places["target_dir"]) / "test-groups-history.json"
    history = read_history(history_file)
    runs = invocations(crates, chosen, places)
    for each in runs:
        threads = 1 if each.key.startswith(f"{UNITTEST} ") else max(1, min(cap, each.expected or 1))
        each.env["RUST_TEST_THREADS"] = str(threads)
        each.cost = cost_of(history.get(each.key), threads)
    runs += [Invocation(" ".join(c), c, ROOT, {}, None, {group.name}, cost_of(history.get(" ".join(c)), cap))
             for group in chosen for c in group.commands]
    runs.sort(key=lambda each: -history.get(each.key, {}).get("seconds", float("inf")))
    pool = Pool(share, record, logs)
    passed = pool.run(runs)
    write_history(history_file, history | pool.history)
    if passed:
        names = " ".join(g.name for g in chosen)
        print(f"test-groups: {len(chosen)} groups passed in {time.monotonic() - began:.0f} s ({share} CPUs, at most {cap} test threads a binary): {names}")
    return passed


def read_history(path: Path) -> dict[str, dict[str, float]]:
    """What the last runs of each invocation took: its wall `seconds` and the `cores` it used. A file that cannot be read,
    and an entry that is not two finite, non-negative numbers, are as no history (a first run)."""
    def refuse(constant: str) -> float:
        raise ValueError(f"{constant} in the history")

    try:
        entries = json.loads(path.read_text(), parse_constant=refuse)
    except (OSError, ValueError):
        return {}
    if not isinstance(entries, dict):
        return {}

    def sound(entry: object) -> bool:
        fields = ("seconds", "cores")
        return isinstance(entry, dict) and all(
            isinstance(entry.get(f), (int, float)) and not isinstance(entry.get(f), bool) and math.isfinite(entry[f]) and entry[f] >= 0
            for f in fields
        )

    return {key: entry for key, entry in entries.items() if isinstance(key, str) and sound(entry)}


def write_history(path: Path, history: dict[str, dict[str, float]]) -> None:
    """The history, replaced whole: a run that stops or another run beside it never leaves half a file."""
    fresh = path.with_name(f"{path.name}.{os.getpid()}")
    fresh.write_text(json.dumps(history, indent=1, sort_keys=True))
    os.replace(fresh, path)


def cost_of(entry: dict[str, float] | None, threads: int) -> int:
    """The CPUs an invocation holds of the share: what it used last time, at least one, at most its threads."""
    if not entry or "cores" not in entry:
        return threads
    return max(1, min(threads, math.ceil(entry["cores"])))


class Pool:
    """Runs invocations side by side while their threads fit the share (one at a time when one alone does not), each in a
    process group of its own with its output in a file. The first failure (a test, a wrong count, a timeout, the runner
    itself, SIGINT or SIGTERM) stops the rest and is reported last."""

    def __init__(self, share: int, record: Path | None, logs: Path):
        self.share = share
        self.used = 0  # threads of the invocations in flight
        self.most = 0  # the most threads in flight at once
        self.room = threading.Condition()
        self.record = record
        self.logs = logs
        self.lock = threading.Lock()
        self.running: dict[int, subprocess.Popen] = {}
        self.failure: str | None = None  # the first failure's summary
        self.history: dict[str, dict[str, float]] = {}  # of the invocations that passed

    def run(self, runs: list[Invocation]) -> bool:
        self.logs.mkdir(parents=True, exist_ok=True)
        interrupted: list[int] = []

        def interrupt(number, _frame):
            interrupted.append(number)
            with self.lock:
                self.fail(f"test-groups: interrupted by {signal.Signals(number).name}", set())

        handlers = {number: signal.signal(number, interrupt) for number in (signal.SIGINT, signal.SIGTERM)}
        threads = []
        try:
            for index, each in enumerate(runs):
                self.take(each.cost)
                if self.failure is not None:
                    self.give(each.cost)
                    break
                thread = threading.Thread(target=self.execute, args=(each, self.logs / f"{index}.log"))
                thread.start()
                threads.append(thread)
        finally:
            for thread in threads:
                thread.join()
            for number, handler in handlers.items():
                signal.signal(number, handler)
        if self.failure is not None:
            print(self.failure, flush=True)  # last, where a tail of the output finds it
        if interrupted:
            raise KeyboardInterrupt if interrupted[0] == signal.SIGINT else SystemExit(128 + interrupted[0])
        return self.failure is None

    def execute(self, each: Invocation, log: Path) -> None:
        began = time.monotonic()
        try:
            shown = " ".join(f"{k}={v}" for k, v in each.env.items() if not k.startswith("CARGO_")) + " " + " ".join(each.command)
            print(f"test-groups: [{' '.join(sorted(each.groups))}] {shown.strip()}", flush=True)
            status, cpu_seconds = self.spawn(each, log)
            seconds = time.monotonic() - began
            with self.lock:
                if self.failure is not None and status != 0:
                    return  # stopped because another one failed
                if self.report(each, log, status, seconds):
                    self.history[each.key] = {"seconds": round(seconds, 1), "cores": round(cpu_seconds / max(seconds, 0.1), 2)}
        except BaseException as error:  # the runner itself failed: never a pass
            groups = " ".join(sorted(each.groups))
            with self.lock:
                self.fail(f"test-groups: FAILED after {time.monotonic() - began:.0f} s (the runner: {error!r}): groups {groups}", each.groups)
        finally:
            self.give(each.cost)

    def take(self, threads: int) -> None:
        """Waits until the share has room for `threads` (or nothing runs), then holds them."""
        with self.room:
            while self.used and self.used + threads > self.share:
                self.room.wait()
            self.used += threads
            self.most = max(self.most, self.used)

    def give(self, threads: int) -> None:
        with self.room:
            self.used -= threads
            self.room.notify_all()

    def spawn(self, each: Invocation, log: Path) -> tuple[int, float]:
        """Runs one invocation to its end or the timeout, its output into `log`, and leaves nothing of its process group
        behind; its exit status and the CPU seconds it and the children it waited for used."""
        with open(log, "wb") as out:
            with self.lock:
                if self.failure is not None:
                    return -signal.SIGKILL, 0.0  # stopped before it started
                process = subprocess.Popen(each.command, cwd=each.cwd, env=os.environ | each.env, stdout=out,
                                           stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
                self.running[process.pid] = process
            timed_out = not wait_unreaped(process.pid, TIMEOUT_SECONDS)
            # The binary is a zombie now, so its process group's id cannot be anyone else's yet: what it left running
            # (servers it started) goes with it.
            kill_group(process.pid)
            _, waited, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(waited)
            with self.lock:
                del self.running[process.pid]
            if timed_out:
                with open(log, "ab") as tail:
                    tail.write(f"\ntest-groups: stopped after {TIMEOUT_SECONDS} s\n".encode())
                return -signal.SIGKILL, 0.0
            return process.returncode, usage.ru_utime + usage.ru_stime

    def report(self, each: Invocation, log: Path, status: int, seconds: float) -> bool:
        """Under the lock: the invocation's output, its verdict (True: it passed), and on a failure the others stopped."""
        if self.failure is None:  # after the first failure only that one is told
            with open(log, "rb") as output:
                for line in output:
                    sys.stdout.write(line.decode(errors="replace"))
        problem = problem_of(log, status, each.expected)
        groups = " ".join(sorted(each.groups))
        if problem is None:
            if self.failure is None:
                print(f"test-groups: {seconds:.0f} s [{groups}]", flush=True)
            return True
        self.fail(f"test-groups: FAILED after {seconds:.0f} s ({problem}): groups {groups}", each.groups)
        return False

    def fail(self, summary: str, groups: set[str]) -> None:
        """Under the lock: the first failure is kept, recorded and told; every failure stops what still runs."""
        if self.failure is None:
            self.failure = summary
            print(summary, flush=True)
            if self.record is not None and groups:
                self.record.write_text("\n".join(sorted(groups)) + "\n")
        self.stop_others()

    def stop_others(self) -> None:
        """Under the lock: every process group still running is killed."""
        for pid in self.running:
            kill_group(pid)


def wait_unreaped(pid: int, seconds: float) -> bool:
    """Waits for a child to end without reaping it; False when it still runs after `seconds`."""
    deadline = time.monotonic() + seconds
    while os.waitid(os.P_PID, pid, os.WEXITED | os.WNOWAIT | os.WNOHANG) is None:
        if time.monotonic() > deadline:
            return False
        time.sleep(0.1)
    return True


def kill_group(pid: int) -> None:
    try:
        os.killpg(pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def problem_of(log: Path, status: int, expected: int | None) -> str | None:
    """Why a finished invocation failed, None when it passed: its exit code, or a count of tests other than its groups'."""
    if status != 0:
        return f"exit code {status}"
    ran = 0
    with open(log, "rb") as output:
        for line in (raw.decode(errors="replace") for raw in output):
            found = re.match(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured", line)
            if found:
                passed, failed, ignored, measured = (int(n) for n in found.groups())
                if failed or ignored or measured:
                    return f"{failed} failed, {ignored} ignored, {measured} measured tests; every selected contract must execute"
                ran += passed
            found = re.match(r"Ran (\d+) tests? in ", line)  # unittest
            if found:
                ran += int(found.group(1))
            if line.startswith("OK ("):
                skipped = re.search(r"\bskipped=(\d+)", line)
                if skipped and int(skipped.group(1)):
                    return f"{skipped.group(1)} skipped unittest contracts"
    if expected is not None and ran != expected:
        return f"it ran {ran} tests where its groups select {expected} (the selection and libtest's disagree)"
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="action", required=True)
    for action in ("check", "plan"):
        each = sub.add_parser(action)
        each.add_argument("--base", default=None, help="compare with the merge-base with this revision (default origin/main)")
        each.add_argument("--all", action="store_true", help="everything, whatever changed")
    sub.add_parser("available")
    running = sub.add_parser("run")
    running.add_argument("--record", type=Path, help="write the names of the groups that failed here")
    running.add_argument("--cpus", default=os.environ.get("TEST_GROUPS_CPUS"),
                         help="the CPU share the run's test threads keep within (default TEST_GROUPS_CPUS, else the CPUs it may use)")
    running.add_argument("groups", nargs="+")
    options = parser.parse_args()
    try:
        if options.action == "available":
            print(" ".join(available_optional(tomllib.loads(CONFIG.read_text()))))
            return 0
        crates, groups, routes = load()
        router = Router(crates, groups, routes)
        if options.action == "run":
            unknown = [n for n in options.groups if n not in router.names]
            if unknown:
                raise Failure(f"no such group: {' '.join(unknown)} (groups: {' '.join(router.names)})")
            cpus = None if options.cpus is None else options.cpus.strip()
            if cpus is not None and (not cpus.isdigit() or int(cpus) < 1):
                raise Failure(f"--cpus or TEST_GROUPS_CPUS {options.cpus!r}: a whole number of at least one")
            return 0 if run(crates, [g for g in groups if g.name in options.groups], options.record, int(cpus) if cpus else None) else 1
        merge_base, paths = (None, None) if options.all else changed(options.base or default_base())
        if options.action == "plan":
            print("\n".join(plan_report(router, merge_base, paths)))
            return 0
        problems = check(crates, groups, router, paths)
        for problem in problems:
            print(f"test-groups: {problem}")
        print(f"test-groups: {'FAILED' if problems else 'ok'}: {len(groups)} groups (at most {MAX_GROUPS})")
        return 1 if problems else 0
    except Failure as failure:
        print(f"test-groups: {failure}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        return 128 + signal.SIGINT


if __name__ == "__main__":
    sys.exit(main())
