import {readFile} from "node:fs/promises"
import {resolve} from "node:path"
import {fileURLToPath} from "node:url"

const packageRoot = resolve(fileURLToPath(new URL("..", import.meta.url)))
const canonicalRoot = resolve(packageRoot, "dist")
const fixtureRoots = [
  ["static", resolve(packageRoot, "../../../examples/cooking/cooking-reference-ui")],
  ["managed", resolve(packageRoot, "../../../examples/cooking/cooking-reference-service-ui")],
]
const files = ["heph-ui-kit-v1.0.0.css", "manifest.json"]

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
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await checkReferenceFixture()
  console.log("reference UI fixture kit output matches canonical output")
}
