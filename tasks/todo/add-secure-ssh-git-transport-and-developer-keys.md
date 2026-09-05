# Add secure SSH Git transport and developer SSH-key management

Owner: unassigned

## Outcome

Add an SSH transport for the existing Hephaestus Git service so developers can
clone, fetch, and push with registered SSH keys while retaining the same
repository authorization, Git capability semantics, audit trail, revocation
behavior, and safety limits as smart HTTP. The SSH service accepts Git
protocol requests only: it never grants a shell, command execution, port
forwarding, SFTP/SCP, or host access.

This is deliberately post-MVP work. It is deferred from the MVP-05 golden
cooking-agent journey and must not delay or expand that application fixture.

## Locked decisions

| Area | Decision |
| --- | --- |
| Scope | This is a developer Git transport. It builds on the MVP 01.1 smart-HTTP Git capability and PAT foundation; it does not replace HTTP, OIDC, PATs, or runtime Git capabilities. |
| Authentication | A registered public SSH key maps to one existing human principal. It is not an independent user, shared anonymous credential, or agent-runtime credential. |
| Authorization | Every SSH Git request performs the same live repository, operation, ref, path, transfer-limit, and revocation checks as the corresponding HTTP Git request. Key possession never bypasses authorization. |
| Protocol surface | The SSH listener permits only a strictly parsed Git upload-pack or receive-pack invocation for an authorized repository. It denies shell, arbitrary `exec`, subsystems, PTY allocation, agent/X11 forwarding, TCP forwarding, environment injection, SCP/SFTP, and Git protocol extensions that escape this policy. |
| Key material | Store public keys and safe metadata only. Private keys, passphrases, SSH-agent material, and plaintext credential substitutes are never accepted, stored, logged, or emitted. |
| Key ownership | One human may register multiple named keys. A key has a stable fingerprint, creation/last-used/revoked timestamps, optional expiry, and immutable audit identity. Duplicate active fingerprints are rejected. |
| Key lifecycle | Registration requires an authenticated human and proof of possession or another explicitly documented anti-takeover ceremony. List views expose only safe metadata. Rotation adds a new key before revoking the old key; revocation is immediate for new authentication and new Git operations. |
| Host identity | Publish a stable SSH host-key fingerprint through authenticated UI/documentation. Protect private host keys in the platform secret boundary, support planned rotation with an overlap/announcement period, and never silently regenerate a host key on ordinary restart. |
| Auditing | Allowed and denied SSH authentication and Git operations are audited against the same human principal and repository operation model as HTTP, with key fingerprint/key ID rather than a raw public-key blob where possible. |
| Network exposure | Bind and expose a separately configurable SSH Git origin. It is Git-only and must not reuse a general-purpose host SSH daemon or imply host-login authority. |

## Dependencies

- [`mvp-01.1-git-capabilities-and-developer-personal-access-tokens.md`](../done/mvp-01.1-git-capabilities-and-developer-personal-access-tokens.md): shared Git capability grammar, live enforcement, audit model, and developer credential foundation.
- Completion of the MVP-05 golden cooking-agent journey is not required. This
  task remains deferred from that journey and may be scheduled independently
  after its MVP 01.1 dependency is complete.

## Non-goals

- Providing SSH shell accounts, host administration, remote command execution,
  SCP, SFTP, port forwarding, or a replacement for the documented host-recovery
  SSH/Tailscale path.
- Replacing smart HTTP, OIDC browser sessions, PATs, or exact-run runtime Git
  capabilities.
- Issuing SSH keys to agent guests, passing user private keys into guests, or
  using a user SSH key as a runtime capability.
- Supporting unsigned/unbounded SSH certificates or external key authorities
  unless added in a separately reviewed task.
- Changing cooking, Telegram, or other MVP-05 application code.

## Implementation checklist

- [ ] **1. Specify the SSH Git protocol boundary**
  - [ ] Define the supported SSH user/authority model, clone URL format,
    listener/origin configuration, repository identifier grammar, and strict
    mapping from Git client request to existing Git service operations.
  - [ ] Define an allowlist parser for only `git-upload-pack` and
    `git-receive-pack`, including quoting, repository-name normalization,
    protocol-v2 behavior, malformed input, command length, and argument-count
    limits.
  - [ ] Define deterministic denials for interactive shell requests, arbitrary
    commands, subsystems, PTY, forwarding, environment variables, SCP/SFTP,
    unknown extensions, malformed repository paths, and ambiguous identity.
  - [ ] Document public SSH host-key fingerprints and first-use verification
    instructions without teaching users to bypass host-key warnings.

- [ ] **2. Implement developer SSH-key lifecycle**
  - [ ] Add persistence for a versioned authorized public-key representation,
    normalized algorithm/key bytes, SHA-256 fingerprint, human-principal
    ownership, display label, creation/last-used/expiry/revoked timestamps,
    and safe audit metadata.
  - [ ] Accept only explicitly supported modern public-key algorithms and
    conservative size/format limits; reject private keys, certificates,
    duplicate fingerprints, malformed encodings, deprecated algorithms, and
    keys belonging to another active principal.
  - [ ] Provide authenticated UI/API flows to register a key, list safe
    metadata and fingerprints, rename a key, set supported expiry, rotate, and
    revoke. Never display a private value or persist unneeded raw key comments.
  - [ ] Design and implement a proof-of-possession or equally strong
    authenticated registration ceremony that prevents a user from registering
    an inaccessible or attacker-substituted public key unnoticed.
  - [ ] Make revocation and expiry take effect before authentication and before
    every new Git service authorization; record last use and redact key data in
    audit/UI output.

- [ ] **3. Implement a dedicated Git-only SSH listener**
  - [ ] Run an embedded or dedicated SSH Git service with a separately managed
    host identity and no shell-capable host account mapping.
  - [ ] Configure and protect persistent host private keys, publish current
    fingerprints, and implement documented planned host-key rotation with a
    grace/overlap mechanism where protocol support permits it.
  - [ ] Authenticate only registered, active keys and resolve each to the same
    human principal used by HTTP. Do not treat an SSH username as an
    authorization identity.
  - [ ] Reject all non-Git SSH channel/session features before they reach a
    subprocess, shell, filesystem path resolver, or host service.
  - [ ] Route accepted Git requests through the shared Git authorization and
    receive/fetch enforcement path rather than duplicating or weakening the
    capability grammar.
  - [ ] Bound concurrent connections, authentication attempts, request size,
    Git transfer/storage/quarantine work, and failure diagnostics to resist
    resource exhaustion and information disclosure.

- [ ] **4. Preserve shared authorization and audit semantics**
  - [ ] Apply live human-principal authorization and the MVP 01.1 repository,
    operation, ref, changed-path, force-update, creation/deletion, expiry, and
    transfer-limit checks to every SSH clone, fetch, and receive request.
  - [ ] Ensure an SSH key cannot act after user/project membership removal,
    repository restriction change, key revocation, key expiry, or other live
    authority loss, including during connection reuse as applicable.
  - [ ] Produce auditable allowed and denied records with human principal,
    key ID/fingerprint, repository, operation, result, reason class, and safe
    request metadata; avoid raw public-key blobs, Git object contents, tokens,
    or sensitive paths where existing audit policy forbids them.
  - [ ] Keep HTTP and SSH user-visible repository/ref behavior equivalent; any
    protocol-required difference must be documented and covered by fixtures.

- [ ] **5. Add Git-client UX and operational documentation**
  - [ ] Show the SSH clone URL and verified host-key fingerprint beside the
    existing HTTP clone command, including a copyable command with an
    application-owned SSH user if required by the protocol.
  - [ ] Document key generation, registration, fingerprint verification,
    `known_hosts` handling, multiple-key selection, rotation, revocation,
    expiry, troubleshooting, and the distinction between Git SSH and host SSH.
  - [ ] Document listener binding/origin configuration, host-key backup and
    restore, planned rotation, incident response for host-key compromise, and
    health/status signals without exposing secret material.

- [ ] **6. Test security, compatibility, and revocation**
  - [ ] Add unit/property tests for key parsing/normalization, supported
    algorithms, fingerprint uniqueness, expiry, registration proof,
    authorization mapping, command parsing, repository normalization, and
    audit redaction.
  - [ ] Add real SSH Git integration tests for authenticated clone, fetch, and
    push using a disposable developer key, with results equivalent to the
    corresponding smart-HTTP fixtures.
  - [ ] Add allowed and denied tests for repository/membership scope, ref/path
    scopes, protected updates, branch/tag creation/deletion, force pushes,
    object-transfer limits, malformed commands, and unauthenticated/unknown
    keys.
  - [ ] Add explicit tests that shell, arbitrary exec, SCP/SFTP, subsystems,
    PTY, agent/X11/TCP forwarding, and environment injection are rejected.
  - [ ] Add lifecycle tests for initial registration proof, duplicate key,
    key rotation, key expiry, immediate revocation, principal/membership
    removal, reconnect, and in-flight/new-operation behavior.
  - [ ] Add host-key persistence, fingerprint publication, restore, planned
    rotation, and compromise-recovery fixtures; prove an ordinary restart does
    not change the advertised host identity.

- [ ] **7. Verify and document**
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev quality`.
  - [ ] Run `git diff --check`.

## Completion evidence

Record the SSH Git protocol specification version; supported public-key
algorithms; host-key fingerprints and rotation procedure; registration and
revocation evidence; allowed and denied real-client clone/fetch/push fixtures;
explicit Git-only channel-denial results; HTTP/SSH authorization-equivalence
fixtures; lifecycle and live-revocation results; audit samples proving human
principal attribution and redaction; and the verification commands/results.

Before moving this task to `tasks/done/`, link any deliberately deferred
certificate-authority, hardware-key, or non-Git SSH follow-up work as separate
todo tasks.
