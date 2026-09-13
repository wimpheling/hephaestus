#!/usr/bin/env python3
"""Private, intentionally incomplete PR fixture for the unchanged a291 helper.

Exit codes are finite stage classifications, not the nested command's status.
The shell emits them using its existing argument-free failure protocol.
"""
import argparse
import hashlib
import importlib.util
from pathlib import Path
import signal
import subprocess
from types import SimpleNamespace

HELPER_SHA256 = "500135da298df559dd8734d1741783e1d9235cb6c470a73871d4d95b596fc5db"
# Exact callsites in the hash-pinned helper; never derive labels from error text.
BASE_IMAGE_STAGES = {64: 10, 66: 11, 67: 12, 68: 13, 69: 13, 70: 14, 71: 15, 72: 16, 73: 17, 74: 17, 75: 18, 76: 19, 77: 20, 78: 21, 79: 22, 80: 22}
DERIVE_STAGES = {85: 25, 86: 26, 87: 27, 88: 28, 91: 29, 92: 30, 93: 31, 110: 32, 111: 32, 114: 33, 115: 34, 116: 35, 117: 36, 118: 37, 122: 38, 125: 39, 126: 39, 128: 39, 130: 39, 132: 40, 133: 40, 134: 40, 135: 40, 136: 40, 137: 41, 138: 42, 139: 43, 142: 43, 144: 43, 145: 43, 146: 43, 147: 43, 148: 44, 149: 44, 150: 44, 152: 44, 153: 44, 154: 45, 155: 45, 159: 46, 160: 46, 162: 46, 164: 46, 165: 46, 166: 46, 167: 46, 168: 46, 169: 46, 170: 46, 171: 46, 172: 46, 173: 46, 174: 46, 175: 47, 178: 47, 182: 48, 184: 48}
SUCCESS = 78
UNKNOWN = 79


def classify(error, helper):
    frames = []
    trace = error.__traceback__
    while trace:
        if Path(trace.tb_frame.f_code.co_filename) == helper:
            frames.append((trace.tb_frame.f_code.co_name, trace.tb_lineno))
        trace = trace.tb_next
    derive = next((line for name, line in frames if name == "derive"), None)
    if derive == 90:
        return next((BASE_IMAGE_STAGES.get(line, UNKNOWN) for name, line in frames if name == "image"), UNKNOWN)
    return DERIVE_STAGES.get(derive, UNKNOWN)


def run(repo, output, local_root):
    helper = repo / "scripts/derive-cooking-verifier.py"
    try:
        if hashlib.sha256(helper.read_bytes()).hexdigest() != HELPER_SHA256:
            return UNKNOWN
        spec = importlib.util.spec_from_file_location("original_derivation", helper)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        workflow = dict(line.split("=", 1) for line in (local_root / "repository-images/workflow.env").read_text().splitlines() if "=" in line)
        args = SimpleNamespace(script=repo / "platform/builders/oci-verifier-ubuntu/oci-verify", source_revision=subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip(), baseline_reference=workflow["verifier_vm_image"], baseline_layout=Path(workflow["verifier_layout"]), output=output)
        module.derive(args)
        return SUCCESS
    except Exception as error:
        return classify(error, helper)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("repo", "output", "local-root"):
        parser.add_argument("--" + name, type=lambda value: Path(value).absolute(), required=True)
    args = parser.parse_args()
    def interrupted(_signum, _frame):
        raise InterruptedError("fixture deadline")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    raise SystemExit(run(args.repo, args.output, args.local_root))


if __name__ == "__main__":
    main()
