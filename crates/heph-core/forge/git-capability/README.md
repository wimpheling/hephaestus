# Purpose

`git-capability-domain` describes the exact Git authority a caller may use on
one repository. It turns an operation, ref and path selection, ref mutation
policy, transfer limits, and expiry into a normalized scope that Git transport
code can enforce consistently.

# Responsibilities

`GitCapabilityCeiling` represents the release or platform maximum. A bound
capability may narrow that ceiling, and `GitCapabilityScope::allows` checks
operation, ref, expiry, and receive changes against the normalized rules.
Explicit glob parsing prevents an accidental broad match; update policies
distinguish branch, tag, and other ref mutations, and transfer limits bound
requests, pack size, object count, and ref updates.

Canonical JSON and `GitCapabilityHash` make the exact scope inspectable and
verifiable. The same scope can authorize runtime Git credentials, personal
access tokens, and Git HTTP operations without allowing one caller to widen a
grant by changing its textual representation.

# When

Use it when a release declares Git access, when an instance narrows that
access, and when a transport checks a request. Construct a
`GitCapabilityScopeInput`, then call `scope.allows(...)` or
`scope.allows_receive(...)` before serving or mutating Git data.
