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
the platform imports its controlled proposal. A separate immutable static build
runs Hugo and exports `public/`; generated output is never proposed into Git.
Recipe pages are stable `content/recipes/recipe-<provider-update-id>.md` files.

For reproducible image preparation, vendor a checksum-verified Hugo release
archive in a separate reviewed source revision and use the platform's offline
repository-image workflow (`docs/repository-image-builds.md` in Hephaestus).
The static build selects the resulting same-project image by
`image = { project_image = "cooking-blog-hugo" }`; the platform records its exact
digest. No floating tag or runtime download belongs in the guest.
