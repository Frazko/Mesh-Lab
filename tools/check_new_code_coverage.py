#!/usr/bin/env python3
"""Fail a change when its new executable Rust/Dart lines cover under 90%."""

from __future__ import annotations

import argparse
import re
import subprocess
from collections import defaultdict
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MINIMUM_PERCENT = 90.0
HUNK = re.compile(r"@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


def normalized(path: str, flutter_root: Path | None = None) -> str:
    raw = path.removeprefix("./")
    if raw.startswith("lib/"):
        # Flutter reports product paths relative to the package that generated
        # the LCOV file. The app remains the compatibility default for local
        # callers and single-report projects.
        if flutter_root is not None:
            try:
                return f"{flutter_root.resolve().relative_to(ROOT).as_posix()}/{raw}"
            except ValueError:
                pass
        return f"app/{raw}"
    candidate = Path(path)
    try:
        return candidate.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return candidate.as_posix().removeprefix("./")


def source_path(path: str) -> bool:
    return (
        path.startswith("crates/") and "/src/" in path and path.endswith(".rs")
    ) or (
        (path.startswith("app/lib/") or path.startswith("packages/"))
        and "/lib/" in path
        and path.endswith(".dart")
    )


def changed_lines(base: str) -> dict[str, set[int]]:
    output = subprocess.check_output(
        ["git", "diff", "--find-renames", "--unified=0", f"{base}...HEAD"],
        cwd=ROOT,
        text=True,
    )
    result: dict[str, set[int]] = defaultdict(set)
    path: str | None = None
    next_line: int | None = None
    for line in output.splitlines():
        if line.startswith("+++ "):
            raw = line[4:]
            path = None if raw == "/dev/null" else raw.removeprefix("b/")
            next_line = None
        elif match := HUNK.match(line):
            next_line = int(match.group(1))
        elif path is not None and next_line is not None:
            if line.startswith("+") and not line.startswith("+++"):
                result[path].add(next_line)
                next_line += 1
            elif line.startswith("-") and not line.startswith("---"):
                continue
            else:
                next_line += 1
    return {path: lines for path, lines in result.items() if source_path(path)}


def lcov_lines(paths: list[Path]) -> dict[str, dict[int, int]]:
    result: dict[str, dict[int, int]] = defaultdict(dict)
    for path in paths:
        current: str | None = None
        flutter_root = path.parent.parent if path.parent.name == "coverage" else None
        for raw in path.read_text(encoding="utf-8").splitlines():
            if raw.startswith("SF:"):
                current = normalized(raw[3:], flutter_root)
            elif current and raw.startswith("DA:"):
                line, hits, *_ = raw[3:].split(",")
                result[current][int(line)] = int(hits)
            elif raw == "end_of_record":
                current = None
    return result


def report(changed: dict[str, set[int]], coverage: dict[str, dict[int, int]]) -> tuple[int, int]:
    covered = total = 0
    for path, lines in sorted(changed.items()):
        measured = coverage.get(path, {})
        relevant = sorted(lines.intersection(measured))
        if not relevant:
            continue
        file_covered = sum(measured[line] > 0 for line in relevant)
        covered += file_covered
        total += len(relevant)
        print(f"{path}: {file_covered}/{len(relevant)} new executable lines")
    return covered, total


def unmeasured_sources(
    changed: dict[str, set[int]], coverage: dict[str, dict[int, int]]
) -> list[str]:
    """Return changed product files for which the runner emitted no line data.

    A new source file which is never loaded by the test run has no ``DA``
    record at all.  Ignoring it would turn the 90% rule into a loophole.
    Lines without a DA record *inside* an otherwise measured file are fine:
    they are imports, declarations, or comments rather than executable lines.
    """
    return sorted(path for path in changed if not coverage.get(path))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, help="merge-base ref or SHA")
    parser.add_argument("--rust-lcov", type=Path, required=True)
    parser.add_argument(
        "--flutter-lcov",
        type=Path,
        required=True,
        action="append",
        help="Flutter LCOV report; repeat for every product package.",
    )
    args = parser.parse_args()
    report_paths = [args.rust_lcov, *args.flutter_lcov]
    for report_path in report_paths:
        if not report_path.is_file():
            parser.error(f"missing coverage report: {report_path}")
    changed = changed_lines(args.base)
    coverage = lcov_lines(report_paths)
    unmeasured = unmeasured_sources(changed, coverage)
    if unmeasured:
        print("FAIL: coverage emitted no executable-line data for:")
        for path in unmeasured:
            print(f"  - {path}")
        return 1
    covered, total = report(changed, coverage)
    if total == 0:
        print("No new executable Rust or Dart lines require coverage.")
        return 0
    percent = 100 * covered / total
    print(f"New executable code coverage: {covered}/{total} ({percent:.2f}%)")
    if percent + 1e-9 < MINIMUM_PERCENT:
        print(f"FAIL: new executable code requires at least {MINIMUM_PERCENT:.0f}% coverage.")
        return 1
    print(f"PASS: new executable code meets the {MINIMUM_PERCENT:.0f}% policy.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
