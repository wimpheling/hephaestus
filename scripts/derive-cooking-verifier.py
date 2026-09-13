#!/usr/bin/env python3
"""Derive a private Cooking operation image; never publish a platform release."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import tarfile

TARGET = "usr/libexec/hephaestus/oci-verify"
INDEX = "application/vnd.oci.image.index.v1+json"
MANIFEST = "application/vnd.oci.image.manifest.v1+json"
HISTORY = {"created": "2026-09-13T00:00:00Z", "created_by": "COPY hephaestus oci-verify (private workload derivation)"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def regular(path):
    require(stat.S_ISREG(path.lstat().st_mode), f"not a regular file: {path}")
    return path


def write_json(path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def blob(layout, descriptor):
    digest = descriptor["digest"]
    require(re.fullmatch(r"sha256:[0-9a-f]{64}", digest), "invalid blob digest")
    path = regular(layout / "blobs/sha256" / digest[7:])
    require(path.stat().st_size == descriptor["size"] and sha(path) == digest[7:], "blob integrity mismatch")
    return path


def store_blob(layout, contents, media_type):
    digest = "sha256:" + hashlib.sha256(contents).hexdigest()
    (layout / "blobs/sha256" / digest[7:]).write_bytes(contents)
    return {"mediaType": media_type, "digest": digest, "size": len(contents)}


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def image(layout, expected=None):
    require(not layout.is_symlink() and layout.is_dir(), "unsafe layout")
    for directory in (layout / "blobs", layout / "blobs/sha256"):
        require(not directory.is_symlink() and directory.is_dir(), "unsafe blob directory")
    index = json.loads(regular(layout / "index.json").read_text())
    require(index["schemaVersion"] == 2 and len(index["manifests"]) == 1, "expected one image reference")
    outer = index["manifests"][0]
    require(outer["mediaType"] == INDEX, "expected canonical index")
    require(expected is None or outer["digest"] == expected, "baseline reference mismatch")
    wrapper = json.loads(blob(layout, outer).read_text())
    require(wrapper["schemaVersion"] == 2 and len(wrapper["manifests"]) == 1, "expected one platform")
    descriptor = wrapper["manifests"][0]
    require(descriptor["mediaType"] == MANIFEST, "expected OCI manifest")
    manifest = json.loads(blob(layout, descriptor).read_text())
    config = json.loads(blob(layout, manifest["config"]).read_text())
    require(config["os"] == "linux" and config["architecture"] == "amd64", "unexpected platform")
    for layer in manifest["layers"]:
        blob(layout, layer)
    return outer, manifest, config


def derive(args):
    source = regular(args.script)
    require(source.stat().st_size <= 65536, "verifier script exceeds private derivation budget")
    require(re.fullmatch(r"[0-9a-f]{40}", args.source_revision), "invalid source revision")
    require(re.fullmatch(r"[A-Za-z0-9._:/-]+@sha256:[0-9a-f]{64}", args.baseline_reference), "invalid image reference")
    digest = args.baseline_reference.split("@", 1)[1]
    outer, baseline_manifest, baseline_config = image(args.baseline_layout, digest)
    require(outer.get("annotations", {}).get("org.opencontainers.image.ref.name") == "heph-" + digest.replace(":", "-"), "noncanonical baseline tag")
    require(not args.output.exists() and not args.output.is_symlink(), "output must be fresh")
    args.output.mkdir(mode=0o700)
    owned_image = False
    cid = args.output / "container-id"
    success = False

    def podman(entrypoint, *arguments):
        cid.unlink(missing_ok=True)
        try:
            return command("podman", "run", "--rm", "--pull", "never", "--network", "none", "--user", "0:0", "--cidfile", str(cid), "--volume", f"{args.output}:/derivation:rw,Z", "--entrypoint", entrypoint, args.baseline_reference, *arguments)
        finally:
            if cid.exists():
                container = cid.read_text().strip()
                if re.fullmatch(r"[0-9a-f]{64}", container):
                    subprocess.run(["podman", "rm", "--force", container], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
                cid.unlink(missing_ok=True)

    try:
        exists = subprocess.run(["podman", "image", "exists", args.baseline_reference], check=False).returncode
        require(exists in (0, 1), "cannot inspect baseline image")
        # Match the existing rootless store namespace, including sandboxed CI.
        if exists == 1:
            owned_image = True
            command("podman", "unshare", "skopeo", "copy", "--preserve-digests", f"oci:{args.baseline_layout}", f"containers-storage:{args.baseline_reference}")
        require(command("podman", "unshare", "skopeo", "inspect", "--format", "{{.Digest}}", "containers-storage:" + args.baseline_reference) == digest, "imported baseline digest mismatch")
        identity = podman("/bin/sh", "-ec", f"sha256sum /{TARGET} /usr/bin/umoci; /usr/bin/umoci --version").splitlines()
        require(len(identity) == 3 and all(re.fullmatch(r"[0-9a-f]{64}  /[^ ]+", line) for line in identity[:2]), "invalid baseline tool identity")
        script_hash = sha(source)
        record = {"trust": "workload", "purpose": "private verifier derivation, not platform release approval", "baseline_reference": args.baseline_reference, "source_revision": args.source_revision, "script_sha256": script_hash, "umoci_sha256": identity[1].split()[0], "umoci_version": identity[2], "reference": args.baseline_reference, "layout": str(args.baseline_layout), "derived": False}
        if script_hash != identity[0].split()[0]:
            layout = args.output / "image"
            shutil.copytree(args.baseline_layout, layout, symlinks=True)
            # Only regular OCI files are copied into the writable image; never
            # let a malformed unused blob redirect Umoci writes outside it.
            for parent, directories, files in os.walk(layout):
                Path(parent).chmod(0o700)
                for name in directories:
                    require(not (Path(parent) / name).is_symlink(), "symlink in copied layout")
                for name in files:
                    regular(Path(parent) / name).chmod(0o600)
            layer = args.output / "script.tar"
            contents = source.read_bytes()
            with tarfile.open(layer, "w", format=tarfile.USTAR_FORMAT) as archive:
                entry = tarfile.TarInfo(TARGET)
                entry.size, entry.mode, entry.uid, entry.gid, entry.mtime = len(contents), 0o555, 0, 0, 0
                archive.addfile(entry, io.BytesIO(contents))
            podman("/usr/bin/umoci", "raw", "add-layer", "--image", "/derivation/image:" + outer["annotations"]["org.opencontainers.image.ref.name"], "--history.created", HISTORY["created"], "--history.created_by", HISTORY["created_by"], "/derivation/script.tar")
            new_outer, new_manifest, new_config = image(layout)
            require(new_manifest["layers"][:-1] == baseline_manifest["layers"] and len(new_manifest["layers"]) == len(baseline_manifest["layers"]) + 1, "unexpected layer changes")
            expected_manifest = dict(baseline_manifest)
            expected_manifest.update(config=new_manifest["config"], layers=new_manifest["layers"])
            require(new_manifest == expected_manifest, "unexpected manifest changes")
            expected_config = dict(baseline_config)
            expected_config["history"] = baseline_config.get("history", []) + [HISTORY]
            expected_config["rootfs"] = dict(baseline_config["rootfs"])
            expected_config["rootfs"]["diff_ids"] = baseline_config["rootfs"]["diff_ids"] + ["sha256:" + sha(layer)]
            require(new_config == expected_config, "unexpected runtime configuration changes")
            with tarfile.open(blob(layout, new_manifest["layers"][-1]), "r:*") as archive:
                entries = archive.getmembers()
                require(len(entries) == 1 and entries[0].isreg() and entries[0].name == TARGET, "unexpected inserted paths")
                entry = entries[0]
                require((entry.uid, entry.gid, entry.mode, entry.mtime) == (0, 0, 0o555, 0), "incorrect inserted metadata")
                require(archive.extractfile(entry).read() == contents, "inserted script mismatch")
            for path in (args.baseline_layout / "blobs/sha256").iterdir():
                require(sha(regular(path)) == sha(regular(layout / "blobs/sha256" / path.name)), "baseline blob changed")
            # Umoci 0.4.7 writes a gzip timestamp. Normalize only the added
            # layer header, then rebind its manifest/index descriptors. The
            # uncompressed layer and config diff_id remain unchanged.
            added = blob(layout, new_manifest["layers"][-1]).read_bytes()
            require(added[:4] == b"\x1f\x8b\x08\x00", "unexpected gzip header")
            old_generated = {new_outer["digest"], new_manifest["layers"][-1]["digest"]}
            wrapper = json.loads(blob(layout, new_outer).read_text())
            old_generated.add(wrapper["manifests"][0]["digest"])
            new_manifest["layers"][-1] = store_blob(layout, added[:4] + b"\0" * 4 + added[8:], new_manifest["layers"][-1]["mediaType"])
            descriptor = store_blob(layout, json_bytes(new_manifest), MANIFEST)
            wrapper["manifests"][0].update(descriptor)
            new_outer = store_blob(layout, json_bytes(wrapper), INDEX)
            retained = {new_outer["digest"], descriptor["digest"], new_manifest["layers"][-1]["digest"]}
            for obsolete in old_generated - retained:
                if not (args.baseline_layout / "blobs/sha256" / obsolete[7:]).exists():
                    (layout / "blobs/sha256" / obsolete[7:]).unlink()
            new_outer["annotations"] = {"org.opencontainers.image.ref.name": "heph-" + new_outer["digest"].replace(":", "-")}
            write_json(layout / "index.json", {"schemaVersion": 2, "manifests": [new_outer]})
            record.update(derived=True, reference=args.baseline_reference.split("@", 1)[0] + "@" + new_outer["digest"], layout=str(layout), layer_sha256=sha(layer), entrypoint_uid=0, entrypoint_gid=0, entrypoint_mode="0555", history=HISTORY)
        write_json(args.output / "derivation.json", record)
        # Existing Cooking stdout capture retains this bounded workload
        # provenance without changing the diagnostic collector or timing schema.
        print("Cooking verifier derivation: " + json.dumps(record, sort_keys=True))
        success = True
    finally:
        if owned_image:
            subprocess.run(["podman", "rmi", args.baseline_reference], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        if not success:
            shutil.rmtree(args.output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("baseline-layout", "script", "output"):
        parser.add_argument("--" + name, type=lambda value: Path(value).absolute(), required=True)
    parser.add_argument("--baseline-reference", required=True)
    parser.add_argument("--source-revision", required=True)
    args = parser.parse_args()
    # Convert lifecycle signals to unwinding so owned containers/layouts are
    # cleaned while the surrounding Cooking deadline remains authoritative.
    def interrupted(signum, _frame):
        raise InterruptedError(f"derivation interrupted by signal {signum}")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        derive(args)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Cooking verifier derivation failed: {error}\n")


if __name__ == "__main__":
    main()
