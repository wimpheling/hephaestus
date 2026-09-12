#!/usr/bin/env python3
"""Create a stable digest of the host network topology.

The proc route tables also expose reference/use counters.  Those counters are
runtime accounting, so they are validated and deliberately omitted from the
canonical representation.  All topology and route attributes are retained.
"""

from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import re
import sys


MAX_INPUT_BYTES = 4 * 1024 * 1024
MAX_LINES = 65_536
MAX_INTERFACE_BYTES = 15  # Linux IFNAMSIZ - 1.
UINT32_MAX = 0xFFFF_FFFF
UINT16_MAX = 0xFFFF

_DECIMAL = re.compile(r"(?:0|[1-9][0-9]{0,9})\Z")
_HEX32 = re.compile(r"[0-9a-fA-F]{32}\Z")
_HEX8 = re.compile(r"[0-9a-fA-F]{8}\Z")
_HEX4 = re.compile(r"[0-9a-fA-F]{4}\Z")
_HEX2 = re.compile(r"[0-9a-fA-F]{2}\Z")
_IFACE = re.compile(r"[^\s/]{1,15}\Z")


class SnapshotError(ValueError):
    """The input did not have the expected bounded procfs shape."""


def _uint(value: str, *, maximum: int, field: str) -> int:
    if not _DECIMAL.fullmatch(value):
        raise SnapshotError(f"invalid {field}")
    parsed = int(value, 10)
    if parsed > maximum:
        raise SnapshotError(f"invalid {field}")
    return parsed


def _hex(value: str, *, pattern: re.Pattern[str], maximum: int, field: str) -> int:
    if not pattern.fullmatch(value):
        raise SnapshotError(f"invalid {field}")
    parsed = int(value, 16)
    if parsed > maximum:
        raise SnapshotError(f"invalid {field}")
    return parsed


def _iface(value: str) -> str:
    try:
        encoded = value.encode("ascii")
    except UnicodeEncodeError as error:
        raise SnapshotError("invalid interface name") from error
    if len(encoded) > MAX_INTERFACE_BYTES or not _IFACE.fullmatch(value):
        raise SnapshotError("invalid interface name")
    return value


def _bounded_read(path: Path) -> str:
    try:
        with path.open("rb") as stream:
            data = stream.read(MAX_INPUT_BYTES + 1)
    except OSError as error:
        raise SnapshotError("unable to read network source") from error
    if len(data) > MAX_INPUT_BYTES:
        raise SnapshotError("network source is too large")
    try:
        return data.decode("ascii")
    except UnicodeDecodeError as error:
        raise SnapshotError("network source is not ASCII") from error


def _lines(text: str) -> list[str]:
    lines = text.splitlines()
    if len(lines) > MAX_LINES:
        raise SnapshotError("too many network rows")
    return lines


def read_interfaces(directory: Path) -> tuple[str, ...]:
    try:
        entries = os.scandir(directory)
    except OSError as error:
        raise SnapshotError("unable to read interface directory") from error
    names: list[str] = []
    try:
        for entry in entries:
            names.append(_iface(entry.name))
            if len(names) > MAX_LINES:
                raise SnapshotError("too many interfaces")
    finally:
        entries.close()
    return tuple(sorted(names))


def parse_ipv4_routes(text: str) -> tuple[str, ...]:
    lines = _lines(text)
    if not lines or lines[0].split() != [
        "Iface",
        "Destination",
        "Gateway",
        "Flags",
        "RefCnt",
        "Use",
        "Metric",
        "Mask",
        "MTU",
        "Window",
        "IRTT",
    ]:
        raise SnapshotError("invalid IPv4 route header")

    canonical: list[str] = []
    for line in lines[1:]:
        if not line.strip():
            continue
        fields = line.split()
        if len(fields) != 11:
            raise SnapshotError("invalid IPv4 route row")
        interface = _iface(fields[0])
        destination = _hex(fields[1], pattern=_HEX8, maximum=UINT32_MAX, field="IPv4 destination")
        gateway = _hex(fields[2], pattern=_HEX8, maximum=UINT32_MAX, field="IPv4 gateway")
        flags = _hex(fields[3], pattern=_HEX4, maximum=UINT16_MAX, field="IPv4 flags")
        # Linux's /proc/net/route format places volatile RefCnt and Use at
        # indices 4 and 5; the kernel proc documentation describes this file
        # as the routing table.  Validate these fields, but omit only them.
        _uint(fields[4], maximum=UINT32_MAX, field="IPv4 RefCnt")
        _uint(fields[5], maximum=UINT32_MAX, field="IPv4 Use")
        metric = _uint(fields[6], maximum=UINT32_MAX, field="IPv4 metric")
        mask = _hex(fields[7], pattern=_HEX8, maximum=UINT32_MAX, field="IPv4 mask")
        mtu = _uint(fields[8], maximum=UINT32_MAX, field="IPv4 MTU")
        window = _uint(fields[9], maximum=UINT32_MAX, field="IPv4 window")
        irtt = _uint(fields[10], maximum=UINT32_MAX, field="IPv4 IRTT")
        canonical.append(
            f"v4|{interface}|{destination:08x}|{gateway:08x}|{flags:04x}|"
            f"{metric}|{mask:08x}|{mtu}|{window}|{irtt}"
        )
    return tuple(sorted(canonical))


def parse_ipv6_routes(text: str) -> tuple[str, ...]:
    canonical: list[str] = []
    for line in _lines(text):
        if not line.strip():
            continue
        fields = line.split()
        if len(fields) != 10:
            raise SnapshotError("invalid IPv6 route row")
        destination = _hex(fields[0], pattern=_HEX32, maximum=(1 << 128) - 1, field="IPv6 destination")
        destination_prefix = _hex(fields[1], pattern=_HEX2, maximum=128, field="IPv6 destination prefix")
        source = _hex(fields[2], pattern=_HEX32, maximum=(1 << 128) - 1, field="IPv6 source")
        source_prefix = _hex(fields[3], pattern=_HEX2, maximum=128, field="IPv6 source prefix")
        gateway = _hex(fields[4], pattern=_HEX32, maximum=(1 << 128) - 1, field="IPv6 gateway")
        metric = _hex(fields[5], pattern=_HEX8, maximum=UINT32_MAX, field="IPv6 metric")
        # Linux's /proc/net/ipv6_route format places volatile reference/use at
        # indices 6 and 7 (after destination/source/gateway/metric).  Keep
        # every other route field in the canonical representation.
        _hex(fields[6], pattern=_HEX8, maximum=UINT32_MAX, field="IPv6 reference")
        _hex(fields[7], pattern=_HEX8, maximum=UINT32_MAX, field="IPv6 use")
        flags = _hex(fields[8], pattern=_HEX8, maximum=UINT32_MAX, field="IPv6 flags")
        interface = _iface(fields[9])
        canonical.append(
            f"v6|{destination:032x}|{destination_prefix}|{source:032x}|{source_prefix}|"
            f"{gateway:032x}|{metric}|{flags}|{interface}"
        )
    return tuple(sorted(canonical))


def canonical_bytes(
    interfaces: tuple[str, ...], ipv4_routes: tuple[str, ...], ipv6_routes: tuple[str, ...]
) -> bytes:
    rows = [*(f"if|{name}" for name in interfaces), *ipv4_routes, *ipv6_routes]
    return ("\n".join(rows) + "\n").encode("ascii")


def snapshot(*, interfaces_dir: Path, route_path: Path, ipv6_route_path: Path) -> str:
    data = canonical_bytes(
        read_interfaces(interfaces_dir),
        parse_ipv4_routes(_bounded_read(route_path)),
        parse_ipv6_routes(_bounded_read(ipv6_route_path)),
    )
    return hashlib.sha256(data).hexdigest()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--interfaces-dir", type=Path, default=Path("/sys/class/net"))
    parser.add_argument("--route", type=Path, default=Path("/proc/net/route"))
    parser.add_argument("--ipv6-route", type=Path, default=Path("/proc/net/ipv6_route"))
    arguments = parser.parse_args(argv)
    try:
        print(
            snapshot(
                interfaces_dir=arguments.interfaces_dir,
                route_path=arguments.route,
                ipv6_route_path=arguments.ipv6_route,
            )
        )
    except SnapshotError as error:
        print(f"canonical-network-snapshot: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
