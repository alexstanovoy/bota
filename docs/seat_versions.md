# Seat versions

A public seat identity contains its slot, main display name, bot flag, and optional opaque
`version`. A version is 1–32 printable ASCII bytes, with no leading or trailing whitespace.
Human identities have no version. The version is a separate field throughout transport and
recording; the seat name retains its existing byte budget.

A direct bot can use `ClientMsg::HelloVersion` or the shared client's
`join_on_with_version`. A direct human or spectator claiming a version is refused with
`VersionNotAllowed`; an invalid direct version is refused with `BadVersion`.

The launcher's `{seats_with_versions}` placeholder carries each authenticated version in a
separate tagged, hex-encoded reservation field. Use it in the server's `BOTA_SEATS`
environment variable. Reserved identities override a connecting client's name and version,
including an absent authoritative version for a human. Tickets remain private.

`ServerMsg::SeatIdentity` supplies the same public fact to live clients, late spectators,
and recordings through `ReplayRecord::Msg`. The metadata trailer also retains that fact as
`MetaRecord::SeatIdentity`. The server performs no display formatting.

The wire fingerprint changes with these additions and no earlier wire is admitted: clients and
servers of the wire before it are refused at the handshake. All existing variant ids and field
positions are preserved. The append-only registries in `crates/bota_proto/registry/wire.txt`
and `crates/bota_replay/registry/meta.txt` identify the added facts.
