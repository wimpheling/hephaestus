import {createHash} from "node:crypto"
import {mkdir, readFile, rm, writeFile} from "node:fs/promises"
import {dirname, resolve} from "node:path"
import {fileURLToPath} from "node:url"

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const tokenSourcePath = resolve(packageRoot, "../css/design_system/tokens.css")
const componentSourcePath = resolve(packageRoot, "src/components.css")
const outputDirectory = resolve(packageRoot, "dist")

function sha256(value) {
  return createHash("sha256").update(value).digest("hex")
}

function withTrailingNewline(value) {
  return value.endsWith("\n") ? value : `${value}\n`
}

async function render() {
  const packageJson = JSON.parse(await readFile(resolve(packageRoot, "package.json"), "utf8"))
  const tokenSource = await readFile(tokenSourcePath, "utf8")
  const componentSource = await readFile(componentSourcePath, "utf8")
  const css = `${withTrailingNewline(tokenSource)}\n${withTrailingNewline(componentSource)}`
  const cssBytes = Buffer.from(css, "utf8")
  const cssFile = `heph-ui-kit-v${packageJson.version}.css`
  const manifest = `${JSON.stringify(
    {
      schema_version: 1,
      package: packageJson.name,
      version: packageJson.version,
      css_file: cssFile,
      css_sha256: sha256(cssBytes),
      token_source: "../css/design_system/tokens.css",
      token_sha256: sha256(tokenSource),
      component_source: "src/components.css",
      component_sha256: sha256(componentSource),
    },
    null,
    2,
  )}\n`

  return {css, cssFile, manifest, packageJson, tokenSource, componentSource}
}

export async function build() {
  const result = await render()
  await rm(outputDirectory, {recursive: true, force: true})
  await mkdir(outputDirectory, {recursive: true})
  await writeFile(resolve(outputDirectory, result.cssFile), result.css)
  await writeFile(resolve(outputDirectory, "manifest.json"), result.manifest)
  return result
}

export async function check() {
  const result = await render()
  const cssPath = resolve(outputDirectory, result.cssFile)
  const manifestPath = resolve(outputDirectory, "manifest.json")
  let actualCss
  let actualManifest
  try {
    actualCss = await readFile(cssPath, "utf8")
    actualManifest = await readFile(manifestPath, "utf8")
  } catch (error) {
    throw new Error(`generated release UI kit output is missing; run npm run build (${error.message})`)
  }
  if (actualCss !== result.css || actualManifest !== result.manifest) {
    throw new Error("generated release UI kit output is stale; run npm run build")
  }
  return result
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = await build()
  console.log(`built ${result.cssFile} (${result.manifest.match(/"css_sha256": "([^"]+)/u)[1]})`)
}
