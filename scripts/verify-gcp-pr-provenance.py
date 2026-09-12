#!/usr/bin/env python3
"""Validate the bounded GitHub API response for a same-repository PR run."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import NoReturn


REPOSITORY = "wimpheling/hephaestus"
REPOSITORY_ID = "1312377552"
SHA_RE = re.compile(r"[0-9a-f]{40}\Z")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--response-file", type=Path, required=True)
    parser.add_argument("--pull-number", required=True)
    parser.add_argument("--repository-id", required=True)
    parser.add_argument("--head-sha", required=True)
    return parser.parse_args()


def fail(message: str) -> NoReturn:
    raise SystemExit(f"PR provenance rejected: {message}")


def main() -> int:
    args = parse_args()
    if not re.fullmatch(r"[1-9][0-9]{0,8}", args.pull_number):
        fail("pull number is invalid")
    if args.repository_id != REPOSITORY_ID:
        fail("repository ID is not the reviewed repository")
    if SHA_RE.fullmatch(args.head_sha) is None:
        fail("head SHA is invalid")
    try:
        value = json.loads(args.response_file.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"GitHub API response is invalid: {error}")
    if not isinstance(value, dict):
        fail("GitHub API response is not an object")
    head = value.get("head")
    base = value.get("base")
    if not isinstance(head, dict) or not isinstance(base, dict):
        fail("PR head or base is missing")
    head_repo = head.get("repo")
    base_repo = base.get("repo")
    if not isinstance(head_repo, dict) or not isinstance(base_repo, dict):
        fail("PR head or base repository is missing")
    if head_repo.get("id") != int(REPOSITORY_ID) or base_repo.get("id") != int(REPOSITORY_ID):
        fail("PR is not based on the reviewed repository")
    if head_repo.get("full_name") != REPOSITORY or base_repo.get("full_name") != REPOSITORY:
        fail("PR repository name is not the reviewed repository")
    if head.get("sha") != args.head_sha:
        fail("submitted SHA does not equal the current PR head SHA")
    if value.get("number") != int(args.pull_number):
        fail("API response PR number does not match the submitted number")
    if value.get("state") != "open":
        fail("PR is not open")
    print(f"repository={REPOSITORY} repository_id={REPOSITORY_ID} pull_number={args.pull_number} head_sha={args.head_sha}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
