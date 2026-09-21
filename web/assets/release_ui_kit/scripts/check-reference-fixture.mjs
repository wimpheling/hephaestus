import {execFileSync} from "node:child_process"
import {readFile} from "node:fs/promises"
import {resolve} from "node:path"
import {fileURLToPath} from "node:url"

const packageRoot = resolve(fileURLToPath(new URL("..", import.meta.url)))
const canonicalRoot = resolve(packageRoot, "dist")
const fixtureRoots = [
  ["static", resolve(packageRoot, "../../../examples/cooking/cooking-reference-ui")],
  ["managed", resolve(packageRoot, "../../../examples/cooking/cooking-reference-service-ui")],
]
const files = ["heph-ui-kit-v1.0.0.css", "heph-ui-kit-v1.0.0.js", "manifest.json"]

const managedJavaScriptRouteProbe = String.raw`
import importlib.util
import sys
import threading
from importlib.machinery import SourceFileLoader
from pathlib import Path
from urllib.request import urlopen

service_path, artifact_root, canonical_path = map(Path, sys.argv[1:])
loader = SourceFileLoader("reference_ui_service_fixture", str(service_path))
spec = importlib.util.spec_from_loader(loader.name, loader)
assert spec is not None and spec.loader is not None
service = importlib.util.module_from_spec(spec)
spec.loader.exec_module(service)
service.__file__ = str(artifact_root / "reference-ui-service")
service.ADDRESS = ("127.0.0.1", 0)
server = service.ReferenceServer()
thread = threading.Thread(target=server.serve_forever, name="reference-ui-route-check")
thread.start()
try:
    host, port = server.server_address
    with urlopen(f"http://{host}:{port}/reference/heph-ui-kit-v1.0.0.js", timeout=5) as response:
        body = response.read()
        status = response.status
        content_type = response.headers.get("Content-Type")
    expected = canonical_path.read_bytes()
    assert status == 200, status
    assert content_type == "text/javascript; charset=utf-8", content_type
    assert body == expected, (len(body), len(expected))
    print(f"managed-js-route=passed status={status} content_type={content_type} bytes={len(body)}")
finally:
    server.shutdown()
    thread.join(timeout=5)
    server.server_close()
    assert not thread.is_alive(), "managed service thread did not stop"
`

export async function checkReferenceFixture() {
  for (const [fixtureName, fixtureRoot] of fixtureRoots) {
    const fixtureDist = resolve(fixtureRoot, "vendor/release-ui-kit/v1.0.0/dist")
    for (const file of files) {
      const canonical = await readFile(resolve(canonicalRoot, file))
      const vendored = await readFile(resolve(fixtureDist, file))
      if (!canonical.equals(vendored)) {
        throw new Error(`${fixtureName} reference UI fixture kit file is stale: ${file}`)
      }
    }
  }

  const managedRoot = fixtureRoots[1][1]
  const managedDist = resolve(managedRoot, "vendor/release-ui-kit/v1.0.0/dist")
  const output = execFileSync(
    "python3",
    [
      "-c",
      managedJavaScriptRouteProbe,
      resolve(managedRoot, "reference-ui-service.py"),
      managedDist,
      resolve(canonicalRoot, "heph-ui-kit-v1.0.0.js"),
    ],
    {encoding: "utf8", timeout: 10_000},
  )
  if (!output.includes("managed-js-route=passed")) {
    throw new Error(`managed reference UI JavaScript route probe returned unexpected output: ${output}`)
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await checkReferenceFixture()
  console.log("reference UI fixture kit output matches canonical output")
}
