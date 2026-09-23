#!/usr/bin/env python3
from __future__ import annotations

import base64
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile

REPO = Path('/home/a/projects/hephaestus')
REVISION = '581b939d5ad5e5a81e77ad01ad8931487a8d2bcf'
RELEASE = REPO / '.local/hephaestus/platform-images/releases' / REVISION
OUTPUT_ROOT = Path('/var/tmp/heph-gcp-cache-replacement-20260922')
BUILDERS = (
    'ubuntu-native', 'rust-ubuntu', 'typescript-node-ubuntu',
    'python-ubuntu', 'oci-builder-ubuntu', 'oci-verifier-ubuntu',
)


def run(args: list[str], *, stdout=None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, check=True, text=True, stdout=stdout)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def regular_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise RuntimeError(f'symlink in staging: {path}')
        if path.is_file():
            files.append(path)
    return files


def normalize_inner_oci_archive(raw: Path, final: Path) -> None:
    members: list[tuple[str, tarfile.TarInfo, bytes | None]] = []
    with tarfile.open(raw, mode='r:*') as source:
        seen: set[str] = set()
        for member in source:
            name = member.name
            if not name or name.startswith('/') or '..' in PurePosixPath(name).parts:
                raise RuntimeError(f'unsafe OCI archive member: {name!r}')
            if name in seen:
                raise RuntimeError(f'duplicate OCI archive member: {name!r}')
            seen.add(name)
            if member.isdir():
                data = None
            elif member.isfile():
                stream = source.extractfile(member)
                if stream is None:
                    raise RuntimeError(f'cannot read OCI member: {name!r}')
                data = stream.read()
            else:
                raise RuntimeError(f'unsupported OCI archive member: {name!r}')
            members.append((name, member, data))
    final.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tarfile.open(final, mode='w') as output:
        for name, original, data in sorted(members, key=lambda item: item[0]):
            info = tarfile.TarInfo(name)
            info.mtime = 0
            info.uid = 0
            info.gid = 0
            info.uname = ''
            info.gname = ''
            if original.isdir():
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                info.size = 0
                output.addfile(info)
            else:
                assert data is not None
                info.type = tarfile.REGTYPE
                info.mode = 0o644
                info.size = len(data)
                output.addfile(info, io.BytesIO(data))


def create_profile(stage: Path) -> tuple[str, str]:
    rust_layout = RELEASE / 'rust-ubuntu' / 'image'
    rust_index = json.loads((rust_layout / 'index.json').read_text())
    rust_descriptor = rust_index['manifests'][0]
    rust_digest = rust_descriptor['digest']
    tag = rust_descriptor['annotations']['org.opencontainers.image.ref.name']
    guest_dir = stage / 'guest-images'
    guest_dir.mkdir(mode=0o700)
    raw = guest_dir / 'rust-profile.raw.oci'
    final = guest_dir / 'rust-ubuntu-profile.oci'
    run([
        'skopeo', 'copy', '--all', '--preserve-digests',
        f'oci:{rust_layout}:{tag}',
        f'oci-archive:{raw}:rust-ubuntu',
    ])
    normalize_inner_oci_archive(raw, final)
    raw.unlink()
    inspected = subprocess.check_output([
        'skopeo', 'inspect', '--format', '{{.Digest}}',
        f'oci-archive:{final}:rust-ubuntu',
    ], text=True).strip()
    if inspected != rust_digest:
        raise RuntimeError(f'Rust profile digest mismatch: {inspected} != {rust_digest}')
    return rust_digest, sha256_file(final)


def write_bundle_metadata(stage: Path) -> None:
    source_refs: dict[str, str] = {}
    for key in BUILDERS:
        release_input = json.loads((RELEASE / key / 'release-input.json').read_text())
        source_refs[key] = release_input['manifest_digest']
    host = 'localhost:55000/platform/images'
    refs = {key: f'{host}/{key}@{source_refs[key]}' for key in BUILDERS}
    layouts = {key: f'layouts/{key}/image' for key in BUILDERS}
    base_refs = {
        f'{host}/{key}@{source_refs[key]}': layouts[key]
        for key in ('python-ubuntu', 'rust-ubuntu', 'typescript-node-ubuntu', 'ubuntu-native')
    }
    (stage / 'base-layouts.json').write_text(json.dumps(base_refs, sort_keys=True, separators=(',', ':')) + '\n')
    (stage / 'workflow.env.template').write_text(
        '\n'.join([
            'version=1',
            f'platform_revision={REVISION}',
            f'builder_vm_image={refs["oci-builder-ubuntu"]}',
            'builder_layout=layouts/oci-builder-ubuntu/image',
            'builder_layout_tag=heph-' + source_refs['oci-builder-ubuntu'].replace(':', '-'),
            f'verifier_vm_image={refs["oci-verifier-ubuntu"]}',
            'verifier_layout=layouts/oci-verifier-ubuntu/image',
            'verifier_layout_tag=heph-' + source_refs['oci-verifier-ubuntu'].replace(':', '-'),
            'base_layout_manifest=base-layouts.json',
            '',
        ])
    )
    release_dir = stage / 'release-inputs'
    release_dir.mkdir(mode=0o700)
    for key in ('oci-builder-ubuntu', 'oci-verifier-ubuntu'):
        record = json.loads((RELEASE / key / 'release-input.json').read_text())
        record['layout'] = layouts[key]
        (release_dir / f'{key}.json').write_text(json.dumps(record, sort_keys=True, indent=2) + '\n')
    rust_digest, profile_hash = create_profile(stage)
    manifest = {
        'schema_version': 1,
        'kind': 'hephaestus.gcp-cooking-cache.v1',
        'platform_revision': REVISION,
        'required_layouts': list(BUILDERS),
        'runtime_image_references': {
            'HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE': refs['python-ubuntu'],
            'HEPHAESTUS_LIBKRUN_RUST_BUILDER_IMAGE': refs['rust-ubuntu'],
        },
        'release_inputs': {
            'oci-builder-ubuntu': 'release-inputs/oci-builder-ubuntu.json',
            'oci-verifier-ubuntu': 'release-inputs/oci-verifier-ubuntu.json',
        },
        'guest_images': {
            'rust-ubuntu-profile': {
                'archive': 'guest-images/rust-ubuntu-profile.oci',
                'archive_sha256': profile_hash,
                'manifest_digest': rust_digest,
            },
        },
    }
    (stage / 'cache-manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')


def write_checksums(stage: Path) -> None:
    entries = []
    for path in regular_files(stage):
        if path.name == 'sha256sums':
            continue
        rel = path.relative_to(stage).as_posix()
        entries.append(f'{sha256_file(path)}  {rel}')
    (stage / 'sha256sums').write_text('\n'.join(sorted(entries)) + '\n')


def create_outer_archive(stage: Path, archive: Path) -> None:
    archive.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    tar_process = subprocess.Popen([
        'tar', '--create', '--sort=name', '--format=posix',
        '--mtime=UTC 1970-01-01', '--owner=0', '--group=0', '--numeric-owner',
        '--pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime', '--file=-', '-C', str(stage), '.',
    ], stdout=subprocess.PIPE)
    assert tar_process.stdout is not None
    zstd_process = subprocess.Popen([
        'zstd', '--compress', '--ultra', '-19', '--threads=1', '--stdout',
    ], stdin=tar_process.stdout, stdout=archive.open('wb'))
    tar_process.stdout.close()
    zstd_status = zstd_process.wait()
    tar_status = tar_process.wait()
    if zstd_status or tar_status:
        raise RuntimeError(f'archive failed: tar={tar_status}, zstd={zstd_status}')


def validate_archive_safety(archive: Path) -> None:
    process = subprocess.Popen(['zstd', '-dc', str(archive)], stdout=subprocess.PIPE)
    assert process.stdout is not None
    try:
        with tarfile.open(fileobj=process.stdout, mode='r|') as outer:
            count = total = 0
            for member in outer:
                name = PurePosixPath(member.name)
                if name.is_absolute() or '..' in name.parts or member.name.startswith('./../'):
                    raise RuntimeError(f'archive path escapes extraction root: {member.name}')
                if member.issym() or member.islnk() or not (member.isfile() or member.isdir()):
                    raise RuntimeError(f'archive contains unsupported member: {member.name}')
                count += 1
                total += member.size
                if count > 100000 or total > 8 * 1024 * 1024 * 1024:
                    raise RuntimeError('archive extraction budget exceeded')
    finally:
        process.stdout.close()
        if process.wait() != 0:
            raise RuntimeError('zstd failed while reading cache archive')


def production_validate(root: Path) -> None:
    validator_source = REPO / 'scripts/gcp-cooking-run.sh'
    text = validator_source.read_text()
    marker = "python3 - \"$stage_root\" <<'PY'\n"
    if marker not in text:
        raise RuntimeError('production cache validator heredoc not found')
    body = text.split(marker, 1)[1].split('\nPY\n', 1)[0]
    subprocess.run([sys.executable, '-c', body, str(root)], check=True)


def package(run_name: str) -> dict[str, object]:
    run_root = OUTPUT_ROOT / run_name
    if run_root.exists():
        raise RuntimeError(f'output exists: {run_root}')
    run_root.mkdir(mode=0o700)
    stage = run_root / 'bundle'
    stage.mkdir(mode=0o700)
    for key in BUILDERS:
        shutil.copytree(RELEASE / key / 'image', stage / 'layouts' / key / 'image', symlinks=False)
    write_bundle_metadata(stage)
    write_checksums(stage)
    subprocess.run(['sha256sum', '--strict', '--check', 'sha256sums'], cwd=stage, check=True, stdout=subprocess.DEVNULL)
    production_validate(stage)
    archive = run_root / 'heph-gcp-cooking-cache-replacement.tar.zst'
    create_outer_archive(stage, archive)
    validate_archive_safety(archive)
    extracted = run_root / 'extracted'
    extracted.mkdir(mode=0o700)
    with subprocess.Popen(['zstd', '-dc', str(archive)], stdout=subprocess.PIPE) as decode:
        assert decode.stdout is not None
        subprocess.run(['tar', '-xf', '-', '-C', str(extracted), '--no-same-owner', '--no-same-permissions', '--no-overwrite-dir'], check=True, stdin=decode.stdout)
        decode.stdout.close()
        if decode.wait() != 0:
            raise RuntimeError('archive extraction failed')
    production_validate(extracted)
    size = archive.stat().st_size
    digest = sha256_file(archive)
    md5 = hashlib.md5(archive.read_bytes()).digest()
    result = {'archive': str(archive), 'sha256': digest, 'size_bytes': size, 'md5_base64': base64.b64encode(md5).decode(), 'platform_revision': REVISION}
    (run_root / 'result.json').write_text(json.dumps(result, sort_keys=True, indent=2) + '\n')
    return result


def main() -> None:
    if len(sys.argv) != 1:
        raise SystemExit('no arguments')
    first = package('run1')
    second = package('run2')
    if first['sha256'] != second['sha256'] or first['size_bytes'] != second['size_bytes'] or first['md5_base64'] != second['md5_base64']:
        raise RuntimeError('two package runs differ')
    provenance = {'recipe': str(Path(__file__).resolve()), 'source_repository': str(REPO), 'source_platform_release': str(RELEASE), 'platform_revision': REVISION, 'runs': [first, second], 'byte_identical': True}
    (OUTPUT_ROOT / 'provenance.json').write_text(json.dumps(provenance, sort_keys=True, indent=2) + '\n')
    print(json.dumps(provenance, sort_keys=True))


if __name__ == '__main__':
    main()
