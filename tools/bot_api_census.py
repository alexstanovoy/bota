#!/usr/bin/env python3
"""Bounded source-only API inventory for a hero's script entry/module closure.

The catalogue comes from `bota_botscript --api-catalogue`, not a copied API list.
This is a conservative static closure: branches in shared helpers are included.
Unresolved detected imports are explicit; dynamic reachability is not proven. Presence coverage
does not claim signature, gameplay or native-behavior parity.
"""
import argparse
from collections import defaultdict
import json
import os
from pathlib import Path
import re
import stat

MAX_SOURCE = 1 << 20
MAX_TOTAL = 32 << 20
MAX_ENTRIES = 16384
MAX_MODULES = 2048
MAX_TOKENS = 1_000_000
MAX_ANALYSIS_WORK = 8_000_000
MAX_IMPORT_EXPRESSION = 512
MODES = "laning attack roam retreat secret_shop side_shop rune push_tower_top push_tower_mid push_tower_bot defend_tower_top defend_tower_mid defend_tower_bot assemble team_roam farm defend_ally evasive_maneuvers roshan item ward outpost".split()
HOOKS = {"Think", "GetDesire", "OnStart", "OnEnd", "MinionThink", "ItemPurchaseThink", "AbilityUsageThink", "ItemUsageThink", "CourierUsageThink", "BuybackUsageThink", "AbilityLevelUpThink", "TeamThink", "UpdatePushLaneDesires", "UpdateDefendLaneDesires", "UpdateFarmLaneDesires", "UpdateRoamDesire", "UpdateRoshanDesire", "UpdateLaneAssignments", "GetBotNames"}
LONG = re.compile(r"\[(=*)\[")
IDENT = re.compile(r"[A-Za-z_][A-Za-z_0-9]*")


def spend(budget, kind, amount):
    budget[kind] -= amount
    if budget[kind] < 0:
        raise ValueError(f"analysis {kind} limit")


def quoted(source, index):
    quote, value, opaque = source[index], [], False
    index += 1
    escapes = {"a": "\a", "b": "\b", "f": "\f", "n": "\n", "r": "\r", "t": "\t", "v": "\v", "\\": "\\", '"': '"', "'": "'", "\n": "\n"}
    while index < len(source) and source[index] != quote:
        if source[index] != "\\":
            value.append(source[index])
            index += 1
            continue
        index += 1
        if index >= len(source):
            break
        if source[index] in "0123456789":
            end = index
            while end < min(index + 3, len(source)) and source[end] in "0123456789":
                end += 1
            number = int(source[index:end])
            opaque |= number > 255
            value.append(chr(min(number, 255)))
            index = end
        else:
            opaque |= source[index] not in escapes
            value.append(escapes.get(source[index], source[index]))
            index += 1
    return min(index + 1, len(source)), ("opaque_string" if opaque else "string"), "".join(value)


def tokens(source, budget=None):
    """Linear lexer; comments never produce calls, string contents stay opaque."""
    result, index, line = [], 0, 1
    budget = budget if budget is not None else {"tokens": MAX_TOKENS, "work": MAX_ANALYSIS_WORK}
    while index < len(source):
        start = index
        before = len(result)
        comment = source.startswith("--", index)
        begin = index + 2 if comment else index
        long = LONG.match(source, begin)
        if long:
            close = "]" + long[1] + "]"
            end = source.find(close, long.end())
            index = len(source) if end < 0 else end + len(close)
            if not comment:
                result.append(("string", source[long.end():end if end >= 0 else len(source)], line))
        elif comment:
            end = source.find("\n", begin)
            index = len(source) if end < 0 else end
        elif source[index] in "\"'":
            index, kind, value = quoted(source, index)
            result.append((kind, value, line))
        elif source[index].isspace():
            index += 1
        else:
            name = IDENT.match(source, index)
            if name:
                index = name.end()
                result.append(("name", name[0], line))
            else:
                punctuation = next((operator for operator in ("...", "..", "::", "==", "~=", "<=", ">=") if source.startswith(operator, index)), source[index])
                index += len(punctuation)
                result.append(("symbol", punctuation, line))
        line += source.count("\n", start, index)
        spend(budget, "tokens", len(result) - before)
    return result


def read_pinned(pinned, name, maximum):
    info = os.fstat(pinned)
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise ValueError(f"nonregular or linked source: {name!r}")
    if info.st_size > maximum:
        raise ValueError(f"source size limit: {name!r}")
    with open(f"/proc/self/fd/{pinned}", "rb") as source:
        data = source.read(maximum + 1)
        if len(data) > maximum or os.fstat(source.fileno()).st_nlink != 1:
            raise ValueError(f"source size or link limit: {name!r}")
    return data


def read_catalogue(path):
    pinned = os.open(path, os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        text = read_pinned(pinned, "catalogue", MAX_SOURCE).decode("utf-8")
    finally:
        os.close(pinned)
    catalogue = defaultdict(set)
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        namespace, name = line.split("\t")
        if not namespace or not name or len(namespace) > 64 or len(name) > 256:
            raise ValueError("invalid catalogue name")
        catalogue[namespace].add(name)
    return catalogue


def sources(root, budget=None):
    """Pin directories and classify files before reading; never follow pack links."""
    result = {}
    budget = budget if budget is not None else {"bytes": MAX_TOTAL, "entries": MAX_ENTRIES, "modules": MAX_MODULES}
    def walk(directory, prefix, depth):
        if depth > 16:
            raise ValueError("directory depth limit")
        with os.scandir(directory) as listing:
            for entry in listing:
                budget["entries"] -= 1
                if budget["entries"] < 0:
                    raise ValueError("directory entry limit")
                name = prefix + entry.name
                if len(name.encode("utf-8")) > 256:
                    raise ValueError("module path limit")
                pinned = os.open(entry.name, os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=directory)
                try:
                    info = os.fstat(pinned)
                    if stat.S_ISLNK(info.st_mode):
                        raise ValueError(f"symbolic link refused: {name!r}")
                    if stat.S_ISDIR(info.st_mode):
                        readable = os.open(f"/proc/self/fd/{pinned}", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
                        try:
                            walk(readable, name + "/", depth + 1)
                        finally:
                            os.close(readable)
                    elif name.endswith(".lua"):
                        if budget["modules"] <= 0:
                            raise ValueError("module count limit")
                        data = read_pinned(pinned, name, min(MAX_SOURCE, budget["bytes"]))
                        budget["bytes"] -= len(data)
                        budget["modules"] -= 1
                        result[name] = data.decode("utf-8-sig", errors="replace")
                finally:
                    os.close(pinned)
    directory = os.open(Path(root).resolve(), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        walk(directory, "", 0)
    finally:
        os.close(directory)
    return result


def argument(stream, index):
    if index >= len(stream):
        return []
    if stream[index][0] in ("string", "opaque_string"):
        return [stream[index]]
    if stream[index][:2] != ("symbol", "("):
        return []
    depth, end = 1, index + 1
    while end < len(stream):
        if end - index > MAX_IMPORT_EXPRESSION:
            raise ValueError("import expression limit")
        kind, value, _ = stream[end]
        if kind != "symbol":
            end += 1
            continue
        if value == "(":
            depth += 1
        elif value == ")":
            depth -= 1
            if not depth:
                return stream[index + 1:end]
        elif value == "," and depth == 1:
            return stream[index + 1:end]
        end += 1
    return []


def module_name(expression, hero):
    # These are the engine's conventional module-name expressions, not an evaluator.
    text = [(kind, value) for kind, value, _ in expression]
    directory = [("name", "GetScriptDirectory"), ("symbol", "("), ("symbol", ")")]
    unit = [("name", "string"), ("symbol", "."), ("name", "gsub"), ("symbol", "("), ("name", "GetBot"), ("symbol", "("), ("symbol", ")"), ("symbol", ":"), ("name", "GetUnitName"), ("symbol", "("), ("symbol", ")"), ("symbol", ","), ("string", "npc_dota_hero_"), ("symbol", ","), ("string", ""), ("symbol", ")")]
    for pattern, replacement in [(directory, "bots"), (unit, hero)]:
        index = 0
        while index <= len(text) - len(pattern):
            if text[index:index + len(pattern)] == pattern:
                text[index:index + len(pattern)] = [("string", replacement)]
            index += 1
    if not text or len(text) % 2 == 0 or any(kind != "string" if index % 2 == 0 else (kind, value) != ("symbol", "..") for index, (kind, value) in enumerate(text)):
        return None
    name = "".join(value for kind, value in text if kind == "string").replace("\\", "/")
    if name.endswith(".lua"):
        name = name[:-4]
    if "/" not in name:
        name = name.replace(".", "/")
    if name.startswith("/") or any(part in ("", ".", "..") for part in name.split("/")):
        return None
    return (name if name.startswith(("bots/", "game/")) else "bots/" + name) + ".lua"


def declarations(stream):
    names = set()
    for index, (kind, value, _) in enumerate(stream):
        if kind == "name" and value == "function" and index + 1 < len(stream) and stream[index + 1][0] == "name":
            end = index + 1
            while end + 2 < len(stream) and stream[end + 1][:2] in (("symbol", "."), ("symbol", ":")):
                end += 2
            names.add(stream[end][1])
        elif kind == "symbol" and value == "=" and index and index + 1 < len(stream) and stream[index + 1][:2] == ("name", "function"):
            names.add(stream[index - 1][1])
    return names


def entrypoints(files, hero, pack_selection):
    def override(stem):
        return next((name for name in (f"bots/{stem}_{hero}.lua", f"bots/{stem}_generic.lua") if name in files), None)
    takeover = override("bot")
    entries = [name for name in (("bots/hero_selection.lua", "bots/team_desires.lua") if pack_selection else ("bots/team_desires.lua",)) if name in files]
    entries += [name for name in (takeover, override("ability_item_usage"), override("item_purchase")) if name]
    mode_files = {} if takeover else {mode: override("mode_" + mode) for mode in MODES}
    entries += [name for name in mode_files.values() if name]
    return entries, mode_files


def module_closure(files, entries, hero, analysis):
    reachable, unresolved = {}, []
    pending = list(entries)
    while pending:
        name = pending.pop()
        if name in reachable:
            continue
        stream = tokens(files[name], analysis)
        reachable[name] = stream
        for index, (kind, value, line) in enumerate(stream):
            if (kind, value) == ("name", "_G") and index + 1 < len(stream) and stream[index + 1][:2] == ("symbol", "["):
                unresolved.append({"file": name, "line": line, "module": None, "reason": "dynamic environment lookup"})
            if value not in ("require", "dofile", "load", "loadstring") or kind not in ("name", "string"):
                continue
            spend(analysis, "imports", 1)
            expression = argument(stream, index + 1)
            if kind != "name" or not expression or value in ("load", "loadstring"):
                unresolved.append({"file": name, "line": line, "module": None, "reason": "indirect loader or dynamic code"})
                continue
            spend(analysis, "work", len(expression) ** 2)
            dependency = module_name(expression, hero)
            if dependency in files:
                pending.append(dependency)
            else:
                unresolved.append({"file": name, "line": line, "module": dependency, "reason": "source absent" if dependency else "dynamic module expression"})
    return reachable, unresolved


def call_inventory(reachable):
    calls, dialect = defaultdict(list), defaultdict(list)
    for name, stream in sorted(reachable.items()):
        for index, (kind, value, line) in enumerate(stream):
            location = f"{name}:{line}"
            previous = stream[index - 1][1] if index and stream[index - 1][0] in ("name", "symbol") else ""
            following = stream[index + 1][:2] if index + 1 < len(stream) else None
            if (kind, value) in (("name", "goto"), ("symbol", "::")):
                dialect[value].append(location)
            if kind != "name" or following != ("symbol", "(") or previous == "function":
                continue
            root = index
            while root >= 2 and stream[root - 1][:2] in (("symbol", "."), ("symbol", ":")) and stream[root - 2][0] == "name":
                root -= 2
            if root and stream[root - 1][:2] == ("name", "function"):
                continue
            receiver = stream[root][1] if root != index else ""
            if previous == "." and receiver in ("bit", "jit", "ffi", "os", "io", "debug", "math", "table", "string", "coroutine"):
                dialect[receiver + "." + value].append(location)
            elif value in ("require", "dofile", "load", "loadstring", "module", "getfenv", "setfenv", "pcall", "xpcall", "collectgarbage"):
                dialect[value].append(location)
            elif value[:1].isupper() and value not in HOOKS:
                category = "member" if previous in (":", ".") else "global"
                calls[(category, value)].append(location)
    return calls, dialect


def classify_calls(calls, catalogue, reference):
    used = {category: {} for category in ("global", "member")}
    missing = {category: {} for category in ("global", "member")}
    unclassified = {category: {} for category in ("global", "member")}
    for (category, name), locations in sorted(calls.items()):
        supported = catalogue.get("global", set()) if category == "global" else catalogue.get("unit", set()) | catalogue.get("ability", set())
        known = supported | ((reference.get("global", set()) if category == "global" else reference.get("unit", set()) | reference.get("ability", set())) if reference is not None else set())
        if reference is not None and name not in known:
            unclassified[category][name] = sorted(set(locations))
            continue
        used[category][name] = sorted(set(locations))
        if name not in supported:
            missing[category][name] = sorted(set(locations))
    return used, missing, unclassified


def native_mode_dependencies(mode_files, reachable):
    default_hooks = {}
    for mode, name in mode_files.items():
        declared = declarations(reachable[name]) if name else set()
        absent = [hook for hook in ("GetDesire", "Think") if hook not in declared]
        if name and any(left[:2] == ("name", "return") and right[:2] == ("name", "nil") for left, right in zip(reachable[name], reachable[name][1:])) and "GetDesire" not in absent:
            absent.append("GetDesire")
        if absent:
            default_hooks[mode] = sorted(absent)
    return default_hooks


def dialect_inventory(dialect, catalogue):
    language = {}
    for name, locations in sorted(dialect.items()):
        namespace, member = name.split(".", 1) if "." in name else ("syntax" if name in ("goto", "::") else "global", name)
        language[name] = {"sites": sorted(set(locations)), "supported": member in catalogue.get(namespace, set())}
    return language


def scan(root, catalogue, hero="nevermore", runtime=None, pack_selection=False, reference=None):
    if not re.fullmatch(r"[a-z][a-z0-9_]{0,63}", hero):
        raise ValueError("hero must be a short ASCII key")
    storage = {"bytes": MAX_TOTAL, "entries": MAX_ENTRIES, "modules": MAX_MODULES}
    analysis = {"tokens": MAX_TOKENS, "work": MAX_ANALYSIS_WORK, "imports": 8192}
    files = {"bots/" + name: source for name, source in sources(root, storage).items()}
    if runtime:
        files.update({"game/" + name: source for name, source in sources(runtime, storage).items()})
    entries, mode_files = entrypoints(files, hero, pack_selection)
    reachable, unresolved = module_closure(files, entries, hero, analysis)
    calls, dialect = call_inventory(reachable)
    used, missing, unclassified = classify_calls(calls, catalogue, reference)
    total = sum(map(len, used.values()))
    gaps = sum(map(len, missing.values()))
    language = dialect_inventory(dialect, catalogue)
    return {
        "hero": hero,
        "scope": "conservative named-import closure; shared branches included; dynamic dispatch is not proven",
        "selection": "pack" if pack_selection else "preselected",
        "files": sorted(reachable),
        "entrypoints": sorted(entries),
        "imports_resolved": not unresolved,
        "unresolved_dependencies": unresolved,
        "used_globals": used["global"],
        "used_members": used["member"],
        "missing_globals": missing["global"],
        "missing_members": missing["member"],
        "unclassified_calls": unclassified,
        "api_coverage_percent": round(100 * (total - gaps) / total, 2) if total else None,
        "api_names": total,
        "default_hooks": native_mode_dependencies(mode_files, reachable),
        "dialect": language,
        "missing_dialect": sorted(name for name, data in language.items() if not data["supported"]),
        "coverage_note": "named API presence only; unclassified calls, dynamic dispatch, unresolved imports and native fallback behavior need separate conformance",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pack", type=Path)
    parser.add_argument("--catalogue", required=True, type=Path, help="TSV emitted by bota_botscript --api-catalogue")
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--api-reference", type=Path, default=Path(__file__).with_name("bot_api_reference.tsv"), help="engine function-name reference; unknown calls remain unclassified")
    parser.add_argument("--hero", default="nevermore")
    parser.add_argument("--pack-selection", action="store_true", help="include selection hooks; default assumes a preselected hero")
    options = parser.parse_args()
    try:
        catalogue = read_catalogue(options.catalogue)
        reference = read_catalogue(options.api_reference)
        report = scan(options.pack, catalogue, options.hero, options.runtime, options.pack_selection, reference)
    except (ValueError, OSError, UnicodeError) as error:
        parser.exit(2, f"census refused input: {error}\n")
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
