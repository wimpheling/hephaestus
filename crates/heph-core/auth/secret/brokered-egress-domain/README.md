# Purpose

`brokered-egress-domain` describes exactly where a broker may place a secret
during an HTTPS request. A rule binds one secret version to one instance
revision and exact origin, header, and injection location, producing a stable
placeholder that the guest or gateway can refer to without seeing the value.

# Responsibilities

Rule construction parses and normalizes the HTTPS origin and header name so a
configuration cannot silently widen its destination or injection target. The
normalized `BrokeredSecretRule` records the binding, instance revision, secret
version, injection location, and optional gateway route that will use it.

The released configuration gets a `placeholder()` token, while the host uses
`normalized_hash()` to compare the rule during a later authority check. Both
values describe where substitution is allowed and neither operation handles
plaintext.

# When

After the application authorizes a brokered binding, normalize and persist its
rule. The host can later compare the hash before substituting a value:

```rust
let rule = BrokeredSecretRule { id, binding_id, instance_revision_id,
    secret_version_id, destination: Some(ExactHttpsOrigin::parse(
        "https://api.example.test")?), location, gateway_route_id };
let rule = rule.normalized()?;
let placeholder = rule.placeholder();
```
