import {expect, test} from "@playwright/test";
import {readFileSync} from "node:fs";
import {parseReceiveCommand} from "./session-chat-concurrent-parser.mjs";

type SessionChatConcurrentFixture = {
  project_id: string;
  repository_id: string;
  installation_id: string;
  generation_id: string;
  actor_id: string;
  ui_path: string;
  agent_response_text: string;
  initial_transcript_count: string;
  initial_agent_count: string;
};

type CookingFixture = {
  session_chat_concurrent: SessionChatConcurrentFixture;
};

type ReceiveCommand = {
  oldOid: string;
  ref: string;
};

const fixturePath = process.env.HEPHAESTUS_COOKING_BROWSER_FIXTURE;
const oidcUrl = process.env.HEPHAESTUS_OIDC_URL ?? "http://127.0.0.1:5556";

test("cooking concurrent session chat clients reconcile a stale Git push and preserve both turns", async ({page, browser}, testInfo) => {
  test.setTimeout(240_000);
  const session = loadFixture();
  const initialTranscriptCount = parseCount(session.initial_transcript_count);
  const initialAgentCount = parseCount(session.initial_agent_count);
  const contextB = await browser.newContext();
  const pageB = await contextB.newPage();
  let releaseA = () => {};
  let releaseB = () => {};
  let onBResponse: ((response: import("@playwright/test").Response) => void) | undefined;
  try {
    const transcriptA = page.locator("[data-transcript] article");
    const transcriptB = pageB.locator("[data-transcript] article");
    let prefixA: string[] = [];
    await test.step("session-chat-concurrent-initialize", async () => {
      // Each context launches through the platform installation card so each
      // child receives a fresh, single-use handoff grant.
      await launchInstalledSession(page, session);
      await launchInstalledSession(pageB, session);
      await expect(transcriptA).toHaveCount(initialTranscriptCount);
      await expect(transcriptB).toHaveCount(initialTranscriptCount);
      prefixA = (await transcriptA.allTextContents()).map(text => text.trim());
      const prefixB = (await transcriptB.allTextContents()).map(text => text.trim());
      expect(prefixB).toEqual(prefixA);
    });

    const aGate = new Promise<void>(resolve => {
      releaseA = resolve;
    });
    const bGate = new Promise<void>(resolve => {
      releaseB = resolve;
    });
    const aReceiveCommands: ReceiveCommand[] = [];
    const bReceiveCommands: ReceiveCommand[] = [];
    let bReceiveRequests = 0;
    const bReceiveStatuses: number[] = [];
    onBResponse = (response: import("@playwright/test").Response) => {
      const command = receiveCommand(response.request());
      if (command) bReceiveStatuses.push(response.status());
    };
    pageB.on("response", onBResponse);
    const holdAReceive = async (route: import("@playwright/test").Route) => {
      const command = receiveCommand(route.request());
      if (command) {
        aReceiveCommands.push(command);
        if (aReceiveCommands.length === 1) await aGate;
      }
      await route.continue();
    };
    const holdBReceive = async (route: import("@playwright/test").Route) => {
      const command = receiveCommand(route.request());
      if (command) {
        bReceiveCommands.push(command);
        bReceiveRequests += 1;
        if (bReceiveCommands.length === 1) await bGate;
      }
      await route.continue();
    };
    await page.route("**/*", holdAReceive);
    await pageB.route("**/*", holdBReceive);

    const firstMessage = `concurrent browser writer A ${testInfo.workerIndex}-${Date.now()}`;
    const secondMessage = `concurrent browser writer B ${testInfo.workerIndex}-${Date.now()}`;
    const agentA = page.locator("article.message-agent").filter({hasText: session.agent_response_text});
    const agentB = pageB.locator("article.message-agent").filter({hasText: session.agent_response_text});
    const firstBReceive = pageB.waitForResponse(response => {
      try {
        return new URL(response.url()).pathname.endsWith("/git-receive-pack") &&
          response.request().method() === "POST";
      } catch {
        return false;
      }
    }, {timeout: 60_000});
    // Mark pending failures handled while the race gate is being evaluated;
    // Promise.all below still observes and propagates the original failures.
    void firstBReceive.catch(() => undefined);

    // Both clients submit before the held B request is released. A's first
    // assistant commit establishes a deterministic stale parent for B.
    const submitA = submitMessage(page, firstMessage);
    const submitB = submitMessage(pageB, secondMessage);
    void submitA.catch(() => undefined);
    void submitB.catch(() => undefined);
    await test.step("session-chat-concurrent-race", async () => {
      await expect.poll(() => aReceiveCommands.length, {timeout: 60_000}).toBeGreaterThanOrEqual(1);
      await expect.poll(() => bReceiveCommands.length, {timeout: 60_000}).toBeGreaterThanOrEqual(1);
      const firstA = aReceiveCommands[0];
      const firstB = bReceiveCommands[0];
      expect(firstA.ref).toBe("refs/heads/main");
      expect(firstB.ref).toBe("refs/heads/main");
      expect(firstA.oldOid).toMatch(/^[0-9a-f]{40,64}$/);
      expect(firstB.oldOid).toBe(firstA.oldOid);
      releaseA();
      await expect(agentA).toHaveCount(initialAgentCount + 1, {timeout: 60_000});
      await expect(page.locator("[data-status]")).toHaveText("Assistant response received");
    });
    await test.step("session-chat-concurrent-stale-retry", async () => {
      releaseB();
      await Promise.all([submitA, submitB]);
      const rejectedB = await firstBReceive;
      const rejectedBody = new TextDecoder().decode(await rejectedB.body());
      // Smart HTTP normally returns 200 with an `ng refs/heads/main` pkt-line;
      // retain only this fixed protocol marker as evidence of the first reject.
      expect(rejectedB.status()).toBe(200);
      expect(rejectedBody).toMatch(/ng refs\/heads\/main(?:[\s\r\n]|$)/);
      await expect.poll(() => bReceiveRequests, {timeout: 60_000}).toBeGreaterThanOrEqual(2);
      await expect.poll(() => bReceiveStatuses.length, {timeout: 60_000}).toBeGreaterThanOrEqual(2);
      expect(bReceiveStatuses[0]).toBe(200);
      expect(bReceiveStatuses[1]).toBe(200);
      await expect(agentB).toHaveCount(initialAgentCount + 2, {timeout: 60_000});
    });
    await test.step("session-chat-concurrent-reconnect", async () => {
      await reconnect(page);
      await reconnect(pageB);
      await assertFinalTranscript(page, transcriptA, prefixA, initialTranscriptCount, initialAgentCount, firstMessage, secondMessage, session.agent_response_text);
      await assertFinalTranscript(pageB, transcriptB, prefixA, initialTranscriptCount, initialAgentCount, firstMessage, secondMessage, session.agent_response_text);
    });
  } finally {
    releaseA();
    releaseB();
    if (onBResponse) pageB.off("response", onBResponse);
    await page.unroute("**/*").catch(() => undefined);
    await pageB.unroute("**/*").catch(() => undefined);
    await contextB.close();
  }
});

async function launchInstalledSession(page: import("@playwright/test").Page, session: SessionChatConcurrentFixture) {
  await signIn(page, "reviewer");
  await page.goto(`/repositories/${session.repository_id}`);
  await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
  const card = page.locator(`#installed-ui-${session.installation_id}`);
  await expect(card).toBeVisible();
  const documentResponse = page.waitForResponse(response => {
    try {
      const url = new URL(response.url());
      return url.hostname.startsWith("g-") && url.pathname === session.ui_path &&
        response.request().resourceType() === "document";
    } catch {
      return false;
    }
  }, {timeout: 60_000});
  const contextResponse = page.waitForResponse(response => {
    try {
      return new URL(response.url()).pathname === "/_heph/ui-context" &&
        response.request().resourceType() === "fetch";
    } catch {
      return false;
    }
  }, {timeout: 60_000});
  const discoveryResponse = page.waitForResponse(response => {
    try {
      return new URL(response.url()).pathname.endsWith("/info/refs") &&
        response.request().resourceType() === "fetch";
    } catch {
      return false;
    }
  }, {timeout: 60_000});
  void documentResponse.catch(() => undefined);
  void contextResponse.catch(() => undefined);
  void discoveryResponse.catch(() => undefined);
  await card.getByRole("button", {name: /Launch/}).click();
  const document = await documentResponse;
  expect(document.status()).toBe(200);
  assertCookieIsolation(await document.request().allHeaders());
  await expect.poll(() => new URL(page.url()).pathname).toBe(session.ui_path);
  await expect(page).toHaveTitle("Session chat");
  const context = await contextResponse;
  expect(context.status()).toBe(200);
  expect(await context.json()).toEqual({repository_id: session.repository_id});
  const discovery = await discoveryResponse;
  expect(discovery.status()).toBe(200);
  expect(discovery.headers()["heph-git-actor-id"]).toBe(session.actor_id);
  assertCookieIsolation(await discovery.request().allHeaders());
  await expect(page.locator("[data-status]")).toHaveText(`Connected as user:${session.actor_id}`);
}

async function submitMessage(page: import("@playwright/test").Page, message: string) {
  const receiveResponse = page.waitForResponse(response => {
    try {
      return new URL(response.url()).pathname.endsWith("/git-receive-pack") &&
        response.request().method() === "POST";
    } catch {
      return false;
    }
  }, {timeout: 60_000});
  await page.locator("textarea#message").fill(message);
  await page.getByRole("button", {name: "Send"}).click();
  const receive = await receiveResponse;
  expect(receive.status()).toBe(200);
  assertCookieIsolation(await receive.request().allHeaders());
}

function receiveCommand(request: import("@playwright/test").Request): ReceiveCommand | undefined {
  try {
    const url = new URL(request.url());
    if (!url.pathname.endsWith("/git-receive-pack") || request.method() !== "POST") return undefined;
    const body = request.postDataBuffer();
    if (!body) return undefined;
    return parseReceiveCommand(body);
  } catch {
    // Malformed or non-Git requests provide no command evidence.
    return undefined;
  }
}

async function reconnect(page: import("@playwright/test").Page) {
  const discovery = page.waitForResponse(response => {
    try {
      return new URL(response.url()).pathname.endsWith("/info/refs") &&
        response.request().resourceType() === "fetch";
    } catch {
      return false;
    }
  }, {timeout: 60_000});
  const fetch = page.waitForResponse(response => {
    try {
      return new URL(response.url()).pathname.endsWith("/git-upload-pack") &&
        response.request().method() === "POST";
    } catch {
      return false;
    }
  }, {timeout: 60_000});
  await page.getByRole("button", {name: "Reconnect"}).click();
  expect((await discovery).status()).toBe(200);
  expect((await fetch).status()).toBe(200);
}

async function assertFinalTranscript(
  page: import("@playwright/test").Page,
  transcript: import("@playwright/test").Locator,
  prefix: string[],
  initialTranscriptCount: number,
  initialAgentCount: number,
  firstMessage: string,
  secondMessage: string,
  responseText: string,
) {
  await expect(transcript).toHaveCount(initialTranscriptCount + 4);
  expect((await transcript.allTextContents()).map(text => text.trim()).slice(0, initialTranscriptCount)).toEqual(prefix);
  await expect(transcript.nth(initialTranscriptCount)).toHaveClass(/message-human/);
  await expect(transcript.nth(initialTranscriptCount)).toContainText(firstMessage);
  await expect(transcript.nth(initialTranscriptCount + 1)).toHaveClass(/message-agent/);
  await expect(transcript.nth(initialTranscriptCount + 1)).toContainText(responseText);
  await expect(transcript.nth(initialTranscriptCount + 2)).toHaveClass(/message-human/);
  await expect(transcript.nth(initialTranscriptCount + 2)).toContainText(secondMessage);
  await expect(transcript.nth(initialTranscriptCount + 3)).toHaveClass(/message-agent/);
  await expect(transcript.nth(initialTranscriptCount + 3)).toContainText(responseText);
  await expect(page.locator("article.message-agent")).toHaveCount(initialAgentCount + 2);
  await expect(page.locator("article.message-human").filter({hasText: firstMessage})).toHaveCount(1);
  await expect(page.locator("article.message-human").filter({hasText: secondMessage})).toHaveCount(1);
}

function loadFixture(): SessionChatConcurrentFixture {
  if (!fixturePath) throw new Error("HEPHAESTUS_COOKING_BROWSER_FIXTURE is required");
  const fixture = JSON.parse(readFileSync(fixturePath, "utf8")) as Partial<CookingFixture>;
  const session = fixture.session_chat_concurrent;
  if (!session || !session.project_id || !session.repository_id || !session.installation_id ||
      !session.generation_id || !session.actor_id || !session.ui_path || !session.agent_response_text ||
      !session.initial_transcript_count || !session.initial_agent_count) {
    throw new Error("session-chat concurrent fixture is missing its installed UI projection");
  }
  if (!/^\/[^?]*\.html$/.test(session.ui_path)) throw new Error("session-chat fixture UI path is invalid");
  parseCount(session.initial_transcript_count);
  parseCount(session.initial_agent_count);
  return session;
}

function parseCount(value: string): number {
  if (!/^[0-9]+$/.test(value)) throw new Error("session-chat fixture count is invalid");
  const count = Number(value);
  if (!Number.isSafeInteger(count)) throw new Error("session-chat fixture count is too large");
  return count;
}

async function signIn(page: import("@playwright/test").Page, account: string) {
  await page.goto("/");
  await expect(page.getByTestId("oidc-login")).toBeVisible();
  await page.getByTestId("oidc-login").click();
  await expect(page).toHaveURL(new RegExp(escapeRegExp(`${oidcUrl}/authorize`)));
  await page.locator('input[name="login"]').fill(account);
  await page.getByRole("button", {name: /Continue as/}).click();
  await expect(page).toHaveURL(/\/organizations$/);
  await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function assertCookieIsolation(headers: Record<string, string>) {
  const cookie = headers.cookie ?? "";
  const hasUiCookie = cookie.split(";").some(value => value.trim().startsWith("__Host-hephaestus_ui="));
  const hasPlatformCookie = cookie.split(";").some(value => value.trim().startsWith("__Host-hephaestus_web_key="));
  expect(hasUiCookie && !hasPlatformCookie).toBe(true);
}
