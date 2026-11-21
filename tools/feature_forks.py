#!/usr/bin/env python3
"""Check that every test build of the workspace resolves each dependency with the same features.

Cargo resolves features per invocation, so two test commands that enable a dependency's features differently compile it, and
everything above it, twice. The invocations compared are the test groups' builds (tools/test_groups.toml: crate, profile and
features) and the whole-workspace test run with the script host's native build. A package whose host and target variants differ
is compared as the whole set of its variants. A workspace crate may differ only in features a group turns on for that crate
itself (that group exists to test them); any other difference fails and names the package and the invocations.
crates/bota_dev_features is the place to align a third-party package.
"""

from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TEST_EDGES = ("--edges", "normal,build,dev")


def invocations() -> tuple[dict[tuple[str, ...], str], dict[str, set[str]]]:
    config = tomllib.loads(ROOT.joinpath("tools/test_groups.toml").read_text())
    crates = config.get("crate", {})
    found = {(*TEST_EDGES, "--workspace", "--features", "bota_lua/native"): "workspace tests"}
    own = {}   # crate -> the invocations of its own groups (their features of that crate are its own)
    for group in config.get("group", []):
        for test in group.get("tests", []):
            crate = test.get("crate")
            if not crate or crates.get(crate, {}).get("manifest"):
                continue   # Python suites; crates outside the workspace build in a graph of their own
            features = sorted(test.get("features", crates.get(crate, {}).get("features", [])))
            arguments = (*TEST_EDGES, "-p", crate, *(("--features", ",".join(features)) if features else ()))
            found.setdefault(arguments, f"{crate} tests" + (f" ({', '.join(features)})" if features else ""))
            own.setdefault(crate, set()).add(arguments)
    return found, own


def features_for(label: str, arguments: tuple[str, ...]) -> dict[str, set[frozenset[str]]]:
    command = ["cargo", "tree", "--locked", "--offline", "--color", "never", "--prefix", "none", "--format", "{p}\t{f}", *arguments]
    try:
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=300)
    except subprocess.TimeoutExpired:
        raise ValueError(f"{label}: cargo tree took longer than 300 s") from None
    if result.returncode:
        raise ValueError(f"{label}: cargo tree failed\n{result.stderr.strip()}")
    features = {}
    for line in result.stdout.splitlines():
        if not line:
            continue
        package, separator, names = line.partition("\t")
        if not separator:
            raise ValueError(f"{label}: unrecognized cargo tree row: {line!r}")
        features.setdefault(package, set()).add(frozenset(filter(None, names.removesuffix(" (*)").split(","))))
    if not features:
        raise ValueError(f"{label}: cargo tree listed no packages")
    return features


def describe(feature_sets: frozenset[frozenset[str]]) -> str:
    return " or ".join("[" + ", ".join(names) + "]" for names in sorted(map(sorted, feature_sets)))


def main() -> int:
    try:
        found, own = invocations()
        by_package, own_features = {}, {}
        for arguments, label in found.items():
            for package, features in features_for(label, arguments).items():
                by_package.setdefault(package, {}).setdefault(frozenset(features), []).append(label)
                name = package.split()[0]
                if arguments in own.get(name, ()):   # what a crate's own groups turn on for it, implied features included
                    own_features.setdefault(name, set()).update(*features)
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        print(f"feature forks: {error}", file=sys.stderr)
        return 1
    forks = {}
    for package, variants in by_package.items():
        if len(variants) < 2:
            continue
        name = package.split()[0]
        differing = set().union(*(set().union(*v) for v in variants)) - set.intersection(*(set().union(*v) for v in variants))
        if " (" in package and differing <= own_features.get(name, set()):
            continue   # a workspace crate built with the features its own group tests
        forks[package] = variants
    for package, variants in sorted(forks.items()):
        print(package.split(" (", 1)[0], file=sys.stderr)
        for features, labels in variants.items():
            print(f"  {', '.join(labels)}: {describe(features)}", file=sys.stderr)
    print(f"feature forks: {len(forks)} packages")
    return int(bool(forks))


if __name__ == "__main__":
    sys.exit(main())
