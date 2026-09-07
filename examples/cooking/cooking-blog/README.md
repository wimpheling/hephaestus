# Cooking Blog

Source for the Cooking Notes Hugo site. Recipe pages belong under
`content/recipes/`; generated output is intentionally not tracked.

## Check the site

Run Hugo from this directory:

```sh
hugo
```

The generated site is written to `public/`, which is ignored by Git.

The cooking agent runs the declared offline source check `python3 check.py` before
the platform imports its controlled proposal. The declared isolated build in
`agent.toml` runs `build.sh` with the same-project `cooking-blog-hugo` image and exports `public/`; generated output is
never proposed into Git. It also exports `bin/check-site`, a verifier that checks
the released HTML without a source checkout, state volume, or network access.
Recipe pages are stable `content/recipes/recipe-<provider-update-id>.md` files.

The offline image build starts from the official
[Hugo v0.165.0 source tag](https://github.com/gohugoio/hugo/tree/v0.165.0),
whose source archive is vendored with SHA-256
`e9c1e7d8e6e09356cc56317fd01b7493d712692390b89b3d33810cfe1305650e`.
It applies a reviewed dependency patch (`x/crypto` 0.55.0 and `x/mod` 0.40.0,
with their resulting module graph) and builds with the official Go 1.27.1
Linux amd64 archive (`63d339f0da5ab53635a56f2490a7984dfe12dfcff22ad749f63edaf590168445`).
The complete vendored module archive is pinned as
`9abcbb5581e55c4d4333228a9ad15a4c9323cf82177259aacfc9403b88e0edbc`.
The dependency patch itself is pinned as
`07e8fe1c0e2c0a133ce0df89f7a45b91960002546f0845b52b699e972757fe1d`.
`./verify-hugo.sh` verifies every input, performs two identical offline builds,
and compares the result with the vendored executable
`vendor/hugo-0.165.0-rebuilt` (SHA-256
`9eff60e74ba1387eddbfceb9e48028743d046f06f08eb837eea1daeec88c014c`). The
final image retains the upstream license under
`/usr/local/share/licenses/hugo/LICENSE`.
Pass an output path to keep the verified binary for the application tests:

```sh
./verify-hugo.sh /tmp/cooking-hugo
cd ../cooking-agent
COOKING_HUGO_BINARY=/tmp/cooking-hugo python3 -B -m unittest -v
```

Image preparation uses the platform's offline repository-image workflow
([documentation](../../../docs/repository-image-builds.md)).
The static build selects the resulting same-project image by
`image = { project_image = "cooking-blog-hugo" }`; the platform records its exact
digest. No floating tag or runtime download belongs in the guest.

The E2E fixture publishes the image definition and vendored tool inputs in a
separate repository in the same project, then publishes the site sources with
`agent.toml` in the blog repository. Blog commits therefore build the site
without queuing another tool-image build. The image Dockerfile checks the
rebuilt executable's SHA-256 before making it available to builds.
