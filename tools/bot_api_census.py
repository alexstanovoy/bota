#!/usr/bin/env python3
"""Bounded source-only API inventory for a hero's script entry/module closure.

The catalogue comes from `bota_botscript --api-catalogue`, not a copied API list.
This is a conservative static closure: branches in shared helpers are included.
Unresolved imports are explicit and prevent a completeness claim. Presence coverage
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
MODES = "laning attack roam retreat secret_shop side_shop rune push_tower_top push_tower_mid push_tower_bot defend_tower_top defend_tower_mid defend_tower_bot assemble team_roam farm defend_ally evasive_maneuvers roshan item ward outpost".split()
HOOKS = {"Think", "GetDesire", "OnStart", "OnEnd", "MinionThink", "ItemPurchaseThink", "AbilityUsageThink", "ItemUsageThink", "CourierUsageThink", "BuybackUsageThink", "AbilityLevelUpThink", "TeamThink", "UpdatePushLaneDesires", "UpdateDefendLaneDesires", "UpdateFarmLaneDesires", "UpdateRoamDesire", "UpdateRoshanDesire", "UpdateLaneAssignments", "GetBotNames"}
LONG = re.compile(r"\[(=*)\[")
IDENT = re.compile(r"[A-Za-z_][A-Za-z_0-9]*")


def tokens(source):
    """Linear lexer; comments never produce calls, string contents stay opaque."""
    result, index, line = [], 0, 1
    while index < len(source):
        start = index
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
            quote, value = source[index], []
            index += 1
            while index < len(source) and source[index] != quote:
                if source[index] == "\\" and index + 1 < len(source):
                    index += 1
                value.append(source[index])
                index += 1
            index = min(index + 1, len(source))
            result.append(("string", "".join(value), line))
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
    return result


def sources(root):
    """Pin directories and classify files before reading; never follow pack links."""
    result, total, entries = {}, 0, 0
    def walk(directory, prefix, depth):
        nonlocal total, entries
        if depth > 16:
            raise ValueError("directory depth limit")
        with os.scandir(directory) as listing:
            for entry in listing:
                entries += 1
                if entries > MAX_ENTRIES:
                    raise ValueError("directory entry limit")
                name = prefix + entry.name
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
                        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
                            raise ValueError(f"nonregular or linked source: {name!r}")
                        if info.st_size > MAX_SOURCE or total + info.st_size > MAX_TOTAL:
                            raise ValueError(f"source size limit: {name!r}")
                        if len(result) >= MAX_MODULES:
                            raise ValueError("module count limit")
                        with open(f"/proc/self/fd/{pinned}", "rb") as source:
                            data = source.read(MAX_SOURCE + 1)
                        total += len(data)
                        if len(data) > MAX_SOURCE or total > MAX_TOTAL:
                            raise ValueError(f"source size limit: {name!r}")
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
    if stream[index][0] == "string":
        return [stream[index]]
    if stream[index][1] != "(":
        return []
    depth, end = 1, index + 1
    while end < len(stream):
        value = stream[end][1]
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
    if not text or any(kind != "string" if index % 2 == 0 else value != ".." for index, (kind, value) in enumerate(text)):
        return None
    name = "".join(value for kind, value in text if kind == "string").replace("\\", "/")
    if name.endswith(".lua"):
        name = name[:-4]
    if name.startswith("bots/"):
        name = name[5:]
    elif "/" not in name:
        name = name.replace(".", "/")
    if name.startswith("/") or any(part in ("", ".", "..") for part in name.split("/")):
        return None
    return name + ".lua"


def declarations(stream):
    names = set()
    for index, (_, value, _) in enumerate(stream):
        if value == "function" and index + 1 < len(stream) and stream[index + 1][0] == "name":
            end = index + 1
            while end + 2 < len(stream) and stream[end + 1][1] in (".", ":"):
                end += 2
            names.add(stream[end][1])
        elif value == "=" and index and index + 1 < len(stream) and stream[index + 1][1] == "function":
            names.add(stream[index - 1][1])
    return names


def local_bindings(stream):
    names = set()
    for index, (_, value, _) in enumerate(stream):
        if value != "local" or index + 1 >= len(stream):
            continue
        index += 1
        if stream[index][1] == "function":
            index += 1
        while index < len(stream) and stream[index][0] == "name":
            names.add(stream[index][1])
            index += 1
            if index >= len(stream) or stream[index][1] != ",":
                break
            index += 1
    return names


def scan(root, catalogue, hero="nevermore", runtime=None, pack_selection=False):
    files = sources(root)
    if runtime:
        files.update({"game/" + name: source for name, source in sources(runtime).items()})
    def override(stem):
        return next((name for name in (f"{stem}_{hero}.lua", f"{stem}_generic.lua") if name in files), None)
    takeover = override("bot")
    entries = [name for name in (("hero_selection.lua", "team_desires.lua") if pack_selection else ("team_desires.lua",)) if name in files]
    entries += [name for name in (takeover, override("ability_item_usage"), override("item_purchase")) if name]
    mode_files = {} if takeover else {mode: override("mode_" + mode) for mode in MODES}
    entries += [name for name in mode_files.values() if name]
    reachable, unresolved, module_variables = {}, [], set()
    pending = list(entries)
    while pending:
        name = pending.pop()
        if name in reachable:
            continue
        stream = tokens(files[name])
        reachable[name] = stream
        for index, (kind, value, line) in enumerate(stream):
            if kind != "name" or value not in ("require", "dofile"):
                continue
            expression = argument(stream, index + 1)
            if not expression:
                continue
            if index >= 2 and stream[index - 1][1] == "=":
                module_variables.add(stream[index - 2][1])
            dependency = module_name(expression, hero)
            if dependency in files:
                pending.append(dependency)
            else:
                unresolved.append({"file": name, "line": line, "module": dependency, "reason": "source absent" if dependency else "dynamic module expression"})
    defined = set().union(HOOKS, *(declarations(stream) for stream in reachable.values()))
    calls, dialect = defaultdict(list), defaultdict(list)
    for name, stream in sorted(reachable.items()):
        bound = local_bindings(stream)
        for index, (kind, value, line) in enumerate(stream):
            location = f"{name}:{line}"
            previous = stream[index - 1][1] if index else ""
            following = stream[index + 1][1] if index + 1 < len(stream) else ""
            if value in ("goto", "::"):
                dialect[value].append(location)
            if kind != "name" or following != "(" or previous == "function":
                continue
            receiver = stream[index - 2][1] if index >= 2 else ""
            if previous == "." and receiver in ("bit", "jit", "ffi", "os", "io", "debug", "math", "table", "string", "coroutine"):
                dialect[receiver + "." + value].append(location)
            elif value in ("require", "dofile", "load", "loadstring", "module", "getfenv", "setfenv", "pcall", "xpcall", "collectgarbage"):
                dialect[value].append(location)
            elif value[:1].isupper() and value not in defined and receiver not in module_variables and not (previous not in (":", ".") and value in bound):
                category = "member" if previous in (":", ".") else "global"
                calls[(category, value)].append(location)
    used = {category: {} for category in ("global", "member")}
    missing = {category: {} for category in ("global", "member")}
    for (category, name), locations in sorted(calls.items()):
        used[category][name] = sorted(set(locations))
        supported = catalogue.get("global", set()) if category == "global" else catalogue.get("unit", set()) | catalogue.get("ability", set())
        if name not in supported:
            missing[category][name] = sorted(set(locations))
    default_hooks = {}
    for mode, name in mode_files.items():
        declared = declarations(reachable[name]) if name else set()
        absent = [hook for hook in ("GetDesire", "Think") if hook not in declared]
        if name and any(left[1] == "return" and right[1] == "nil" for left, right in zip(reachable[name], reachable[name][1:])) and "GetDesire" not in absent:
            absent.append("GetDesire")
        if absent:
            default_hooks[mode] = sorted(absent)
    total = sum(map(len, used.values()))
    gaps = sum(map(len, missing.values()))
    language = {}
    for name, locations in sorted(dialect.items()):
        namespace, member = name.split(".", 1) if "." in name else ("syntax" if name in ("goto", "::") else "global", name)
        language[name] = {"sites": sorted(set(locations)), "supported": member in catalogue.get(namespace, set())}
    return {"hero": hero, "scope": "conservative entry/module closure; shared branches included", "selection": "pack" if pack_selection else "preselected", "files": sorted(reachable), "entrypoints": sorted(entries), "closure_complete": not unresolved, "unresolved_dependencies": unresolved, "used_globals": used["global"], "used_members": used["member"], "missing_globals": missing["global"], "missing_members": missing["member"], "api_coverage_percent": round(100 * (total - gaps) / total, 2) if total else 100.0, "api_names": total, "default_hooks": default_hooks, "dialect": language, "missing_dialect": sorted(name for name, data in language.items() if not data["supported"]), "coverage_note": "presence only; unresolved imports and native fallback behavior are not counted as covered"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pack", type=Path)
    parser.add_argument("--catalogue", required=True, type=Path, help="TSV emitted by bota_botscript --api-catalogue")
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--hero", default="nevermore")
    parser.add_argument("--pack-selection", action="store_true", help="include selection hooks; default assumes a preselected hero")
    options = parser.parse_args()
    catalogue = defaultdict(set)
    with options.catalogue.open() as source:
        text = source.read(MAX_SOURCE + 1)
        if len(text) > MAX_SOURCE:
            parser.error("catalogue size limit")
        for line in text.splitlines():
            namespace, name = line.rstrip("\n").split("\t")
            catalogue[namespace].add(name)
    try:
        report = scan(options.pack, catalogue, options.hero, options.runtime, options.pack_selection)
    except (ValueError, OSError) as error:
        parser.exit(2, f"census refused input: {error}\n")
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
