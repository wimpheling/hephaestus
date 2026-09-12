"""Focused tests for canonical network topology snapshots."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location(
    "canonical_network_snapshot", ROOT / "canonical-network-snapshot.py"
)
SNAPSHOT = importlib.util.module_from_spec(SPEC)
assert SPEC is not None and SPEC.loader is not None
SPEC.loader.exec_module(SNAPSHOT)


IPV4_HEADER = "Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT"
IPV4_ROW = "eth0 00000000 0100000A 0003 0 0 100 00000000 1500 0 0"
IPV6_ROW = (
    "20010DB8000000000000000000000000 40 "
    "00000000000000000000000000000000 00 "
    "00000000000000000000000000000000 00000064 00000001 00000002 00000000 eth0"
)


def _snapshot(root: Path, ipv4: str, ipv6: str) -> str:
    (root / "route").write_text(f"{IPV4_HEADER}\n{ipv4}\n", encoding="ascii")
    (root / "ipv6_route").write_text(f"{ipv6}\n", encoding="ascii")
    return SNAPSHOT.snapshot(
        interfaces_dir=root / "net",
        route_path=root / "route",
        ipv6_route_path=root / "ipv6_route",
    )


class CanonicalNetworkSnapshotTests(unittest.TestCase):
    def _write(self, root: Path, ipv4: str = IPV4_ROW, ipv6: str = IPV6_ROW) -> None:
        (root / "net").mkdir()
        (root / "net" / "eth0").mkdir()
        (root / "route").write_text(f"{IPV4_HEADER}\n{ipv4}\n", encoding="ascii")
        (root / "ipv6_route").write_text(f"{ipv6}\n", encoding="ascii")

    def test_counter_only_changes_do_not_change_digest(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-network-") as raw:
            root = Path(raw)
            self._write(root)
            original = _snapshot(root, IPV4_ROW, IPV6_ROW)
            changed = IPV4_ROW.replace(" 0 0 100 ", " 17 9321 100 ").replace(
                " 0003 00000001 00000002 ", " 0003 0000ABCD 0000FFFF "
            )
            changed_v6 = IPV6_ROW.replace("00000001 00000002", "0000ABCD 0000FFFF")
            self.assertEqual(original, _snapshot(root, changed, changed_v6))

    def test_topology_fields_change_digest(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-network-") as raw:
            root = Path(raw)
            self._write(root)
            original = _snapshot(root, IPV4_ROW, IPV6_ROW)
            for changed_ipv4, changed_ipv6 in (
                (IPV4_ROW.replace("00000000 0100000A", "00000001 0100000A"), IPV6_ROW),
                (IPV4_ROW.replace("0100000A", "0200000A"), IPV6_ROW),
                (IPV4_ROW.replace(" 100 ", " 200 "), IPV6_ROW),
                (IPV4_ROW.replace(" 1500 ", " 1400 "), IPV6_ROW),
                (IPV4_ROW, IPV6_ROW.replace("eth0", "eth1")),
                (
                    IPV4_ROW,
                    IPV6_ROW.replace(
                        "00000000000000000000000000000000 00000064",
                        "00000000000000000000000000000001 00000064",
                    ),
                ),
                (IPV4_ROW, IPV6_ROW.replace("00000064", "00000065")),
            ):
                self.assertNotEqual(original, _snapshot(root, changed_ipv4, changed_ipv6))

    def test_actual_proc_inputs_parse_and_cli_returns_digest(self) -> None:
        digest = SNAPSHOT.snapshot(
            interfaces_dir=Path("/sys/class/net"),
            route_path=Path("/proc/net/route"),
            ipv6_route_path=Path("/proc/net/ipv6_route"),
        )
        self.assertRegex(digest, r"^[0-9a-f]{64}$")
        result = subprocess.run(
            [
                sys.executable,
                str(ROOT / "canonical-network-snapshot.py"),
                "--interfaces-dir",
                "/sys/class/net",
                "--route",
                "/proc/net/route",
                "--ipv6-route",
                "/proc/net/ipv6_route",
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.stdout.strip(), digest)

    def test_malformed_or_missing_inputs_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-network-") as raw:
            root = Path(raw)
            self._write(root)
            for invalid_ipv4, invalid_ipv6 in (
                ("eth0 bad 0100000A 0003 0 0 100 00000000 1500 0 0", IPV6_ROW),
                (IPV4_ROW, IPV6_ROW.replace(" 40 ", " 81 ")),
                (IPV4_ROW.replace(" 0 0 100 ", " -1 0 100 "), IPV6_ROW),
            ):
                with self.assertRaises(SNAPSHOT.SnapshotError):
                    _snapshot(root, invalid_ipv4, invalid_ipv6)
            (root / "ipv6_route").unlink()
            with self.assertRaises(SNAPSHOT.SnapshotError):
                SNAPSHOT.snapshot(
                    interfaces_dir=root / "net",
                    route_path=root / "route",
                    ipv6_route_path=root / "ipv6_route",
                )

    def test_non_ascii_interface_fails_as_snapshot_error(self) -> None:
        with tempfile.TemporaryDirectory(prefix="heph-network-") as raw:
            root = Path(raw)
            self._write(root)
            (root / "net" / "eth0").rename(root / "net" / "éth0")
            with self.assertRaises(SNAPSHOT.SnapshotError):
                _snapshot(root, IPV4_ROW, IPV6_ROW)


if __name__ == "__main__":
    unittest.main()
