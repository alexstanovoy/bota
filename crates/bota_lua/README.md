# bota_lua

Lua bot API bindings over filtered `ClientSession` observations. Language execution,
collection, module-name validation and interpreter limits are provided by `luagee`.
The binding issues ordinary `IssuedOrder`s; it changes no simulation rules.

One `LuaTeam` owns a team's state. Each override has its own environment, while
`require` shares the team's module cache. Takeovers run every tick; otherwise modes
arbitrate every nine ticks with transition hooks. Team, ability/item, purchase and
courier hooks use the same scheduler. A failed bot turn discards that bot's orders.
Repeated fuel diagnostics are eligible once per thirty ticks.

Defaults are 500,000 fuel per call, 2,000,000 per team tick and 256 MiB memory.
Host storage reserves 16 MiB against that cap, including bounded snapshot replacement
and handle maps. Snapshots hold at most 4 MiB of copied facts; unit and ability caches
hold at most 8,192 entries each; each bot returns at most 64 orders per turn.
Module banks validate names, module count and source sizes before copying, and
reserve storage for module-loading copies. Roots and userdata retention stay charged
through collection. All numeric arguments crossing into protocol coordinates are
finite; coordinate conversion uses nearest-even Q16.16 rounding and a 9,216 offset.

The default crate uses no filesystem or threads. Embedders of `LuaTeam`, including
Wasm hosts, must supply an execution deadline boundary. Feature `native` supplies
`NativeTeam`: a 16 MiB worker stack, one outstanding request, a 33 ms think deadline,
and retirement after three consecutive missed ticks or a host panic.
Late replies are discarded before submitting another snapshot; a timely reply
resets the miss counter. The next request acknowledges whether the previous
turn was accepted; discarded host continuations, including attack-once completion,
are restored before computing that request. Lua script globals are not rewound.
Native defaults are 125,000 fuel per call and 600,000
per team tick, leaving margin inside the deadline under load. Native interpreter limits may
only reduce the defaults. Active and retiring workers share a process-wide limit
of 32; cleanup does not wait on timed-out work.

Native directory preloading requires Linux `openat2` and procfs. It pins objects
before classification, rejects symlinks/submounts/nonregular files and reads sources
through pinned descriptors. Tree depth, entry count and source bytes are bounded.
Setup I/O has no latency guarantee. Thinks load exclusively from the completed bank.
`bots/` and bare module names address pack sources; `game/` addresses the optional
runtime-source root. Diagnostics use logical module names.

Run `bota_botscript` with `--features native -- --scripts DIR --team radiant`.
Options include `--runtime-scripts DIR`, `--address HOST:PORT`, `--hero KEY` and
`--think-ms N`. It connects directly using `bota_client_core`. `BOTA_SEATS` accepts
server-style `key_hex:team:name[:kind]` reservations; only the selected team's bot
seats are used. Without credentials it joins one unreserved seat. Keys never belong
on the command line.

The normal test group runs original small scripts through complete matches.
It reports local-source conformance separately as not run when no pack is configured;
setting `BOTA_LUA_LOCAL_PACK` also enables that contract in the group.
`tools/test_lua_local.sh` is explicitly optional: set `BOTA_LUA_LOCAL_PACK` and,
when required, `BOTA_LUA_RUNTIME_SCRIPTS`. It runs the ignored local-source match
without copying source into the repository. `fuzz/fuzz_targets/lua_binding.rs`
drives Lua source, hostile arguments, collection modes and varied game states.
