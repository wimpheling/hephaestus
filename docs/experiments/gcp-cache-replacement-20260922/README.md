# GCP Cooking replacement-cache evidence (2026-09-22)

This directory preserves the exact private packaging recipe and safe provenance for a replacement Cooking cache object. It is dated reproduction evidence, not a supported general build tool. The compressed OCI archives remain outside the repository and are intentionally not retained here.

The recipe used these fixed local inputs:

- Repository: `/home/a/projects/hephaestus`
- Platform release: `/home/a/projects/hephaestus/.local/hephaestus/platform-images/releases/581b939d5ad5e5a81e77ad01ad8931487a8d2bcf`
- Platform revision: `581b939d5ad5e5a81e77ad01ad8931487a8d2bcf`
- Recipe source during the run: `/var/tmp/heph-gcp-cache-replacement-20260922/package_cache.py`

The exact invocation was:

```sh
python3 /var/tmp/heph-gcp-cache-replacement-20260922/package_cache.py
```

The recipe requires Python 3, Skopeo, GNU tar, zstd, and GNU `sha256sum`. It generated the Rust profile through `skopeo copy --all --preserve-digests`, normalized inner and outer archive metadata, checked the staged `sha256sums`, and ran the unchanged production cache validator extracted from `scripts/gcp-cooking-run.sh:617-659` before and after extraction. After the recipe terminated, the extracted inventories were checked with these exact posthoc commands:

```sh
(cd /var/tmp/heph-gcp-cache-replacement-20260922/run1/extracted && sha256sum --strict --check sha256sums >/dev/null)
(cd /var/tmp/heph-gcp-cache-replacement-20260922/run2/extracted && sha256sum --strict --check sha256sums >/dev/null)
```

Two independent package runs were byte-identical. The tool-version records, validator-source hash, and posthoc-check fields in `provenance.json` were added after that run; the retained `package_cache.py` remains byte-exact.

The original cache object is missing; expiry is plausible from its lifecycle policy, but deletion or expiry was not proven. The resulting object has a new SHA-256 and does not claim to restore the original cache SHA. Upload and cache-pin changes are separate operator actions. Because the replacement uses a newer Rust image/profile, it still requires fresh cloud runtime acceptance after upload; local packaging and validator success are not cloud acceptance.

The exact invocation above was run against a fresh private output root. The recipe refuses existing `run1`/`run2` directories, so rerunning it requires a new private `OUTPUT_ROOT` path (or a path-adjusted working copy); the retained recipe must remain unchanged.

Recipe SHA-256: `a855833c05e0e8d234b58dcc3fddcad29c8b950abdfff2fb3f99c32026ffa71a`
Provenance SHA-256: `8480926863ff094dd94baf8179b4f7c633d142b9e7c9a7c1f1650b353e71e9fc`
