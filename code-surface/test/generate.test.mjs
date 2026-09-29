import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, isAbsolute, relative, resolve } from "node:path";

const packageDirectory = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(packageDirectory, "..");
const generatedDirectory = resolve(packageDirectory, "public/generated");
const generatedFiles = ["crates.json", "grpc.json"];

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
  assert.equal(crates.schemaVersion, 1);
  assert.equal(grpc.schemaVersion, 1);

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

  const serialized = `${JSON.stringify({ crates, grpc })}`;
  assert.ok(!serialized.includes(repositoryRoot), "generated data contains an absolute repository path");
});
