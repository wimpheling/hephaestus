# Purpose

Explain what `<crate-name>` enables in the product, which workflow or caller
uses it, and the outcome it produces. Describe the role in plain language so a
reader can understand why the crate exists before reading its API.

# Responsibilities

Describe how the crate's operations work together in the real workflow and
what result the caller receives. State its security duties in plain language:
validation, authorization, redaction, credential handling, isolation, or
other protection. Mention an implementation boundary only when it helps the
reader use the crate correctly.

# When

Use this crate when <name the caller and workflow that should invoke it>.
Explain what should already be true, what the call does, and what concrete
result follows. Include one grounded API or command example:

```rust
let result = crate_name::Type::operation(input)?;
```
