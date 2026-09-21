import {expect, test} from "@playwright/test";
import {readFileSync} from "node:fs";

type SessionChatForkFixture = {
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
  session_chat_fork: SessionChatForkFixture;
};

const fixturePath = process.env.HEPHAESTUS_COOKING_BROWSER_FIXTURE;
const oidcUrl = process.env.HEPHAESTUS_OIDC_URL ?? "http://127.0.0.1:5556";

test("cooking forked session chat preserves inherited history and receives a fresh response", async ({page}, testInfo) => {
  test.setTimeout(240_000);
  const session = loadFixture();
  const initialTranscriptCount = parseCount(session.initial_transcript_count);
  const initialAgentCount = parseCount(session.initial_agent_count);
  const transcript = page.locator("[data-transcript] article");
  let inheritedPrefix: string[] = [];
  let uploadPackRequests = 0;
  const onRequest = (request: import("@playwright/test").Request) => {
    try {
      const url = new URL(request.url());
      if (request.method() === "POST" && url.pathname.endsWith("/git-upload-pack")) {
        uploadPackRequests += 1;
      }
    } catch {
      // Ignore requests outside the installed Git UI.
    }
  };
  page.on("request", onRequest);

  try {
    await test.step("session-chat-fork-initialize", async () => {
      await launchInstalledSession(page, session);
      await expect(transcript).toHaveCount(initialTranscriptCount);
      await expect(page.locator("article.message-agent")).toHaveCount(initialAgentCount);
      inheritedPrefix = (await transcript.allTextContents()).map(text => text.trim());
      expect(inheritedPrefix).toHaveLength(initialTranscriptCount);
    });

    const message = `browser fork session message ${testInfo.workerIndex}-${Date.now()}`;
    await test.step("session-chat-fork-send", async () => {
      const receiveResponse = page.waitForResponse(response => {
        try {
          const url = new URL(response.url());
          return url.pathname.endsWith("/git-receive-pack") &&
            response.request().method() === "POST";
        } catch {
          return false;
        }
      }, {timeout: 60_000});
      void receiveResponse.catch(() => undefined);
      await page.locator("textarea#message").fill(message);
      await page.getByRole("button", {name: "Send"}).click();
      const receive = await receiveResponse;
      expect(receive.status()).toBe(200);
      assertCookieIsolation(await receive.request().allHeaders());
      await expect(page.locator("article.message-human").filter({hasText: message})).toHaveCount(1);
    });

    await test.step("session-chat-fork-response", async () => {
      await expect(page.locator("article.message-agent")).toHaveCount(initialAgentCount + 1, {timeout: 60_000});
      await expect(transcript).toHaveCount(initialTranscriptCount + 2);
      expect((await transcript.allTextContents()).map(text => text.trim()).slice(0, initialTranscriptCount))
        .toEqual(inheritedPrefix);
      await expect(transcript.nth(initialTranscriptCount)).toHaveClass(/message-human/);
      await expect(transcript.nth(initialTranscriptCount)).toContainText(message);
      await expect(transcript.nth(initialTranscriptCount + 1)).toHaveClass(/message-agent/);
      await expect(transcript.nth(initialTranscriptCount + 1)).toContainText(session.agent_response_text);
      await expect(page.locator("[data-status]")).toHaveText("Assistant response received");
      await expect(page.locator("article.message-agent").filter({hasText: session.agent_response_text}))
        .toHaveCount(initialAgentCount + 1);
      expect(uploadPackRequests).toBeGreaterThan(0);
    });

    await test.step("session-chat-fork-reconnect", async () => {
      const reconnectDiscovery = page.waitForResponse(response => {
        try {
          return new URL(response.url()).pathname.endsWith("/info/refs") &&
            response.request().resourceType() === "fetch";
        } catch {
          return false;
        }
      }, {timeout: 60_000});
      void reconnectDiscovery.catch(() => undefined);
      const reconnectFetch = page.waitForResponse(response => {
        try {
          return new URL(response.url()).pathname.endsWith("/git-upload-pack") &&
            response.request().method() === "POST";
        } catch {
          return false;
        }
      }, {timeout: 60_000});
      void reconnectFetch.catch(() => undefined);
      await page.getByRole("button", {name: "Reconnect"}).click();
      expect((await reconnectDiscovery).status()).toBe(200);
      expect((await reconnectFetch).status()).toBe(200);
      await expect(transcript).toHaveCount(initialTranscriptCount + 2);
      expect((await transcript.allTextContents()).map(text => text.trim()).slice(0, initialTranscriptCount))
        .toEqual(inheritedPrefix);
      await expect(page.locator("article.message-agent")).toHaveCount(initialAgentCount + 1);
      await expect(page.locator("article.message-human").filter({hasText: message})).toHaveCount(1);
      await expect(transcript.nth(initialTranscriptCount + 1)).toContainText(session.agent_response_text);
    });
  } finally {
    page.off("request", onRequest);
  }
});

async function launchInstalledSession(page: import("@playwright/test").Page, session: SessionChatForkFixture) {
  await signIn(page, "reviewer");
  await page.goto(`/projects/${session.project_id}`);
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
  void documentResponse.catch(() => undefined);
  const contextResponse = page.waitForResponse(response => {
    try {
      return new URL(response.url()).pathname === "/_heph/ui-context" &&
        response.request().resourceType() === "fetch";
    } catch {
      return false;
    }
  }, {timeout: 60_000});
  void contextResponse.catch(() => undefined);
  const discoveryResponse = page.waitForResponse(response => {
    try {
      return new URL(response.url()).pathname.endsWith("/info/refs") &&
        response.request().resourceType() === "fetch";
    } catch {
      return false;
    }
  }, {timeout: 60_000});
  void discoveryResponse.catch(() => undefined);
  await card.getByRole("button", {name: /Launch/}).click();

  const document = await documentResponse;
  expect(document.status()).toBe(200);
  assertCookieIsolation(await document.request().allHeaders());
  await expect(page).toHaveURL(new RegExp(escapeRegExp(session.ui_path) + "$"));
  await expect(page).toHaveTitle("Session chat");

  const context = await contextResponse;
  expect(context.status()).toBe(200);
  expect(context.headers()["cache-control"]).toBe("no-store");
  expect(await context.json()).toEqual({repository_id: session.repository_id});

  const discovery = await discoveryResponse;
  expect(discovery.status()).toBe(200);
  expect(discovery.headers()["heph-git-actor-id"]).toBe(session.actor_id);
  assertCookieIsolation(await discovery.request().allHeaders());
  await expect(page.locator("[data-status]")).toHaveText(`Connected as user:${session.actor_id}`);
}

function loadFixture(): SessionChatForkFixture {
  if (!fixturePath) throw new Error("HEPHAESTUS_COOKING_BROWSER_FIXTURE is required");
  const fixture = JSON.parse(readFileSync(fixturePath, "utf8")) as Partial<CookingFixture>;
  const session = fixture.session_chat_fork;
  if (!session || !session.project_id || !session.repository_id || !session.installation_id ||
      !session.generation_id || !session.actor_id || !session.ui_path || !session.agent_response_text ||
      !session.initial_transcript_count || !session.initial_agent_count) {
    throw new Error("session-chat fork fixture is missing its installed UI projection");
  }
  if (!/^\/[^?]*\.html$/.test(session.ui_path)) throw new Error("session-chat fixture UI path is invalid");
  parseCount(session.initial_transcript_count);
  parseCount(session.initial_agent_count);
  return session as SessionChatForkFixture;
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
