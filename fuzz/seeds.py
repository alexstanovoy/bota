#!/usr/bin/env python3
"""Seeds the corpora of the fuzz targets from the start of a recorded match.

    python3 -I fuzz/seeds.py <replay.brp>

Reads only the first records of the replay. Writes corpus/<target>/seed-*.
"""
import os
import struct
import sys

RECORDS = 400
HERE = os.path.dirname(os.path.abspath(__file__))


def records(path):
    with open(path, "rb") as replay:
        for _ in range(RECORDS):
            prefix = replay.read(4)
            if len(prefix) < 4:
                return
            (length,) = struct.unpack("<I", prefix)
            yield replay.read(length)


def frames(payloads):
    return b"".join(struct.pack("<H", len(p)) + p for p in payloads if len(p) <= 0xFFFF)


def write(target, name, data):
    folder = os.path.join(HERE, "corpus", target)
    os.makedirs(folder, exist_ok=True)
    with open(os.path.join(folder, "seed-" + name), "wb") as out:
        out.write(data)


def main(path):
    payloads = list(records(path))
    messages = [p[1:] for p in payloads if p[0] == 0]
    orders = [p for p in payloads if p[0] == 1]
    kinds = {}
    for message in messages:
        kinds.setdefault(message[0], message)
    for kind, message in kinds.items():
        write("proto_decode", f"server-{kind}", message)
    for record in payloads[:8]:
        write("proto_decode", f"record-{record[0]}-{len(record)}", record)
    for count in (2, 4, 8, 16):
        write("client_stream", f"spectator-{count}", b"\x00" + frames(messages[:count]))
        write("client_stream", f"seat-{count}", b"\x80" + frames(messages[:count]))
        prefix = b"".join(struct.pack("<I", len(p)) + p for p in payloads[: 2 * count])
        write("client_replay", f"start-{count}", prefix)
    for start in range(0, len(orders), 64):
        chunk = frames(orders[start : start + 64])
        for knobs in (0x00, 0x02, 0x45):
            write("server_orders", f"{start}-{knobs:02x}", bytes([knobs]) + chunk)


if __name__ == "__main__":
    main(sys.argv[1])
