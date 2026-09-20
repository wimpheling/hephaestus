import {expect, test} from "@playwright/test";
import {AxeBuilder} from "@axe-core/playwright";
import {readFileSync} from "node:fs";
import {join} from "node:path";

type CookingFixture = {
  installed_reference_uis: {
    project_id: string;
    static_installation_id: string;
    managed_installation_id: string;
  };
};

const fixturePath = process.env.HEPHAESTUS_COOKING_BROWSER_FIXTURE;
const oidcUrl = process.env.HEPHAESTUS_OIDC_URL ?? "http://127.0.0.1:5556";

test("cooking installed UI TLS full-page and managed iframe smoke", async ({page}) => {
  const fixture = loadFixture();
  const installed = fixture.installed_reference_uis;
  await test.step("signin", async () => {
    await signIn(page, "reviewer");
  });
  await page.goto(`/projects/${installed.project_id}`);
  await waitForLiveView(page);

  const staticCard = page.locator(`#installed-ui-${installed.static_installation_id}`);
  const managedCard = page.locator(`#installed-ui-${installed.managed_installation_id}`);
  await expect(staticCard).toBeVisible();
  await expect(managedCard).toBeVisible();

  let staticUrl = new URL(page.url());
  await test.step("static-launch", async () => {
    await staticCard.getByRole("button", {name: /Launch/}).click();
    await expect.poll(async () => new URL(page.url()).hostname.startsWith("g-"), {timeout: 30_000}).toBe(true);
    await expect.poll(async () => new URL(page.url()).pathname === "/reference/index.html", {timeout: 30_000}).toBe(true);
    staticUrl = new URL(page.url());
    expect(staticUrl.hash === "").toBe(true);
    await expect(page.locator("body")).toContainText("Static release UI");
    await expect(page).toHaveTitle("Release reference");
    await expect(page.locator('link[rel="stylesheet"]')).toHaveAttribute(
      "href",
      /heph-ui-kit-v1\.0\.0\.css/,
    );
  });
  await test.step("static-cookie", async () => {
    const staticCookies = await page.context().cookies();
    const staticUiCookie = staticCookies.find(cookie => cookie.name === "__Host-hephaestus_ui");
    expect(staticUiCookie).toBeDefined();
    expect(staticUiCookie?.secure).toBe(true);
    expect(staticUiCookie?.httpOnly).toBe(true);
    expect(staticUiCookie?.sameSite).toBe("Strict");
    expect(staticUiCookie?.path).toBe("/");
    expect(staticUiCookie?.domain).toBe(staticUrl.hostname);
    const platformCookie = staticCookies.find(cookie => cookie.name === "__Host-hephaestus_web_key");
    expect(platformCookie).toBeDefined();
    expect(platformCookie?.secure).toBe(true);
    expect(platformCookie?.httpOnly).toBe(true);
    expect(platformCookie?.domain.startsWith(".")).toBe(false);
    expect(platformCookie?.domain).toBe(new URL(process.env.HEPHAESTUS_WEB_URL ?? "https://invalid/").hostname);
    const uiCookies = await page.context().cookies(staticUrl.origin);
    expect(uiCookies.some(cookie => cookie.name === "__Host-hephaestus_web_key")).toBe(false);
  });
  await page.screenshot({path: screenshotPath("installed-ui-static-full-page.png"), fullPage: true});

  await page.goto(`/projects/${installed.project_id}`);
  await waitForLiveView(page);
  const managedCardAfterReturn = page.locator(`#installed-ui-${installed.managed_installation_id}`);
  await expect(managedCardAfterReturn).toBeVisible();
  const frame = page.locator("#installed-ui-frame-project");
  const embed = page.locator("#installed-ui-embed-project");
  const frameContent = frame.contentFrame();
  await test.step("managed-launch", async () => {
    const managedDocument = page.waitForResponse(response => {
      try {
        const url = new URL(response.url());
        return url.hostname.startsWith("g-") &&
          url.pathname === "/managed-reference/index.html" &&
          response.request().resourceType() === "document" &&
          response.status() === 200;
      } catch {
        return false;
      }
    }, {timeout: 30_000});
    await managedCardAfterReturn.getByRole("button", {name: /Launch/}).click();
    await expect(frame).toBeVisible();
    await expect.poll(async () => {
      const frameHandle = await frame.elementHandle();
      const childFrame = frameHandle ? await frameHandle.contentFrame() : null;
      if (!childFrame) return false;
      const frameUrl = new URL(childFrame.url());
      return /^g-[0-9a-f]{32}\./.test(frameUrl.hostname) && frameUrl.pathname === "/managed-reference/index.html";
    }, {timeout: 30_000}).toBe(true);
    await expect(frameContent.getByText("Managed service UI")).toBeVisible();
    await expect(frameContent.getByRole("heading", {name: "Managed release reference"})).toBeVisible();
    await expect(frameContent.locator('link[rel="stylesheet"]')).toHaveCount(1);
    const response = await managedDocument;
    const headers = response.headers();
    const platformOrigin = new URL(process.env.HEPHAESTUS_WEB_URL ?? "https://invalid.example/").origin;
    const contentSecurityPolicy = headers["content-security-policy"] ?? "";
    expect(contentSecurityPolicy.includes("default-src 'none'")).toBe(true);
    expect(contentSecurityPolicy.includes("connect-src 'self'")).toBe(true);
    expect(contentSecurityPolicy.includes("form-action 'none'")).toBe(true);
    expect(contentSecurityPolicy.includes("frame-src 'none'")).toBe(true);
    expect(contentSecurityPolicy.includes(`frame-ancestors ${platformOrigin}`)).toBe(true);
    expect(headers["x-content-type-options"] === "nosniff").toBe(true);
  });
  await test.step("managed-cookie", async () => {
    const frameHandle = await frame.elementHandle();
    const managedFrame = frameHandle ? await frameHandle.contentFrame() : null;
    expect(managedFrame).toBeTruthy();
    const managedFrameUrl = new URL(managedFrame?.url() ?? "https://invalid.example/");
    const managedCookies = await page.context().cookies(managedFrameUrl.origin);
    const managedUiCookie = managedCookies.find(cookie => cookie.name === "__Host-hephaestus_ui");
    expect(managedUiCookie).toBeDefined();
    expect(managedUiCookie?.secure).toBe(true);
    expect(managedUiCookie?.httpOnly).toBe(true);
    expect(managedUiCookie?.sameSite).toBe("Strict");
    expect(managedUiCookie?.path).toBe("/");
    expect(managedUiCookie?.domain).toBe(managedFrameUrl.hostname);
  });
  await test.step("theme", async () => {
    await page.getByRole("button", {name: "Use dark theme"}).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect.poll(
      () => frameContent.locator("html").getAttribute("data-theme"),
      {timeout: 10_000},
    ).toBe("dark");

    await page.getByRole("button", {name: "Use light theme"}).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect.poll(
      () => frameContent.locator("html").getAttribute("data-theme"),
      {timeout: 10_000},
    ).toBe("light");

    await page.getByRole("button", {name: "Use system theme"}).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme-source", "system");
    await page.emulateMedia({colorScheme: "dark"});
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect.poll(
      () => frameContent.locator("html").getAttribute("data-theme"),
      {timeout: 10_000},
    ).toBe("dark");
  });
  await test.step("accessibility", async () => {
    await expect(embed.locator("[data-ui-status]")).toHaveAttribute("role", "status");
    await expect(embed.locator("[data-ui-status]")).toHaveAttribute("aria-live", "polite");
    await expect(embed.getByRole("button", {name: "Close"})).toBeVisible();
    await expect(frame).toHaveAttribute("title", "Installed UI");
    await expect(frame).toHaveAttribute("sandbox", "allow-scripts allow-same-origin");
    await expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");

    const platformPolicy = await page.locator(
      'meta[http-equiv="Content-Security-Policy"]',
    ).getAttribute("content");
    const uiNamespace = process.env.HEPHAESTUS_UI_NAMESPACE ?? "invalid.example";
    expect(platformPolicy?.includes("frame-src 'self'") ?? false).toBe(true);
    expect(platformPolicy?.includes(`https://*.${uiNamespace}`) ?? false).toBe(true);

    const undeclaredFetchBlocked = await frameContent.locator("body").evaluate(() => new Promise(resolve => {
      const blockedUrl = "https://example.invalid/heph-ui-csp-probe";
      const timeout = window.setTimeout(() => {
        window.removeEventListener("securitypolicyviolation", onViolation);
        resolve(false);
      }, 1_000);
      const onViolation = (event: SecurityPolicyViolationEvent) => {
        const blockedOrigin = new URL(blockedUrl).origin;
        const matchesTarget = event.blockedURI === blockedOrigin || event.blockedURI.startsWith(blockedUrl);
        if (event.effectiveDirective !== "connect-src" || !matchesTarget) return;
        window.clearTimeout(timeout);
        window.removeEventListener("securitypolicyviolation", onViolation);
        resolve(true);
      };
      window.addEventListener("securitypolicyviolation", onViolation);
      void fetch(blockedUrl, {cache: "no-store"}).catch(() => undefined);
    }));
    expect(undeclaredFetchBlocked).toBe(true);

    const platformUrlBeforeParentNavigation = page.url();
    const parentNavigationBlocked = await frameContent.locator("body").evaluate(() => {
      try {
        const topWindow = window.top;
        if (!topWindow) return false;
        topWindow.location.href = "https://example.invalid/heph-ui-parent-probe";
        return false;
      } catch (error) {
        return error instanceof DOMException && error.name === "SecurityError";
      }
    });
    expect(parentNavigationBlocked).toBe(true);
    expect(page.url()).toBe(platformUrlBeforeParentNavigation);

    const axe = await new AxeBuilder({page}).include("#installed-ui-navigation-project").analyze();
    expect(axe.violations.length).toBe(0);
    const frameAxe = await new AxeBuilder({page}).include("#installed-ui-frame-project").analyze();
    expect(frameAxe.violations.length).toBe(0);
  });
  await test.step("managed-identity", async () => {
    const identityIsValid = await frameContent.locator("body").evaluate(async () => {
      const response = await fetch("/reference/identity", {
        headers: {accept: "application/json"},
        credentials: "same-origin",
      });
      if (!response.ok) return false;
      const body: unknown = await response.json();
      if (!body || typeof body !== "object") return false;
      const value = body as {pid?: unknown; startup_id?: unknown};
      const keys = Object.keys(value).sort();
      return keys.length === 2 && keys[0] === "pid" && keys[1] === "startup_id" &&
        typeof value.pid === "number" && Number.isInteger(value.pid) && value.pid > 0 &&
        typeof value.startup_id === "string" && value.startup_id.length > 0;
    });
    expect(identityIsValid).toBe(true);
  });
  await expect(page.locator("#installed-ui-terminal-status-project")).toBeHidden();
  await page.screenshot({path: screenshotPath("installed-ui-managed-iframe.png"), fullPage: true});
  await test.step("close", async () => {
    await embed.getByRole("button", {name: "Close"}).click();
    await expect(embed).toBeHidden();
    await expect(frame).not.toHaveAttribute("src");
    await expect(embed.locator("[data-ui-status]")).toHaveText("Closed");
    await expect(page.locator("#installed-ui-terminal-status-project")).toBeHidden();
    const launchButton = managedCardAfterReturn.getByRole("button", {name: /Launch/});
    const launchButtonFocused = await launchButton.evaluate(element => document.activeElement === element);
    expect(launchButtonFocused).toBe(true);
  });
});

function screenshotPath(name: string): string {
  const directory = process.env.HEPHAESTUS_SAFE_SCREENSHOT_DIR;
  if (!directory?.startsWith("/")) throw new Error("HEPHAESTUS_SAFE_SCREENSHOT_DIR is required");
  return join(directory, name);
}

function loadFixture(): CookingFixture {
  if (!fixturePath) throw new Error("HEPHAESTUS_COOKING_BROWSER_FIXTURE is required");
  const fixture = JSON.parse(readFileSync(fixturePath, "utf8")) as Partial<CookingFixture>;
  const installed = fixture.installed_reference_uis;
  for (const key of ["project_id", "static_installation_id", "managed_installation_id"] as const) {
    if (!installed?.[key]) throw new Error(`installed UI fixture is missing installed_reference_uis.${key}`);
  }
  return {installed_reference_uis: installed as CookingFixture["installed_reference_uis"]};
}

async function signIn(page: import("@playwright/test").Page, account: string) {
  await page.goto("/");
  await page.getByTestId("oidc-login").click();
  await expect(page).toHaveURL(new RegExp(escapeRegExp(`${oidcUrl}/authorize`)));
  await page.locator('input[name="login"]').fill(account);
  await page.getByRole("button", {name: /Continue as/}).click();
  await expect(page).toHaveURL(/\/organizations$/);
  await waitForLiveView(page);
}

async function waitForLiveView(page: import("@playwright/test").Page) {
  await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
