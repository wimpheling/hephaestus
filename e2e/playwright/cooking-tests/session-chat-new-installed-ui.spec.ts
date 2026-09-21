import {expect, test} from "@playwright/test";
import {readFileSync} from "node:fs";

type SessionChatNewFixture = {
  project_id: string;
  release_agent_id: string;
  model_import_id: string;
  agent_response_text: string;
  ui_path: string;
};

type CookingFixture = {
  session_chat_new: SessionChatNewFixture;
};

const fixturePath = process.env.HEPHAESTUS_COOKING_BROWSER_FIXTURE;
const oidcUrl = process.env.HEPHAESTUS_OIDC_URL ?? "http://127.0.0.1:5556";

test("cooking new session chat creates and opens a real Git-backed browser session", async ({page}, testInfo) => {
  test.setTimeout(240_000);
  const fixture = loadFixture();
  const session = fixture.session_chat_new;

  let repositoryId = "";
  await test.step("session-chat-initialize", async () => {
    await signIn(page, "reviewer");
    await page.goto(`/projects/${session.project_id}/session-chat/new`);
    await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
    await expect(page.locator("#create-session-chat-form")).toBeVisible();

    const repositoryName = `session-chat-browser-${testInfo.workerIndex}-${Date.now()}`;
    await page.getByLabel("Repository name").fill(repositoryName);
    await page.getByLabel("Default branch").fill("main");
    await page.getByLabel("Session name").fill("Browser session chat");
    await page.getByLabel("Reference release").selectOption(session.release_agent_id);
    await page.getByLabel("Authorized model import").selectOption(session.model_import_id);
    const acknowledgement = page.getByLabel(/Allow the release-owned UI to access this repository/);
    await acknowledgement.check();
    await expect(acknowledgement).toBeChecked();

    const documentResponse = page.waitForResponse(response => {
      try {
        const url = new URL(response.url());
        return url.hostname.startsWith("g-") &&
          url.pathname === session.ui_path &&
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
        const url = new URL(response.url());
        return url.pathname.endsWith("/info/refs") &&
          response.request().resourceType() === "fetch";
      } catch {
        return false;
      }
    }, {timeout: 60_000});
    await page.getByRole("button", {name: "Create and open chat"}).click();

    const document = await documentResponse;
    expect(document.status()).toBe(200);
    assertCookieIsolation(await document.request().allHeaders());
    await expect.poll(() => new URL(page.url()).pathname, {timeout: 60_000}).toBe(session.ui_path);
    await expect(page).toHaveTitle("Session chat");

    const context = await contextResponse;
    expect(context.status()).toBe(200);
    expect(context.headers()["cache-control"]).toBe("no-store");
    const contextBody = await context.json() as {repository_id?: string};
    expect(contextBody).toEqual({repository_id: expect.stringMatching(CANONICAL_UUID)});
    repositoryId = contextBody.repository_id as string;

    const discovery = await discoveryResponse;
    expect(discovery.status()).toBe(200);
    const actorId = discovery.headers()["heph-git-actor-id"];
    expect(actorId).toMatch(CANONICAL_UUID);
    assertCookieIsolation(await discovery.request().allHeaders());
    await expect(page.locator("[data-status]")).toHaveText(`Connected as user:${actorId}`);
    await expect(page.locator("[data-transcript] article")).toHaveCount(0);
  });

  let pollingFetches = 0;
  const onRequest = (request: import("@playwright/test").Request) => {
    try {
      const url = new URL(request.url());
      if (request.method() === "POST" && url.pathname.endsWith("/git-upload-pack")) pollingFetches += 1;
    } catch {
      // Ignore requests outside the Git route.
    }
  };
  page.on("request", onRequest);
  const message = `browser new session message ${testInfo.workerIndex}`;
  const agentMessage = page.locator("article.message-agent").filter({hasText: session.agent_response_text});
  await test.step("session-chat-send", async () => {
    const receiveResponse = page.waitForResponse(response => {
      try {
        const url = new URL(response.url());
        return url.pathname.endsWith("/git-receive-pack") && response.request().method() === "POST";
      } catch {
        return false;
      }
    }, {timeout: 60_000});
    await page.locator("textarea#message").fill(message);
    await page.getByRole("button", {name: "Send"}).click();
    const receive = await receiveResponse;
    expect(receive.status()).toBe(200);
    assertCookieIsolation(await receive.request().allHeaders());
    await expect(page.locator("article.message-human").filter({hasText: message})).toHaveCount(1);
  });
  await test.step("session-chat-response", async () => {
    await expect(agentMessage).toHaveCount(1, {timeout: 60_000});
    await expect(page.locator("[data-status]")).toHaveText("Assistant response received");
    await expect(page.locator("[data-transcript] article")).toHaveCount(2);
  });
  page.off("request", onRequest);
  expect(pollingFetches).toBeGreaterThan(0);

  const secondMessage = `browser second session message ${testInfo.workerIndex}`;
  page.on("request", onRequest);
  await test.step("session-chat-second-send", async () => {
    const receiveResponse = page.waitForResponse(response => {
      try {
        const url = new URL(response.url());
        return url.pathname.endsWith("/git-receive-pack") && response.request().method() === "POST";
      } catch {
        return false;
      }
    }, {timeout: 60_000});
    await page.locator("textarea#message").fill(secondMessage);
    await page.getByRole("button", {name: "Send"}).click();
    const receive = await receiveResponse;
    expect(receive.status()).toBe(200);
    assertCookieIsolation(await receive.request().allHeaders());
    await expect(page.locator("article.message-human").filter({hasText: secondMessage})).toHaveCount(1);
  });
  await test.step("session-chat-second-response", async () => {
    await expect(agentMessage).toHaveCount(2, {timeout: 60_000});
    await expect(page.locator("[data-status]")).toHaveText("Assistant response received");
    await expectTranscript(page, message, secondMessage, session.agent_response_text);
  });
  page.off("request", onRequest);
  expect(pollingFetches).toBeGreaterThan(1);

  await test.step("session-chat-reconnect", async () => {
    const reconnectDiscovery = page.waitForResponse(response => {
      try {
        return new URL(response.url()).pathname.endsWith("/info/refs") &&
          response.request().resourceType() === "fetch";
      } catch {
        return false;
      }
    }, {timeout: 60_000});
    const reconnectFetch = page.waitForResponse(response => {
      try {
        return new URL(response.url()).pathname.endsWith("/git-upload-pack") &&
          response.request().method() === "POST";
      } catch {
        return false;
      }
    }, {timeout: 60_000});
    await page.getByRole("button", {name: "Reconnect"}).click();
    expect((await reconnectDiscovery).status()).toBe(200);
    expect((await reconnectFetch).status()).toBe(200);
    await expectTranscript(page, message, secondMessage, session.agent_response_text);
    expect(repositoryId).toMatch(CANONICAL_UUID);
  });
});

const CANONICAL_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

async function expectTranscript(
  page: import("@playwright/test").Page,
  firstMessage: string,
  secondMessage: string,
  agentResponse: string,
) {
  const transcript = page.locator("[data-transcript] article");
  await expect(transcript).toHaveCount(4);
  await expect(transcript.nth(0)).toHaveClass(/message-human/);
  await expect(transcript.nth(0)).toContainText(firstMessage);
  await expect(transcript.nth(1)).toHaveClass(/message-agent/);
  await expect(transcript.nth(1)).toContainText(agentResponse);
  await expect(transcript.nth(2)).toHaveClass(/message-human/);
  await expect(transcript.nth(2)).toContainText(secondMessage);
  await expect(transcript.nth(3)).toHaveClass(/message-agent/);
  await expect(transcript.nth(3)).toContainText(agentResponse);
  await expect(page.locator("[data-transcript] article.message-human").filter({hasText: firstMessage})).toHaveCount(1);
  await expect(page.locator("[data-transcript] article.message-agent")).toHaveCount(2);
}

function loadFixture(): CookingFixture {
  if (!fixturePath) throw new Error("HEPHAESTUS_COOKING_BROWSER_FIXTURE is required");
  const fixture = JSON.parse(readFileSync(fixturePath, "utf8")) as CookingFixture;
  const session = fixture.session_chat_new;
  if (!session) throw new Error("fixture session_chat_new is required for the new-session acceptance");
  const required = [
    "project_id",
    "release_agent_id",
    "model_import_id",
    "agent_response_text",
    "ui_path",
  ] as const;
  if (required.some(key => typeof session[key] !== "string" || !session[key])) {
    throw new Error("session-chat new browser fixture is missing its creation inputs");
  }
  if (!/^\/[^?]*\.html$/.test(session.ui_path)) throw new Error("session-chat fixture UI path is invalid");
  return fixture;
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
