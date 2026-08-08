# Runtime Git authority

Runtime Git is an exact-run control-plane capability. Released code declares a
maximum normalized Git policy; an immutable instance revision binds an equal or
narrower policy to one repository; dispatch copies that binding into the run's
authorization snapshot. A runtime cannot supply or widen its own repository,
refs, paths, transitions, limits, or expected parent.

After the generic runtime session and Git snapshot are durable, Hephaestus
issues a separate, non-renewable runtime Git credential. Only its
domain-separated verifier is stored. Plaintext is retained temporarily in an
encrypted host handoff envelope, delivered through the authenticated VM
bootstrap stream, persisted by trusted guest bootstrap in the mode-`0400`
runtime authority document, and removed from host handoff after the exact
session/generation acknowledgement. It is not a PAT or user identity and must
never appear in a remote URL, environment variable, Git configuration, command
line, log, or durable run record.

Git HTTP authenticates the credential against the exact repository and
discover, fetch, or receive operation. Authentication also requires an active,
unexpired generic session, its immutable Git snapshot, and current live grants.
Runtime ref advertisement and fetch hide refs outside the declared ref globs;
this is a ref-visibility boundary, not path-level read filtering. Anyone able
to fetch an allowed ref can read every object reachable from it.

For receive, the transport writes the resolved authority to an owner-only,
short-lived host context file and gives Git only its opaque path handle. The
host-installed pre-receive hook revalidates that context, Git's quarantine,
the canonical old refs, all ref commands, ancestry, rename endpoints,
merge-parent path unions, request/pack/object/update limits, transition rules,
and expiry before Git may atomically update canonical refs. Neither bearer
material nor serialized scope is placed in the hook environment.

When trigger-safe publication is requested, dispatch snapshots the triggering
commit as `expected_parent`. The guarded receive must contain exactly one ref
update whose canonical old object equals that commit. A stale parent, missing
context, malformed repository fact, expired scope, hook failure, or any
out-of-scope item denies the complete batch.

An accepted receive is durable publication. A crash after acceptance does not
cause host rollback. Repeating an already-applied transition is harmless if it
changes no ref; an ordinary stale or non-fast-forward retry is denied by Git or
the capability policy. Guest exit, revocation, and expiry destroy remaining
handoff material; recovery purges expired encrypted envelopes. Proposal-mode
runs have no runtime Git snapshot, so issuance fails closed and they receive no
Git credential.

Revocation is evaluated when each smart-HTTP operation authenticates. A
receive already authenticated and admitted to Git's atomic pre-receive phase is
allowed to finish; later revocation does not claim to roll back an accepted Git
transaction. Authorized code can still exfiltrate data to a permitted
repository/ref/path destination, so review of the declared capability remains
part of installation trust.
