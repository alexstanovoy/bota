#!/usr/bin/env python3
"""The test groups of this repository (tools/test_groups.toml): which tests a change runs, and the proof that none is left out.

    tools/test_groups.py check [--base REV | --all]   every test in exactly one group, every file routed, at most 30 groups
    tools/test_groups.py plan [--base REV | --all]    the groups and crates the change since the merge-base with REV affects
    tools/test_groups.py run [--record FILE] [--jobs N] GROUP...
                                                      the groups' tests and commands, N at a time (default 4,
                                                      TEST_GROUPS_JOBS), stopping at the first failure

REV defaults to origin/main (main where there is no origin). `check` lists the tests of the crates the change can touch
(every crate with --all) by building their test targets in each build the groups use and with all their features; a crate
outside the change keeps the tests and the groups it had at the merge-base, which passed. `run` runs the test binaries
the listing built directly, several at once and the longest of the last run first (target/test-groups-seconds.json), and
checks that each ran exactly as many tests as its groups select, so this selection and libtest's own cannot drift apart.
`plan` prints four lines: `groups NAME...`, `crates NAME...` (the workspace crates to lint), `fuzz 0|1` and `scope`
(what a stamp of the check names besides the tree).
"""

import argparse
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip())
CONFIG = ROOT / "tools/test_groups.toml"
PROFILE = "test-release"
# A unittest suite is a build of its own: its directory stands where a crate's name does.
UNITTEST = "unittest"
# At most this many groups: few enough to know each by name, enough to follow the code's crates and subsystems.
MAX_GROUPS = 30


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


@dataclass
class Listed:
    """The tests a build of a crate holds: per target, its names and the file the target starts at."""

    tests: dict[str, list[str]]
    roots: dict[str, Path]
    executables: dict[str, Path] = field(default_factory=dict)


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
    workspace = metadata(None)
    for name, settings in config["crate"].items():
        manifest = settings.get("manifest")
        packages = metadata(manifest)["packages"] if manifest else workspace["packages"]
        package = next((p for p in packages if p["name"] == name), None)
        if package is None:
            raise Failure(f"test_groups.toml: crate {name} is no package of the repository")
        table = package["features"]
        wasm_only = set(settings.get("wasm_only_features", []))
        manifest_toml = tomllib.loads(Path(package["manifest_path"]).read_text())
        crates[name] = Crate(
            name=name,
            directory=str(Path(package["manifest_path"]).parent.relative_to(ROOT)),
            manifest=manifest,
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
        for dependency in package["dependencies"]:
            if dependency.get("path") and dependency["name"] in crates:
                crates[dependency["name"]].dependents.add(name)
    return crates


def load() -> tuple[dict[str, Crate], list[Group], list[tuple[list[str], list[str]]]]:
    config = tomllib.loads(CONFIG.read_text())
    crates = load_crates(config)
    groups = [load_group(entry, crates) for entry in config.get("group", [])]
    routes = [(route["paths"], route["groups"]) for route in config.get("route", [])]
    return crates, groups, routes


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


def plan(router: Router, paths: list[str] | None) -> tuple[set[str], set[str], bool]:
    """Groups to run, workspace crates to lint and whether the fuzz targets are to be checked; `paths` None: everything."""
    workspace = router.workspace()
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
        if routed[0] and crate is not None and crate.manifest is None:
            touched.add(crate.name)
    crates: set[str] = set()
    for crate in touched:
        crates |= router.above(crate, launched=False) & workspace
    return groups, crates, bool(crates) or any(p.startswith("fuzz/") for p in paths)


def plan_report(router: Router, merge_base: str | None, paths: list[str] | None) -> list[str]:
    """`plan`'s lines: the groups, the crates to lint, whether to check the fuzz targets, and the scope of the stamps."""
    chosen, lint, fuzz = plan(router, paths)
    ordered = [n for n in router.names if n in chosen]
    everything = (set(router.names), router.workspace(), True) == (chosen, lint, fuzz)
    scope = scope_of(None if everything else merge_base, ordered, sorted(lint), fuzz)
    return [" ".join(["groups", *ordered]), " ".join(["crates", *sorted(lint)]), f"fuzz {int(fuzz)}", f"scope {scope}"]


def scope_of(merge_base: str | None, groups: list[str], crates: list[str], fuzz: bool) -> str:
    """What a stamp of a check covers besides the tree: `full` (everything, also when a change selects it all), or what was
    compared with and what that selected."""
    if merge_base is None:
        return "full"
    selection = json.dumps([merge_base, groups, crates, fuzz])
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
    if has_doc:
        doc = subprocess.run(cargo_test(crates, build) + ["--doc", "--", "--list", "--format", "terse"], cwd=ROOT, capture_output=True, text=True)
        if doc.returncode != 0:
            raise Failure(f"listing the doc tests of {build} failed:\n{doc.stderr[-2000:]}")
        listed.tests["doc"] = tests_of(doc.stdout)
    return listed


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
    problems = check_names(groups, router)
    every_file = files()
    problems += check_patterns(groups, router, every_file)
    problems += [f"{path}: no group and no route names it (tools/test_groups.toml)" for path in every_file if router.route(path) is None]
    problems += check_harnesses(crates, groups, every_file)
    problems += check_markers(router, every_file)
    scope = set(crates) if paths is None else set()
    for path in paths or []:
        routed = router.route(path)
        crate = router.crate_of(path)
        if routed is None or routed[1]:
            scope = set(crates)
            break
        if crate is not None:
            scope |= router.above(crate.name)
    return problems + check_tests(crates, groups, router, scope)


# What a test is written with, in Rust and in Python: a file holding one must run some group.
TEST_MARKERS = {".rs": ("#[test]", "#[cfg(test)]"), ".py": ("def test_", "unittest.TestCase")}


def check_markers(router: Router, every_file: list[str]) -> list[str]:
    """A file a route sends to no group (fuzz/, scripts/) may hold no test: no group would run it."""
    problems = []
    for path in every_file:
        markers = TEST_MARKERS.get(Path(path).suffix)
        routed = router.route(path)
        if not markers or routed is None or routed[0] or not (ROOT / path).is_file():
            continue
        found = [m for m in markers if m in (ROOT / path).read_text(errors="replace")]
        if found:
            problems.append(f"{path}: holds tests ({', '.join(found)}) but a change to it runs no group, so none runs them")
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


def check_tests(crates: dict[str, Crate], groups: list[Group], router: Router, scope: set[str]) -> list[str]:
    """Every test of the crates in `scope`, in every build a group uses and with all features, is in exactly one group, and a
    change to the file it is written in runs that group."""
    builds = {s.build for g in groups for s in g.selectors if s.build.crate in scope or s.build.profile == UNITTEST}
    builds |= {Build(c, crates[c].profile, crates[c].all_features) for c in scope}
    return assess(groups, router, {build: list_build(crates, build) for build in sorted(builds, key=str)})


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


# How many test binaries run at once unless --jobs or TEST_GROUPS_JOBS says otherwise (each runs its tests on libtest's own
# threads, RUST_TEST_THREADS). Measured on the full run (32 threads, RUST_TEST_THREADS=16): 1 at a time 209 s, 2 93 s, 4 59 s,
# 6 60 s, 8 70 s; at 4 the run waits on its longest binary (tests/malformed.rs), and the processes peak under 1.1 GB.
JOBS = 4


@dataclass
class Invocation:
    """One process of a run: a prebuilt test binary with its filters, or a cargo or unittest command."""

    key: str  # what the run's timing history knows it by
    command: list[str]
    cwd: Path
    env: dict[str, str]
    expected: int | None  # the tests it must run (None: a command that runs no libtest tests)
    groups: set[str]


def invocations(crates: dict[str, Crate], chosen: list[Group]) -> list[Invocation]:
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
            _, owners = targets.pop("*")
            targets = {t: (None, owners | targets.get(t, (None, set()))[1]) for t in listed.tests}
        for target, (filters, owners) in sorted(targets.items()):
            runs.append(invocation(crates, build, args, dict(env), listed, target, sorted(filters or ()), owners))
    return runs


def invocation(crates, build, args, env, listed, target, filters, owners) -> Invocation:
    selector = Selector(build, None, tuple(filters), (), (), ())
    expected = sum(1 for n in listed.tests.get(target, []) if selects(selector, target, n))
    key = " ".join([build.crate, build.profile, ",".join(build.features), target, *filters, *args])
    if target == "doc":  # rustdoc builds and runs them: only cargo can
        return Invocation(key, cargo_test(crates, build) + ["--doc", "--", *filters, *args], ROOT, env, expected, owners)
    crate = crates[build.crate]
    # as `cargo test` runs a test binary: from its crate's directory, told where that is
    env |= {"CARGO_MANIFEST_DIR": str(ROOT / crate.directory), "CARGO_PKG_NAME": crate.name}
    command = [str(listed.executables[target]), *filters, *args]
    return Invocation(key, command, ROOT / crate.directory, env, expected, owners)


def target_directory() -> str:
    return metadata(None)["target_directory"]


def run(crates: dict[str, Crate], chosen: list[Group], record: Path | None, jobs: int) -> bool:
    """Runs the chosen groups: their `before` commands one after another, then their test binaries, doc tests and commands
    `jobs` at a time, the longest of the last run first, stopping at the first failure; `record` gets the failed groups."""
    began = time.monotonic()
    places = {"target_dir": target_directory()}
    befores = {command: group.name for group in reversed(chosen) for s in group.selectors for command in s.before}
    for command, name in befores.items():
        if not Pool(1, record).run([Invocation(" ".join(command), list(command), ROOT, {}, None, {name})]):
            return False
    runs = [
        Invocation(each.key, each.command, each.cwd, {k: v.format(**places) for k, v in each.env.items()}, each.expected, each.groups)
        for each in invocations(crates, chosen)
    ]
    runs += [Invocation(" ".join(c), c, ROOT, {}, None, {group.name}) for group in chosen for c in group.commands]
    history = Path(places["target_dir"]) / "test-groups-seconds.json"
    try:
        seconds = json.loads(history.read_text())
    except (OSError, ValueError):
        seconds = {}
    runs.sort(key=lambda each: -seconds.get(each.key, float("inf")))
    pool = Pool(jobs, record)
    passed = pool.run(runs)
    history.write_text(json.dumps(seconds | pool.seconds, indent=0, sort_keys=True))
    if passed:
        print(f"test-groups: {len(chosen)} groups passed in {time.monotonic() - began:.0f} s ({jobs} at a time): {' '.join(g.name for g in chosen)}")
    return passed


class Pool:
    """Runs invocations a few at a time, each in a process group of its own; the first failure stops the rest."""

    def __init__(self, jobs: int, record: Path | None):
        self.slots = threading.Semaphore(jobs)
        self.record = record
        self.lock = threading.Lock()
        self.running: dict[int, subprocess.Popen] = {}
        self.failed = False
        self.seconds: dict[str, float] = {}

    def run(self, runs: list[Invocation]) -> bool:
        threads = []
        for each in runs:
            self.slots.acquire()
            if self.failed:
                self.slots.release()
                break
            thread = threading.Thread(target=self.execute, args=(each,))
            thread.start()
            threads.append(thread)
        for thread in threads:
            thread.join()
        return not self.failed

    def execute(self, each: Invocation) -> None:
        try:
            shown = " ".join(f"{k}={v}" for k, v in each.env.items() if not k.startswith("CARGO_")) + " " + " ".join(each.command)
            print(f"test-groups: [{' '.join(sorted(each.groups))}] {shown.strip()}", flush=True)
            began = time.monotonic()
            process = subprocess.Popen(each.command, cwd=each.cwd, env=os.environ | each.env, stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, text=True, start_new_session=True)
            with self.lock:
                self.running[process.pid] = process
            output, _ = process.communicate()
            with self.lock:
                del self.running[process.pid]
                self.report(each, output, process.returncode, time.monotonic() - began)
        finally:
            self.slots.release()

    def report(self, each: Invocation, output: str, status: int, seconds: float) -> None:
        """Under the lock: the invocation's output whole, its verdict, and on a failure the others stopped."""
        if self.failed and status != 0:
            return  # stopped because another one failed
        sys.stdout.write(output)
        problem = problem_of(output, status, each.expected)
        if problem is None:
            self.seconds[each.key] = round(seconds, 1)
            print(f"test-groups: {seconds:.0f} s [{' '.join(sorted(each.groups))}]", flush=True)
            return
        print(f"test-groups: FAILED after {seconds:.0f} s ({problem}): groups {' '.join(sorted(each.groups))}", flush=True)
        self.failed = True
        if self.record is not None:
            self.record.write_text("\n".join(sorted(each.groups)) + "\n")
        for process in self.running.values():
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def problem_of(output: str, status: int, expected: int | None) -> str | None:
    """Why a finished invocation failed, None when it passed: its exit code, or a count of tests other than its groups'."""
    if status != 0:
        return f"exit code {status}"
    ran = 0
    for line in output.splitlines():
        found = re.match(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured", line)
        if found:
            ran += sum(int(n) for n in found.groups())
        found = re.match(r"Ran (\d+) tests? in ", line)  # unittest
        if found:
            ran += int(found.group(1))
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
    running = sub.add_parser("run")
    running.add_argument("--record", type=Path, help="write the names of the groups that failed here")
    running.add_argument("--jobs", type=int, default=int(os.environ.get("TEST_GROUPS_JOBS", JOBS)),
                         help=f"test binaries at once (default TEST_GROUPS_JOBS, else {JOBS})")
    running.add_argument("groups", nargs="+")
    options = parser.parse_args()
    try:
        crates, groups, routes = load()
        router = Router(crates, groups, routes)
        if options.action == "run":
            unknown = [n for n in options.groups if n not in router.names]
            if unknown:
                raise Failure(f"no such group: {' '.join(unknown)} (groups: {' '.join(router.names)})")
            if options.jobs < 1:
                raise Failure(f"--jobs {options.jobs}: at least one")
            return 0 if run(crates, [g for g in groups if g.name in options.groups], options.record, options.jobs) else 1
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


if __name__ == "__main__":
    sys.exit(main())
