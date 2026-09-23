#!/usr/bin/env python3
"""Summarize filtered Rust LCOV line coverage by workspace crate."""

from __future__ import annotations

import argparse
import csv
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


EXCLUDED_SOURCE = re.compile(r"(?:^|/)crates/rpc-proto/src/generated/")
CRATE_SOURCE = re.compile(r"(?:^|/)crates/([^/]+)/")


class CoverageError(ValueError):
    """A malformed or unmappable coverage report."""


@dataclass
class LineCoverage:
    """Aggregated line coverage counters."""

    hit: int = 0
    total: int = 0

    def add(self, hit: int, total: int) -> None:
        """Add one LCOV record after validating its counters."""
        if hit < 0 or total < 0 or hit > total:
            raise CoverageError(f"invalid LCOV line counters: LH={hit}, LF={total}")
        self.hit += hit
        self.total += total


def parse_args() -> argparse.Namespace:
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lcov", type=Path, required=True, help="filtered LCOV input")
    parser.add_argument("--csv", type=Path, required=True, help="per-crate CSV output")
    parser.add_argument("--markdown", type=Path, required=True, help="per-crate Markdown output")
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="repository root containing the crates workspace",
    )
    return parser.parse_args()


def workspace_crates(repo_root: Path) -> list[str]:
    """Read package names from every root workspace crate manifest."""
    crates_root = repo_root / "crates"
    manifests = sorted(crates_root.glob("*/Cargo.toml"))
    if not manifests:
        raise CoverageError(f"no crate manifests found under {crates_root}")
    packages: list[str] = []
    for manifest in manifests:
        try:
            with manifest.open("rb") as handle:
                package = tomllib.load(handle).get("package", {})
        except (OSError, tomllib.TOMLDecodeError) as error:
            raise CoverageError(f"cannot read {manifest}: {error}") from error
        name = package.get("name")
        if not isinstance(name, str) or not name:
            raise CoverageError(f"{manifest} has no package.name")
        packages.append(name)
    if len(packages) != len(set(packages)):
        raise CoverageError("workspace package names are not unique")
    return packages


def source_crate(source: str, packages: set[str]) -> str:
    """Map an LCOV source path to its workspace crate package name."""
    normalized = source.replace("\\", "/")
    match = CRATE_SOURCE.search(normalized)
    if match is None:
        raise CoverageError(f"LCOV source is outside crates/*: {source}")
    crate = match.group(1)
    if crate not in packages:
        raise CoverageError(f"LCOV source names unknown workspace crate {crate!r}: {source}")
    return crate


def parse_lcov(path: Path, packages: set[str]) -> dict[str, LineCoverage]:
    """Parse LCOV LF/LH totals and require cargo-llvm-cov filtering."""
    if not path.is_file():
        raise CoverageError(f"LCOV file does not exist: {path}")
    coverage: dict[str, LineCoverage] = {}
    source: str | None = None
    line_total: int | None = None
    line_hit: int | None = None
    line_number = 0

    def finish_record() -> None:
        nonlocal source, line_total, line_hit
        if source is None:
            raise CoverageError(f"end_of_record without SF at {path}:{line_number}")
        if EXCLUDED_SOURCE.search(source.replace("\\", "/")):
            raise CoverageError(
                "LCOV still contains excluded rpc-proto generated source "
                f"{source}; pass --ignore-filename-regex to cargo llvm-cov"
            )
        if line_total is None or line_hit is None:
            raise CoverageError(f"incomplete LCOV record for {source}")
        crate = source_crate(source, packages)
        coverage.setdefault(crate, LineCoverage()).add(line_hit, line_total)
        source = None
        line_total = None
        line_hit = None

    try:
        with path.open(encoding="utf-8") as handle:
            for line_number, raw_line in enumerate(handle, start=1):
                line = raw_line.rstrip("\n")
                if line.startswith("SF:"):
                    if source is not None:
                        raise CoverageError(f"new SF before end_of_record at {path}:{line_number}")
                    source = line[3:]
                elif line.startswith("LF:"):
                    if source is None:
                        raise CoverageError(f"LF outside a record at {path}:{line_number}")
                    line_total = int(line[3:])
                elif line.startswith("LH:"):
                    if source is None:
                        raise CoverageError(f"LH outside a record at {path}:{line_number}")
                    line_hit = int(line[3:])
                elif line == "end_of_record":
                    finish_record()
    except OSError as error:
        raise CoverageError(f"cannot read {path}: {error}") from error
    except CoverageError:
        raise
    except ValueError as error:
        raise CoverageError(f"invalid integer in {path}:{line_number}: {error}") from error

    if source is not None:
        raise CoverageError(f"unterminated LCOV record for {source}")
    if not coverage:
        raise CoverageError(f"LCOV file has no workspace source records: {path}")
    return coverage


def percent(hit: int, total: int) -> str:
    """Format a line coverage percentage, or n/a when no lines exist."""
    return "n/a" if total == 0 else f"{100 * hit / total:.2f}%"


def write_csv(path: Path, packages: list[str], coverage: dict[str, LineCoverage]) -> None:
    """Write one row for every workspace crate."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(("crate", "lines_hit", "lines_total", "line_coverage"))
        for crate in packages:
            counters = coverage.get(crate, LineCoverage())
            writer.writerow(
                (crate, counters.hit, counters.total, percent(counters.hit, counters.total))
            )


def markdown(packages: list[str], coverage: dict[str, LineCoverage]) -> str:
    """Render the aggregate and per-crate table."""
    overall = LineCoverage()
    for counters in coverage.values():
        overall.add(counters.hit, counters.total)
    rows = [
        "## Rust line coverage by crate",
        "",
        f"Overall line coverage: **{percent(overall.hit, overall.total)}** "
        f"({overall.hit:,}/{overall.total:,} lines).",
        "",
        "| Crate | Lines hit | Lines total | Line coverage |",
        "| --- | ---: | ---: | ---: |",
    ]
    for crate in packages:
        counters = coverage.get(crate, LineCoverage())
        if counters.total == 0:
            rows.append(f"| `{crate}` | n/a | n/a | n/a |")
        else:
            rows.append(
                f"| `{crate}` | {counters.hit:,} | {counters.total:,} | "
                f"{percent(counters.hit, counters.total)} |"
            )
    return "\n".join(rows) + "\n"


def main() -> int:
    """Generate CSV and Markdown coverage summaries."""
    args = parse_args()
    try:
        packages = workspace_crates(args.repo_root.resolve())
        coverage = parse_lcov(args.lcov.resolve(), set(packages))
        write_csv(args.csv, packages, coverage)
        rendered = markdown(packages, coverage)
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(rendered, encoding="utf-8")
    except (CoverageError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    overall = LineCoverage()
    for counters in coverage.values():
        overall.add(counters.hit, counters.total)
    print(
        f"Overall line coverage: {percent(overall.hit, overall.total)} "
        f"({overall.hit:,}/{overall.total:,} lines)"
    )
    print(f"Wrote {args.csv} and {args.markdown}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
