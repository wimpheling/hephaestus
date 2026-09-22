import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { readFile, rm } from "node:fs/promises";

const [browser, targetUrl, expected, outputPath, browserLogPath, requireMarkerValue, timeoutValue] = process.argv.slice(2);
if (![browser, targetUrl, expected, outputPath, browserLogPath].every(Boolean)) {
  console.error("probe arguments are incomplete");
  process.exit(2);
}

const timeoutMs = Number(timeoutValue || 15000);
if (!Number.isFinite(timeoutMs) || timeoutMs < 1) {
  console.error("probe timeout is invalid");
  process.exit(2);
}
const startedAt = Date.now();
const deadlineAt = startedAt + timeoutMs;
const profilePath = `/tmp/heph-prerequisite-chrome-profile-${process.pid}-${startedAt}`;
const devtoolsPortPath = `${profilePath}/DevToolsActivePort`;
const chrome = spawn(
  browser,
  [
    "--headless=new",
    "--no-sandbox",
    "--disable-gpu",
    "--disable-background-networking",
    "--remote-debugging-address=127.0.0.1",
    "--remote-debugging-port=0",
    `--user-data-dir=${profilePath}`,
    "about:blank",
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
let browserStderr = "";
chrome.stderr.on("data", (chunk) => {
  browserStderr += chunk.toString();
});

let socket;
let commandId = 0;
const pending = new Map();
let documentResponseStatus;
let documentResponseUrl = "";
let loadedUrl = "";
let navigationError = "";
let certificateError = "";
let targetCrashed = false;
let chromeExitCode = null;
let chromeSignal = null;
let pageKnownMarker = null;
let pageNonempty = null;
chrome.once("exit", (code, signal) => {
  chromeExitCode = code;
  chromeSignal = signal;
});

function remainingMs() {
  const remaining = deadlineAt - Date.now();
  if (remaining <= 0) throw new Error("probe timeout");
  return remaining;
}

function withDeadline(promise) {
  let timeout;
  try {
    timeout = remainingMs();
  } catch (error) {
    promise.catch(() => {});
    throw error;
  }
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("probe timeout")), timeout);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, Math.min(ms, remainingMs())));
}

function record(value) {
  writeFileSync(outputPath, `${JSON.stringify(value)}\n`, { mode: 0o600 });
}

function certificateAuthorityDenied() {
  return navigationError === "net::ERR_CERT_AUTHORITY_INVALID"
    || certificateError === "net::ERR_CERT_AUTHORITY_INVALID"
    || certificateError === "ERR_CERT_AUTHORITY_INVALID";
}

function failResult(reason, exitCode) {
  record({
    expected,
    target_url: targetUrl,
    elapsed_ms: Date.now() - startedAt,
    status: "failed",
    reason,
    navigation_error: navigationError || null,
    certificate_error: certificateError || null,
    document_response_status: documentResponseStatus ?? null,
    document_response_url: documentResponseUrl || null,
    loaded_url: loadedUrl || null,
    target_crashed: targetCrashed,
    certificate_error_marker: certificateAuthorityDenied(),
    known_page_marker: pageKnownMarker,
    document_nonempty: pageNonempty,
    chrome_exit_code: chromeExitCode,
    chrome_signal: chromeSignal,
    exit_code: exitCode,
  });
  process.exitCode = exitCode;
}

function passResult(reason) {
  record({
    expected,
    target_url: targetUrl,
    elapsed_ms: Date.now() - startedAt,
    status: "passed",
    reason,
    navigation_error: navigationError || null,
    certificate_error: certificateError || null,
    document_response_status: documentResponseStatus ?? null,
    document_response_url: documentResponseUrl || null,
    loaded_url: loadedUrl || null,
    target_crashed: targetCrashed,
    certificate_error_marker: certificateAuthorityDenied(),
    known_page_marker: pageKnownMarker,
    document_nonempty: pageNonempty,
    chrome_exit_code: chromeExitCode,
    chrome_signal: chromeSignal,
    exit_code: 0,
  });
  process.exitCode = 0;
}

function send(method, params = {}) {
  const id = ++commandId;
  const timeout = remainingMs();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error("probe timeout"));
    }, timeout);
    const settle = (settler, value) => {
      clearTimeout(timer);
      pending.delete(id);
      settler(value);
    };
    pending.set(id, {
      resolve: (value) => settle(resolve, value),
      reject: (error) => settle(reject, error),
    });
    try {
      socket.send(JSON.stringify({ id, method, params }));
    } catch (error) {
      pending.delete(id);
      clearTimeout(timer);
      reject(error);
    }
  });
}

async function waitFor(predicate) {
  while (true) {
    if (await predicate()) return;
    await sleep(25);
  }
}

async function connectDebugger() {
  while (true) {
    if (chromeExitCode !== null || chromeSignal !== null) {
      throw new Error("browser exited before DevTools endpoint was ready");
    }
    try {
      const activePort = await withDeadline(readFile(devtoolsPortPath, "utf8"));
      const port = Number.parseInt(activePort.split("\n", 1)[0], 10);
      if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("Chrome DevTools port is invalid");
      const controller = new AbortController();
      const timer = setTimeout(
        () => controller.abort(),
        Math.min(1000, Math.max(1, deadlineAt - Date.now())),
      );
      let response;
      try {
        response = await withDeadline(fetch(`http://127.0.0.1:${port}/json/list`, { signal: controller.signal }));
      } finally {
        clearTimeout(timer);
      }
      if (response.ok) {
        const targets = await withDeadline(response.json());
        const target = targets.find((item) => item.type === "page" && item.webSocketDebuggerUrl);
        if (!target) throw new Error("Chrome DevTools page target did not become ready");
        socket = new WebSocket(target.webSocketDebuggerUrl);
        await withDeadline(new Promise((resolve, reject) => {
          socket.addEventListener("open", resolve, { once: true });
          socket.addEventListener("error", reject, { once: true });
        }));
        socket.addEventListener("message", (event) => {
          const message = JSON.parse(event.data);
          if (message.id !== undefined) {
            const request = pending.get(message.id);
            if (request) {
              pending.delete(message.id);
              if (message.error) request.reject(new Error(message.error.message));
              else request.resolve(message.result || {});
            }
          } else if (message.method === "Network.responseReceived") {
            const responseData = message.params.response;
            if (message.params.type === "Document" && responseData.url === targetUrl) {
              documentResponseStatus = responseData.status;
              documentResponseUrl = responseData.url;
            }
          } else if (message.method === "Page.frameNavigated") {
            if (message.params.frame.parentId === undefined) loadedUrl = message.params.frame.url;
          } else if (message.method === "Page.navigate") {
            navigationError = message.params.errorText || navigationError;
          } else if (message.method === "Security.certificateError") {
            if (message.params.requestURL === targetUrl) {
              certificateError = message.params.errorType || certificateError;
            }
          } else if (message.method === "Inspector.targetCrashed") {
            targetCrashed = true;
          }
        });
        return;
      }
    } catch (error) {
      if (error instanceof Error && error.message === "probe timeout") throw error;
      if (chromeExitCode !== null || chromeSignal !== null) {
        throw new Error("browser exited before DevTools endpoint was ready");
      }
      // Chrome is still starting or its debugging endpoint is not ready.
    }
    await sleep(50);
  }
}

async function stopChrome() {
  if (chrome.exitCode !== null || chrome.signalCode !== null) return;
  chrome.kill("SIGTERM");
  const cleanupMs = Math.min(1000, Math.max(0, deadlineAt - Date.now()));
  if (cleanupMs === 0) {
    chrome.kill("SIGKILL");
    return;
  }
  await new Promise((resolve) => {
    const timer = setTimeout(() => {
      chrome.kill("SIGKILL");
      resolve();
    }, cleanupMs);
    chrome.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function evaluatePage() {
  const evaluated = await send("Runtime.evaluate", {
    expression: "JSON.stringify({url: location.href, marker: document.documentElement?.innerHTML.includes('heph-installed-ui-prerequisite'), nonempty: Boolean(document.documentElement?.innerHTML)})",
    returnByValue: true,
  });
  return JSON.parse(evaluated.result?.value || "{}");
}

async function waitForPositivePage(requireMarker) {
  let page = {};
  await waitFor(async () => {
    if (targetCrashed || chromeExitCode !== null || chromeSignal !== null) {
      throw new Error("browser exited before target page was ready");
    }
    if (documentResponseStatus === undefined || loadedUrl !== targetUrl) return false;
    page = await evaluatePage();
    const validStatus = requireMarker
      ? documentResponseStatus === 200
      : Number.isInteger(documentResponseStatus) && documentResponseStatus >= 200 && documentResponseStatus <= 599;
    return validStatus && page.url === targetUrl && page.nonempty === true && (!requireMarker || page.marker === true);
  });
  return page;
}

async function main() {
  try {
    await connectDebugger();
    await send("Network.enable");
    await send("Page.enable");
    await send("Security.enable");
    const navigation = await send("Page.navigate", { url: targetUrl });
    navigationError = navigation.errorText || navigationError;
    if (expected === "fail") {
      await waitFor(() => {
        if (targetCrashed || chromeExitCode !== null || chromeSignal !== null) {
          throw new Error("browser exited before certificate result");
        }
        return Boolean(navigationError || certificateError || documentResponseStatus);
      });
      if (certificateAuthorityDenied()) {
        passResult("certificate-authority-denied");
      } else {
        failResult(navigationError || certificateError || "negative-probe-did-not-deny-certificate", 1);
      }
    } else {
      const page = await waitForPositivePage(requireMarkerValue === "1");
      pageKnownMarker = page.marker === true;
      pageNonempty = page.nonempty === true;
      const requireMarker = requireMarkerValue === "1";
      passResult(requireMarker ? "target-page-200-marker" : "target-page-response");
    }
  } catch (error) {
    const timedOut = error instanceof Error && error.message === "probe timeout";
    const failureExitCode = chromeExitCode !== null && chromeExitCode > 0 ? chromeExitCode : 1;
    failResult(error instanceof Error ? error.message : "probe-failed", timedOut ? 124 : failureExitCode);
  } finally {
    writeFileSync(browserLogPath, browserStderr, { mode: 0o600 });
    if (socket) {
      try {
        socket.close();
      } catch {
        // The bounded WebSocket handshake may have timed out before opening.
      }
    }
    await stopChrome();
    await rm(profilePath, { recursive: true, force: true });
  }
}

await main();
