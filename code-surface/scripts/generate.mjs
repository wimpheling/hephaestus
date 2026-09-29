import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { generateGuides, sourceBaseForRepository, sourceTreeBaseForRepository } from "./guides.mjs";
import { generateRustApi } from "./rustdoc.mjs";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const surfaceDirectory = resolve(scriptDirectory, "..");
const repositoryRoot = resolve(surfaceDirectory, "..");
const generatedDirectory = resolve(surfaceDirectory, "public/generated");

function run(command, args, description) {
  try {
    return execFileSync(command, args, {
      cwd: repositoryRoot,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const detail = error.stderr?.trim() || error.message;
    throw new Error(`Unable to ${description}: ${detail}`);
  }
}

function readGitValue(args) {
  try {
    return execFileSync("git", args, {
      cwd: repositoryRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return "";
  }
}

function sourceBase() {
  const origin = readGitValue(["config", "--get", "remote.origin.url"]);
  const branch = readGitValue(["symbolic-ref", "--short", "HEAD"]);
  const revision = readGitValue(["rev-parse", "HEAD"]);
  return sourceBaseForRepository(origin, branch || revision);
}

function sourceTreeBase() {
  const origin = readGitValue(["config", "--get", "remote.origin.url"]);
  const branch = readGitValue(["symbolic-ref", "--short", "HEAD"]);
  const revision = readGitValue(["rev-parse", "HEAD"]);
  return sourceTreeBaseForRepository(origin, branch || revision);
}

function relativePath(path) {
  return relative(repositoryRoot, path).split("\\").join("/");
}

function selectBuf() {
  if (process.env.BUF) {
    return process.env.BUF;
  }

  try {
    return execFileSync("which", ["buf"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    const localBuf = resolve(repositoryRoot, ".local/protobuf/bin/buf");
    try {
      execFileSync(localBuf, ["--version"], { stdio: "ignore" });
      return localBuf;
    } catch {
      throw new Error(
        "Buf is required to generate the gRPC surface. Install buf, set BUF to its executable, " +
          "or provide .local/protobuf/bin/buf.",
      );
    }
  }
}

function packageId(packageRecord) {
  return packageRecord.name;
}

function generateCrates(metadata) {
  const workspaceIds = new Set(metadata.workspace_members);
  const workspacePackages = metadata.packages
    .filter((packageRecord) => workspaceIds.has(packageRecord.id))
    .sort((left, right) => left.name.localeCompare(right.name));
  const packageByManifest = new Map(
    workspacePackages.map((packageRecord) => [resolve(repositoryRoot, packageRecord.manifest_path), packageRecord]),
  );

  const packages = workspacePackages.map((packageRecord) => {
    const hephaestusMetadata = packageRecord.metadata?.hephaestus ?? {};
    return {
      id: packageId(packageRecord),
      name: packageRecord.name,
      description: packageRecord.description ?? null,
      manifest: relativePath(packageRecord.manifest_path),
      context: hephaestusMetadata.context ?? null,
      layer: hephaestusMetadata.layer ?? null,
      targets: packageRecord.targets
        .map((target) => ({ name: target.name, kinds: [...target.kind].sort() }))
        .sort((left, right) => left.name.localeCompare(right.name)),
      features: Object.keys(packageRecord.features).sort(),
    };
  });

  const dependencyRecords = new Map();
  for (const packageRecord of workspacePackages) {
    for (const dependency of packageRecord.dependencies) {
      if (!dependency.path) {
        continue;
      }
      const target = packageByManifest.get(resolve(repositoryRoot, dependency.path, "Cargo.toml"));
      if (!target) {
        continue;
      }
      const optional = dependency.optional === true;
      const targetCondition = dependency.target ?? "all";
      const key = `${packageId(packageRecord)}\0${packageId(target)}\0${dependency.name}\0${optional}\0${targetCondition}`;
      const record = dependencyRecords.get(key) ?? {
        from: packageId(packageRecord),
        to: packageId(target),
        name: dependency.name,
        kinds: new Set(),
        optional,
        targets: new Set(),
      };
      record.kinds.add(dependency.kind ?? "normal");
      record.targets.add(targetCondition);
      dependencyRecords.set(key, record);
    }
  }

  const dependencies = [...dependencyRecords.values()]
    .map((record) => ({
      from: record.from,
      to: record.to,
      name: record.name,
      kinds: [...record.kinds].sort(),
      optional: record.optional,
      targets: [...record.targets].sort(),
    }))
    .sort((left, right) =>
      left.from.localeCompare(right.from) ||
      left.to.localeCompare(right.to) ||
      left.name.localeCompare(right.name) ||
      Number(left.optional) - Number(right.optional) ||
      left.targets.join("\0").localeCompare(right.targets.join("\0")),
    );

  return { schemaVersion: 1, packages, dependencies };
}

function generateRustApis(workspacePackages) {
  let docsReady = true;
  try {
    run("cargo", ["doc", "--workspace", "--all-features", "--no-deps"], "generate the Rust public API documentation");
  } catch (error) {
    docsReady = false;
    console.warn(`${error.message}. Rust API inventory will show unavailable status.`);
  }
  const apis = generateRustApi(workspacePackages, {
    docRoot: resolve(repositoryRoot, "target/doc"),
    docsReady,
  });
  return new Map(workspacePackages.map((packageRecord, index) => [packageRecord.name, apis[index]]));
}

function fqn(packageName, name) {
  return packageName ? `${packageName}.${name}` : name;
}

function normalizedTypeName(typeName) {
  return typeName?.replace(/^\./, "") ?? null;
}

function generateGrpc(descriptorSet) {
  const files = descriptorSet.file
    .map((file) => ({
      name: file.name,
      package: file.package ?? "",
      imports: [...(file.dependency ?? [])].sort(),
      external: !file.name.startsWith("hephaestus/"),
    }))
    .sort((left, right) => left.name.localeCompare(right.name));

  const services = [];
  const messages = [];
  const enums = [];

  for (const file of descriptorSet.file) {
    const external = !file.name.startsWith("hephaestus/");
    const packageName = file.package ?? "";

    function visitEnum(enumDescriptor, parentName) {
      const id = fqn(packageName, fqn(parentName, enumDescriptor.name));
      enums.push({
        id,
        name: enumDescriptor.name,
        file: file.name,
        package: packageName,
        external,
        values: [...(enumDescriptor.value ?? [])]
          .map((value) => ({ name: value.name, number: value.number }))
          .sort((left, right) => left.number - right.number || left.name.localeCompare(right.name)),
      });
    }

    function visitMessage(messageDescriptor, parentName) {
      const id = fqn(packageName, fqn(parentName, messageDescriptor.name));
      messages.push({
        id,
        name: messageDescriptor.name,
        file: file.name,
        package: packageName,
        external,
        fields: [...(messageDescriptor.field ?? [])]
          .map((field) => ({
            name: field.name,
            number: field.number,
            type: field.type,
            typeName: normalizedTypeName(field.typeName),
            label: field.label,
          }))
          .sort((left, right) => left.number - right.number || left.name.localeCompare(right.name)),
      });
      const nestedParent = fqn(parentName, messageDescriptor.name);
      for (const enumDescriptor of messageDescriptor.enumType ?? []) {
        visitEnum(enumDescriptor, nestedParent);
      }
      for (const nestedMessage of messageDescriptor.nestedType ?? []) {
        visitMessage(nestedMessage, nestedParent);
      }
    }

    for (const enumDescriptor of file.enumType ?? []) {
      visitEnum(enumDescriptor, "");
    }
    for (const messageDescriptor of file.messageType ?? []) {
      visitMessage(messageDescriptor, "");
    }
    for (const service of file.service ?? []) {
      const id = fqn(packageName, service.name);
      services.push({
        id,
        name: service.name,
        file: file.name,
        package: packageName,
        external,
        methods: [...(service.method ?? [])]
          .map((method) => ({
            id: `${id}.${method.name}`,
            name: method.name,
            input: normalizedTypeName(method.inputType),
            output: normalizedTypeName(method.outputType),
            clientStreaming: method.clientStreaming === true,
            serverStreaming: method.serverStreaming === true,
          }))
          .sort((left, right) => left.name.localeCompare(right.name)),
      });
    }
  }

  services.sort((left, right) => left.id.localeCompare(right.id));
  messages.sort((left, right) => left.id.localeCompare(right.id));
  enums.sort((left, right) => left.id.localeCompare(right.id));
  return { schemaVersion: 1, files, services, messages, enums };
}

function writeJson(filename, value) {
  writeFileSync(resolve(generatedDirectory, filename), `${JSON.stringify(value, null, 2)}\n`);
}

function main() {
  const metadata = JSON.parse(run("cargo", ["metadata", "--format-version", "1"], "run cargo metadata"));
  const buf = selectBuf();
  const descriptorJson = run(buf, ["build", "-o", "-#format=json"], "build the protobuf descriptor with Buf");
  const descriptorSet = JSON.parse(descriptorJson);

  mkdirSync(generatedDirectory, { recursive: true });
  const workspaceIds = new Set(metadata.workspace_members);
  const workspacePackages = metadata.packages.filter((packageRecord) => workspaceIds.has(packageRecord.id));
  const crates = generateCrates(metadata);
  const rustApis = generateRustApis(workspacePackages);
  const grpc = generateGrpc(descriptorSet);
  const guides = generateGuides({
    repositoryRoot,
    crates,
    grpc,
    rustApis,
    sourceBase: sourceBase(),
    sourceTreeBase: sourceTreeBase(),
  });
  writeJson("crates.json", crates);
  writeJson("grpc.json", grpc);
  writeJson("guides.json", guides);
  const unplacedCount = guides.unplaced.contexts.length + guides.unplaced.services.length;
  console.log(`Generated ${metadata.workspace_members.length} crates, ${descriptorSet.file.length} protobuf files, and ${guides.guides.length} guides.`);
  if (unplacedCount > 0) {
    console.log(`Unplaced inventory entries: ${unplacedCount} (${guides.unplaced.contexts.length} contexts, ${guides.unplaced.services.length} services).`);
  }
}

main();
