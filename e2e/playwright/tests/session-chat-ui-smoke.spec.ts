import {expect, test} from "@playwright/test";
import {createReadStream, statSync} from "node:fs";
import {mkdtemp, rm, writeFile} from "node:fs/promises";
import {tmpdir} from "node:os";
import {dirname, extname, join, normalize, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {execFile, spawn} from "node:child_process";
import {promisify} from "node:util";
import {createServer, type IncomingMessage, type ServerResponse} from "node:http";

const exec = promisify(execFile);
const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../examples/session-chat/ui");
const repositoryId = "123e4567-e89b-12d3-a456-426614174000";

function collect(stream: NodeJS.ReadableStream): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    stream.on("data", (chunk: Buffer | string) => chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk)));
    stream.on("error", reject);
    stream.on("end", () => resolve(Buffer.concat(chunks)));
  });
}

async function runGitBackend(root: string, request: IncomingMessage, body: Buffer) {
  const requestPath = request.url?.split("?")[0] ?? "/";
  const pathInfo = requestPath.includes("/git-receive-pack")
    ? "/remote.git/git-receive-pack"
    : "/remote.git/info/refs";
  const query = request.url?.split("?")[1] ?? "";
  const child = spawn("git", ["http-backend"], {
    env: {
      ...process.env,
      GIT_PROJECT_ROOT: root,
      PATH_INFO: pathInfo,
      QUERY_STRING: query,
      REQUEST_METHOD: request.method ?? "GET",
      CONTENT_TYPE: request.headers["content-type"] ?? "",
      CONTENT_LENGTH: String(body.length),
      DOCUMENT_ROOT: root,
      REMOTE_USER: "smoke",
    },
    stdio: ["pipe", "pipe", "pipe"],
  });
  child.stdin.end(body);
  const exit = new Promise<number | null>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  });
  const [stdout, , exitCode] = await Promise.all([collect(child.stdout), collect(child.stderr), exit]);
  return {stdout, exitCode};
}

function statSafe(path: string): boolean {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

async function serveGitAndUi(): Promise<{url: string; remote: string; close: () => Promise<void>}> {
  const fixtureRoot = await mkdtemp(join(tmpdir(), "heph-session-chat-ui-smoke-"));
  const remote = join(fixtureRoot, "remote.git");
  await exec("git", ["init", "--bare", "--initial-branch=main", remote]);
  await writeFile(join(remote, "git-daemon-export-ok"), "");

  const handleRequest = async (request: IncomingMessage, response: ServerResponse) => {
    if (request.url === "/_heph/ui-context") {
      response.writeHead(200, {"cache-control": "no-store", "content-type": "application/json"});
      response.end(JSON.stringify({repository_id: repositoryId}));
      return;
    }

    if (request.url?.startsWith("/_heph/git/")) {
      const backend = await runGitBackend(fixtureRoot, request, await collect(request));
      const separator = Buffer.from("\r\n\r\n");
      const bodyOffset = backend.stdout.indexOf(separator);
      if (backend.exitCode !== 0 || bodyOffset < 0) {
        response.writeHead(500, {"content-type": "text/plain"});
        response.end("git backend failed");
        return;
      }
      const headers: Record<string, string> = {};
      let statusCode = 200;
      const headerText = backend.stdout.subarray(0, bodyOffset).toString("utf8");
      for (const line of headerText.split("\r\n")) {
        const separatorIndex = line.indexOf(":");
        if (separatorIndex <= 0) continue;
        const name = line.slice(0, separatorIndex).toLowerCase();
        const value = line.slice(separatorIndex + 1).trim();
        if (name === "status") {
          const match = /^(\d{3})\b/.exec(value);
          if (match) statusCode = Number(match[1]);
        } else {
          headers[name] = value;
        }
      }
      headers["heph-git-actor-id"] = repositoryId;
      response.writeHead(statusCode, headers);
      response.end(backend.stdout.subarray(bodyOffset + separator.length));
      return;
    }

    const requested = request.url === "/" ? "/index.html" : request.url?.split("?")[0] ?? "/index.html";
    const path = normalize(join(uiRoot, requested));
    if (!(path === uiRoot || path.startsWith(`${uiRoot}/`)) || !statSafe(path)) {
      response.writeHead(404);
      response.end();
      return;
    }
    response.writeHead(200, {"content-type": {".css": "text/css", ".html": "text/html", ".js": "text/javascript"}[extname(path)] ?? "application/octet-stream"});
    createReadStream(path).pipe(response);
  };
  const server = createServer((request, response) => {
    void handleRequest(request, response).catch(() => {
      if (!response.headersSent) response.writeHead(500, {"content-type": "text/plain"});
      if (!response.writableEnded) response.end("smoke server request failed");
    });
  });

  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("smoke server did not expose a TCP address");
  return {
    url: `http://127.0.0.1:${address.port}/index.html`,
    remote,
    close: async () => {
      await new Promise<void>((resolve, reject) => server.close((error) => error === undefined ? resolve() : reject(error)));
      await rm(fixtureRoot, {force: true, recursive: true});
    },
  };
}

async function assertInitializedRepository(remote: string): Promise<void> {
  const tree = await exec("git", ["--git-dir", remote, "ls-tree", "-r", "--name-only", "refs/heads/main"]);
  const paths = tree.stdout.trim().split("\n");
  expect(paths).toEqual(expect.arrayContaining([
    ".heph/session/v1/manifest.json",
    ".heph/session/v1/participants/release%3Areference-chat.json",
    ".heph/session/v1/participants/agent%3Areference-chat.json",
    ".heph/session/v1/participants/user%3A123e4567-e89b-12d3-a456-426614174000.json",
  ]));
  const manifest = await exec("git", ["--git-dir", remote, "show", "refs/heads/main:.heph/session/v1/manifest.json"]);
  expect(JSON.parse(manifest.stdout)).toMatchObject({
    data: {agent_id: "agent:reference-chat", ref: "refs/heads/main", release_id: "release:reference-chat"},
  });
}

test("packaged session chat initializes against a real empty git HTTP repository", async ({page}) => {
  const server = await serveGitAndUi();
  const pageErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error.name));
  try {
    await page.goto(server.url, {waitUntil: "networkidle"});
    await expect(page.locator("[data-status]")).toHaveText(`Connected as user:${repositoryId}`);
    expect(pageErrors).toEqual([]);
    await assertInitializedRepository(server.remote);
  } finally {
    await server.close();
  }
});
