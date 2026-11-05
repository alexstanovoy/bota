#!/usr/bin/env python3
"""The test groups of this repository (tools/test_groups.toml): which tests a change runs, and the proof that none is left out.

    tools/test_groups.py check [--base REV | --all]   every test in exactly one group, every file routed, at most 30 groups
    tools/test_groups.py plan [--base REV | --all]    the groups and crates the change since the merge-base with REV affects
    tools/test_groups.py run [--record FILE] GROUP... the groups' tests and commands, stopping at the first failure

REV defaults to origin/main (main where there is no origin). `check` lists the tests of the crates the change can touch
(every crate with --all) by building their test targets in each build the groups use and with all their features; a crate
outside the change keeps the tests and the groups it had at the merge-base, which passed. `run` checks that every cargo
invocation ran exactly as many tests as its groups select, so this selection and libtest's own cannot drift apart.
`plan` prints three lines: `groups NAME...`, `crates NAME...` (the workspace crates to lint) and `fuzz 0|1`.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip())
CONFIG = ROOT / "tools/test_groups.toml"
PROFILE = "test-release"
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


@dataclass
class Listed:
    """The tests a build of a crate holds: per target, its names and the file the target starts at."""

    tests: dict[str, list[str]]
    roots: dict[str, Path]


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
        self.also = [([glob_regex(p) for p in group.also_paths], group.name) for group in groups]

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
        crate = self.crate_of(path)
        if crate is not None and not crate.routed:
            groups |= self.crate_wide(crate.name)
            named = True
        if not named:
            return None
        groups |= {name for patterns, name in self.also if any(p.match(path) for p in patterns)}
        return groups, everything


def files() -> list[str]:
    return sorted(set(git("ls-files").splitlines()) | set(git("ls-files", "--others", "--exclude-standard").splitlines()))


def changed(base: str) -> list[str]:
    found = subprocess.run(["git", "merge-base", "HEAD", base], cwd=ROOT, capture_output=True, text=True)
    if found.returncode != 0:
        raise Failure(f"no merge-base of HEAD and {base}: {found.stderr.strip()}")
    merge_base = found.stdout.strip()
    diff = git("diff", "--name-only", merge_base).splitlines()
    return sorted(set(diff) | set(git("ls-files", "--others", "--exclude-standard").splitlines()))


def default_base() -> str:
    found = subprocess.run(["git", "rev-parse", "-q", "--verify", "origin/main"], cwd=ROOT, capture_output=True)
    return "origin/main" if found.returncode == 0 else "main"


def plan(router: Router, paths: list[str] | None) -> tuple[set[str], set[str], bool]:
    """Groups to run, workspace crates to lint and whether the fuzz targets are to be checked; `paths` None: everything."""
    workspace = {name for name, crate in router.crates.items() if crate.manifest is None}
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
        if crate is not None and crate.manifest is None:
            touched.add(crate.name)
    crates: set[str] = set()
    for crate in touched:
        crates |= router.above(crate, launched=False) & workspace
    return groups, crates, bool(crates) or any(p.startswith("fuzz/") for p in paths)


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
    if has_doc:
        doc = subprocess.run(cargo_test(crates, build) + ["--doc", "--", "--list", "--format", "terse"], cwd=ROOT, capture_output=True, text=True)
        if doc.returncode != 0:
            raise Failure(f"listing the doc tests of {build} failed:\n{doc.stderr[-2000:]}")
        listed.tests["doc"] = tests_of(doc.stdout)
    return listed


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
    owners = [(group.paths + group.also_paths, f"group {group.name}") for group in groups]
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
    builds = {s.build for g in groups for s in g.selectors if s.build.crate in scope}
    builds |= {Build(c, crates[c].profile, crates[c].all_features) for c in scope}
    owners: dict[tuple[str, str, str], set[str]] = {}
    files_of: dict[tuple[str, str, str], Path] = {}
    problems = []
    for build in sorted(builds, key=str):
        listed = list_build(crates, build)
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


@dataclass
class Invocation:
    command: list[str]
    env: dict[str, str]
    expected: int
    groups: set[str]


def invocations(crates: dict[str, Crate], chosen: list[Group]) -> list[Invocation]:
    """The cargo runs of the chosen groups: per build, environment and extra arguments, the targets run whole in one run, and
    the targets run with filters in one run per filter set; doc tests apart (cargo runs them on their own)."""
    keyed: dict[tuple, dict[str, tuple[set[str] | None, set[str]]]] = {}
    for group in chosen:
        for selector in group.selectors:
            key = (selector.build, selector.args, selector.env)
            targets = keyed.setdefault(key, {})
            for target in [selector.target] if selector.target else ["*"]:
                filters, owners = targets.get(target, (set(), set()))
                merged = None if filters is None or not selector.filters else filters | set(selector.filters)
                targets[target] = (merged, owners | {group.name})
    runs = []
    for (build, args, env), targets in keyed.items():
        listed = list_build(crates, build)
        if "*" in targets:  # the whole crate
            filters, owners = targets.pop("*")
            targets = {t: (None, owners | targets.get(t, (None, set()))[1]) for t in listed.tests}
        plain = sorted(t for t, (f, _) in targets.items() if f is None and t != "doc")
        batches = [(plain, None)] if plain else []
        batches += [([t], f) for t, (f, _) in sorted(targets.items()) if f is not None and t != "doc"]
        if "doc" in targets:
            batches.append((["doc"], targets["doc"][0]))
        for names, filters in batches:
            runs.append(invocation(crates, build, args, dict(env), listed, names, filters, {o for t in names for o in targets[t][1]}))
    return runs


def invocation(crates, build, args, env, listed, names, filters, owners) -> Invocation:
    flags = []
    for target in names:
        if target == "doc":
            flags.append("--doc")
        elif target == "lib":
            flags.append("--lib")
        else:
            kind, name = target.split(":", 1)
            flags += [f"--{kind}", name]
    selector = Selector(build, None, tuple(sorted(filters or ())), (), (), ())
    expected = sum(1 for t in names for n in listed.tests.get(t, []) if selects(selector, t, n))
    command = cargo_test(crates, build) + flags + ["--", *sorted(filters or ()), *args]
    return Invocation(command, env, expected, owners)


def target_directory() -> str:
    return metadata(None)["target_directory"]


def run(crates: dict[str, Crate], chosen: list[Group], record: Path | None) -> bool:
    """Runs the chosen groups, stopping at the first failure; `record` gets the names of the groups that failed."""
    began = time.monotonic()
    done_before: set[tuple[str, ...]] = set()
    places = {"target_dir": target_directory()}
    for group in chosen:
        for selector in group.selectors:
            for command in selector.before:
                if command not in done_before:
                    done_before.add(command)
                    if not execute(list(command), {}, None, {group.name}, record):
                        return False
    for each in invocations(crates, chosen):
        env = {k: v.format(**places) for k, v in each.env.items()}
        if not execute(each.command, env, each.expected, each.groups, record):
            return False
    for group in chosen:
        for command in group.commands:
            if not execute(command, {}, None, {group.name}, record):
                return False
    print(f"test-groups: {len(chosen)} groups passed in {time.monotonic() - began:.0f} s: {' '.join(g.name for g in chosen)}")
    return True


def execute(command: list[str], env: dict[str, str], expected: int | None, groups: set[str], record: Path | None) -> bool:
    shown = " ".join(f"{k}={v}" for k, v in env.items()) + (" " if env else "") + " ".join(command)
    print(f"test-groups: [{' '.join(sorted(groups))}] {shown}", flush=True)
    began = time.monotonic()
    process = subprocess.Popen(command, cwd=ROOT, env=os.environ | env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    ran = 0
    for line in process.stdout:
        sys.stdout.write(line)
        found = re.match(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured", line)
        if found:
            ran += sum(int(n) for n in found.groups())
    status = process.wait()
    seconds = time.monotonic() - began
    problem = None
    if status != 0:
        problem = f"exit code {status}"
    elif expected is not None and ran != expected:
        problem = f"it ran {ran} tests where its groups select {expected} (the selection and libtest's disagree)"
    if problem is None:
        print(f"test-groups: {seconds:.0f} s", flush=True)
        return True
    print(f"test-groups: FAILED after {seconds:.0f} s ({problem}): groups {' '.join(sorted(groups))}", flush=True)
    if record is not None:
        record.write_text("\n".join(sorted(groups)) + "\n")
    return False


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="action", required=True)
    for action in ("check", "plan"):
        each = sub.add_parser(action)
        each.add_argument("--base", default=None, help="compare with the merge-base with this revision (default origin/main)")
        each.add_argument("--all", action="store_true", help="everything, whatever changed")
    running = sub.add_parser("run")
    running.add_argument("--record", type=Path, help="write the names of the groups that failed here")
    running.add_argument("groups", nargs="+")
    options = parser.parse_args()
    try:
        crates, groups, routes = load()
        router = Router(crates, groups, routes)
        if options.action == "run":
            unknown = [n for n in options.groups if n not in router.names]
            if unknown:
                raise Failure(f"no such group: {' '.join(unknown)} (groups: {' '.join(router.names)})")
            return 0 if run(crates, [g for g in groups if g.name in options.groups], options.record) else 1
        paths = None if options.all else changed(options.base or default_base())
        if options.action == "plan":
            chosen, lint, fuzz = plan(router, paths)
            print("groups", *[n for n in router.names if n in chosen])
            print("crates", *sorted(lint))
            print("fuzz", int(fuzz))
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
