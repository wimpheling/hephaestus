import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { sourceBaseForRepository, sourceTreeBaseForRepository } from "../scripts/guides.mjs";

const packageDirectory = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(packageDirectory, "..");
const generatedDirectory = resolve(packageDirectory, "public/generated");
const generatedFiles = ["crates.json", "grpc.json", "guides.json"];

test("source bases normalize GitHub origins and refs", () => {
  const expected = "https://github.com/wimpheling/hephaestus/blob/feat/code-surface/";
  assert.equal(sourceBaseForRepository("git@github.com:wimpheling/hephaestus.git", "feat/code-surface"), expected);
  assert.equal(sourceBaseForRepository("https://github.com/wimpheling/hephaestus.git", "feat/code-surface"), expected);
  assert.equal(sourceBaseForRepository("ssh://git@github.com/wimpheling/hephaestus.git", "4253ac6"), "https://github.com/wimpheling/hephaestus/blob/4253ac6/");
  assert.equal(sourceTreeBaseForRepository("git@github.com:wimpheling/hephaestus.git", "feat/code-surface"), "https://github.com/wimpheling/hephaestus/tree/feat/code-surface/");
  assert.equal(sourceBaseForRepository("https://gitlab.com/wimpheling/hephaestus.git", "main"), null);
  assert.equal(sourceTreeBaseForRepository("https://gitlab.com/wimpheling/hephaestus.git", "main"), null);
});

function runGenerator() {
  execFileSync(process.execPath, ["scripts/generate.mjs"], {
    cwd: packageDirectory,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function readGenerated() {
  return new Map(generatedFiles.map((filename) => [filename, readFileSync(resolve(generatedDirectory, filename), "utf8")]));
}

function assertSorted(values, label) {
  assert.deepEqual(values, [...values].sort((left, right) => left.localeCompare(right)), `${label} must be sorted`);
}

test("generated code surface is deterministic and referentially complete", () => {
  runGenerator();
  const first = readGenerated();
  runGenerator();
  const second = readGenerated();
  for (const filename of generatedFiles) {
    assert.equal(second.get(filename), first.get(filename), `${filename} changed between identical generations`);
  }

  const crates = JSON.parse(second.get("crates.json"));
  const grpc = JSON.parse(second.get("grpc.json"));
  const guides = JSON.parse(second.get("guides.json"));
  assert.equal(crates.schemaVersion, 1);
  assert.equal(grpc.schemaVersion, 1);
  assert.equal(guides.schemaVersion, 1);
  assert.equal(typeof guides.sourceBase, "string");
  assert.match(guides.sourceBase, /^https:\/\/github\.com\/[^/]+\/[^/]+\/blob\/.+\/$/);
  assert.equal(typeof guides.sourceTreeBase, "string");
  assert.match(guides.sourceTreeBase, /^https:\/\/github\.com\/[^/]+\/[^/]+\/tree\/.+\/$/);
  assert.ok(guides.sourceDirectories.includes("crates/heph-core/auth/secret"));
  assert.ok(guides.sourceDirectories.includes("crates/heph-std/runtime/vm/libkrun"));

  const workspace = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1"], {
      cwd: repositoryRoot,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
  const workspacePackageNames = new Set(
    workspace.packages
      .filter((pkg) => workspace.workspace_members.includes(pkg.id))
      .map((pkg) => pkg.name),
  );
  assert.deepEqual(new Set(crates.packages.map((pkg) => pkg.id)), workspacePackageNames);
  assert.equal(crates.packages.length, workspace.workspace_members.length);
  assert.equal(new Set(crates.packages.map((pkg) => pkg.id)).size, crates.packages.length);
  assertSorted(crates.packages.map((pkg) => pkg.id), "crate IDs");

  const crateIds = new Set(crates.packages.map((pkg) => pkg.id));
  for (const pkg of crates.packages) {
    assert.ok(!isAbsolute(pkg.manifest), `${pkg.id} has an absolute manifest path`);
    assert.ok(!relative(repositoryRoot, resolve(repositoryRoot, pkg.manifest)).startsWith(".."), `${pkg.id} escapes the repository`);
    assert.ok(existsSync(resolve(repositoryRoot, pkg.manifest)), `${pkg.id} has a missing manifest`);
    assert.ok(!pkg.manifest.includes("\\"), `${pkg.id} has a host-specific path separator`);
    assertSorted(pkg.features, `${pkg.id} features`);
    for (const target of pkg.targets) {
      assertSorted(target.kinds, `${pkg.id}/${target.name} target kinds`);
    }
  }
  for (const dependency of crates.dependencies) {
    assert.ok(crateIds.has(dependency.from), `unknown dependency source ${dependency.from}`);
    assert.ok(crateIds.has(dependency.to), `unknown dependency target ${dependency.to}`);
    assert.ok(dependency.name.length > 0);
    assert.ok(dependency.kinds.length > 0);
    assertSorted(dependency.kinds, `${dependency.from}/${dependency.name} dependency kinds`);
    assert.equal(typeof dependency.optional, "boolean");
    assert.ok(dependency.targets.length > 0);
    assert.ok(dependency.targets.every((target) => typeof target === "string" && target.length > 0));
    assertSorted(dependency.targets, `${dependency.from}/${dependency.name} dependency targets`);
  }

  const fileNames = new Set(grpc.files.map((file) => file.name));
  const messageIds = new Set(grpc.messages.map((message) => message.id));
  const enumIds = new Set(grpc.enums.map((enumDescriptor) => enumDescriptor.id));
  assert.equal(new Set(grpc.files.map((file) => file.name)).size, grpc.files.length);
  assert.equal(new Set(grpc.messages.map((message) => message.id)).size, grpc.messages.length);
  assert.equal(new Set(grpc.enums.map((enumDescriptor) => enumDescriptor.id)).size, grpc.enums.length);
  assert.equal(new Set(grpc.services.map((service) => service.id)).size, grpc.services.length);
  assertSorted(grpc.files.map((file) => file.name), "protobuf files");

  for (const file of grpc.files) {
    assert.equal(file.external, !file.name.startsWith("hephaestus/"));
    for (const importedFile of file.imports) {
      assert.ok(fileNames.has(importedFile), `${file.name} imports missing ${importedFile}`);
    }
  }
  for (const service of grpc.services) {
    assert.ok(fileNames.has(service.file), `${service.id} has an unknown file`);
    for (const method of service.methods) {
      assert.ok(messageIds.has(method.input), `${method.id} has an unknown input ${method.input}`);
      assert.ok(messageIds.has(method.output), `${method.id} has an unknown output ${method.output}`);
    }
  }
  for (const message of grpc.messages) {
    assert.ok(fileNames.has(message.file), `${message.id} has an unknown file`);
    for (const field of message.fields) {
      if (field.typeName !== null) {
        assert.ok(messageIds.has(field.typeName) || enumIds.has(field.typeName), `${message.id}.${field.name} has an unknown type`);
      }
    }
  }
  for (const enumDescriptor of grpc.enums) {
    assert.ok(fileNames.has(enumDescriptor.file), `${enumDescriptor.id} has an unknown file`);
  }

  const contextNames = new Set(crates.packages.map((pkg) => pkg.context).filter(Boolean));
  const serviceIds = new Set(grpc.services.map((service) => service.id));
  assert.ok(guides.guides.length > 0, "at least one guide must be generated");
  assert.equal(new Set(guides.guides.map((guide) => guide.id)).size, guides.guides.length);
  assert.equal(new Set(guides.guides.map((guide) => guide.path)).size, guides.guides.length);
  const guideById = new Map(guides.guides.map((guide) => [guide.id, guide]));
  for (const guide of guides.guides) {
    assert.ok(!isAbsolute(guide.path), `${guide.id} has an absolute source path`);
    assert.ok(guide.path === "README.md" || guide.path.endsWith("/README.md"), `${guide.id} has an invalid guide source`);
    const sourceExists = existsSync(resolve(repositoryRoot, guide.path));
    assert.equal(guide.missingDocumentation, !sourceExists, `${guide.id} has an incorrect documentation flag`);
    if (sourceExists) assert.ok(guide.content.trim().length > 0, `${guide.id} has empty content`);
    else assert.equal(guide.content, "", `${guide.id} should not synthesize placeholder content`);
    if (guide.parent) assert.ok(guideById.has(guide.parent), `${guide.id} has an unknown parent ${guide.parent}`);
    for (const context of guide.contexts) assert.ok(contextNames.has(context), `${guide.id} references unknown context ${context}`);
    for (const service of guide.services) assert.ok(serviceIds.has(service), `${guide.id} references unknown service ${service}`);
  }
  const crateGuides = guides.guides.filter((guide) => guide.crateId);
  assert.equal(crateGuides.length, crates.packages.length, "every workspace crate needs a guide entry");
  assert.equal(new Set(crateGuides.map((guide) => guide.crateId)).size, crateGuides.length, "each crate needs exactly one guide entry");
  const packageById = new Map(crates.packages.map((pkg) => [pkg.id, pkg]));
  for (const guide of crateGuides) {
    const pkg = packageById.get(guide.crateId);
    assert.ok(pkg, `${guide.id} references an unknown crate ${guide.crateId}`);
    assert.equal(guide.manifest, pkg.manifest);
    assert.equal(guide.directory, pkg.manifest.slice(0, -"/Cargo.toml".length));
  }
  assert.ok(guides.guides.some((guide) => guide.missingDocumentation), "the tree should expose missing documentation");
  assert.equal(guideById.get("root")?.path, "README.md");
  assert.deepEqual(
    guides.guides.filter((guide) => guide.parent === "root").map((guide) => guide.id).sort(),
    ["app", "core", "dev", "std"],
    "the repository root should expose conceptual top-level branches",
  );
  assert.equal(guideById.has("directory-637261746573"), false, "the crates source wrapper must not become a guide node");
  assert.equal(guideById.get("dev")?.path, "crates/heph-dev/README.md");
  assert.equal(guideById.get("dev")?.parent, "root");
  assert.equal(guideById.get("dev")?.crateId, "hephaestus-dev");
  assert.equal(guideById.get("dev")?.missingDocumentation, true, "development branch should call out its missing README");
  assert.equal(guideById.get("dev")?.content, "", "missing documentation must not synthesize README content");
  assert.equal(guideById.get("core")?.parent, "root");
  for (const id of ["auth", "forge", "image", "runtime", "platform"]) assert.equal(guideById.get(id)?.parent, "core");
  assert.equal(guideById.get("auth-identity")?.parent, "auth");
  for (const context of guides.unplaced.contexts) assert.ok(contextNames.has(context), `unplaced context ${context} is unknown`);
  for (const service of guides.unplaced.services) assert.ok(serviceIds.has(service), `unplaced service ${service} is unknown`);
  assertSorted(guides.unplaced.contexts, "unplaced contexts");
  assertSorted(guides.unplaced.services, "unplaced services");

  const serialized = `${JSON.stringify({ crates, grpc, guides })}`;
  assert.ok(!serialized.includes(repositoryRoot), "generated data contains an absolute repository path");
});
