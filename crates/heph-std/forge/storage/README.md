# Purpose

`forge-storage` owns the canonical bare-Git filesystem used by Forge. It maps
typed repository IDs to private bare repositories, creates and validates their
layout, and reads exact files from exact commits for configuration and build
inspection.

# Responsibilities

`GitStorage::initialize` validates the storage root, while
`create_bare`, `validate_existing`, and `delete_bare` manage repository
lifecycle. Route parsing accepts a repository ID and `repository_path` derives
the canonical location, keeping callers from constructing arbitrary storage
paths.

`read_file_at_commit` resolves a validated relative path from a complete commit
and returns its bounded contents. Path and object validation, canonicalization,
symlink handling, and typed filesystem errors protect receive inspection and
build preparation from crossing the repository boundary.

# When

Initialize `GitStorage` during Forge startup, create or validate a repository
when its metadata is provisioned, and use `read_file_at_commit` when inspecting
the exact source revision associated with a receive or build request.
