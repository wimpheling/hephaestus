# Review

This is a manual human review of the current state of the project.

## Architecture

The crates are all at the same level but there are lots of them so it could be clearer.

Some parts of heph are unopinionated : for example VM access is made through a generic trait. What are the other parts that follow this pattern ?

So we should distinguish first between heph-core and heph-std. Heph core is most of the code, but implementaions (such as the VM libkrun engine) are in heph-std

then within heph-core more structure. The distinction sub crates like broker domain service etc is good, but they should be in a parent crate too, where exports are controllded : secret, runtime, run. Then maybe some crates are specific to the git forge component (like build ?). Let's consider some high-level folders like heph-core/forge/build/build-orchestrator

### Max rust file length

it's a good rule to enforce readbility and splitting. 300 or 350 lines seems good to me.
use a custom dylint linter





