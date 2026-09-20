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
const webUrl = process.env.HEPHAESTUS_WEB_URL;
const uiNamespace = process.env.HEPHAESTUS_UI_NAMESPACE;
if (!webUrl) throw new Error("HEPHAESTUS_WEB_URL is required");
if (!uiNamespace) throw new Error("HEPHAESTUS_UI_NAMESPACE is required");

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
  let staticDocumentSentUiCookie = false;
  let staticDocumentSentPlatformCookie = false;
  await test.step("static-launch", async () => {
    const staticDocument = page.waitForResponse(response => {
      try {
        const url = new URL(response.url());
        return url.hostname.startsWith("g-") &&
          url.pathname === "/reference/index.html" &&
          response.request().resourceType() === "document" &&
          response.status() === 200;
      } catch {
        return false;
      }
    }, {timeout: 30_000});
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
    const response = await staticDocument;
    const requestHeaders = await response.request().allHeaders();
    staticDocumentSentUiCookie = requestHasCookie(requestHeaders, "__Host-hephaestus_ui");
    staticDocumentSentPlatformCookie = requestHasCookie(requestHeaders, "__Host-hephaestus_web_key");
    const headers = response.headers();
    const platformOrigin = new URL(webUrl).origin;
    const contentSecurityPolicy = headers["content-security-policy"] ?? "";
    expect(contentSecurityPolicy.includes(`frame-ancestors ${platformOrigin}`)).toBe(true);
    expect(headers["x-content-type-options"] === "nosniff").toBe(true);
  });
  await test.step("static-ui-cookie-presence", async () => {
    const staticCookies = await page.context().cookies();
    expect(staticCookies.some(cookie => cookie.name === "__Host-hephaestus_ui")).toBe(true);
  });
  await test.step("static-ui-cookie-secure", async () => {
    const staticUiCookie = (await page.context().cookies()).find(cookie => cookie.name === "__Host-hephaestus_ui");
    expect(staticUiCookie?.secure === true).toBe(true);
  });
  await test.step("static-ui-cookie-http-only", async () => {
    const staticUiCookie = (await page.context().cookies()).find(cookie => cookie.name === "__Host-hephaestus_ui");
    expect(staticUiCookie?.httpOnly === true).toBe(true);
  });
  await test.step("static-ui-cookie-same-site", async () => {
    const staticUiCookie = (await page.context().cookies()).find(cookie => cookie.name === "__Host-hephaestus_ui");
    expect(staticUiCookie?.sameSite === "Strict").toBe(true);
  });
  await test.step("static-ui-cookie-path", async () => {
    const staticUiCookie = (await page.context().cookies()).find(cookie => cookie.name === "__Host-hephaestus_ui");
    expect(staticUiCookie?.path === "/").toBe(true);
  });
  await test.step("static-ui-cookie-host-domain", async () => {
    const staticUiCookie = (await page.context().cookies()).find(cookie => cookie.name === "__Host-hephaestus_ui");
    expect(staticUiCookie !== undefined && staticUiCookie.domain === staticUrl.hostname).toBe(true);
  });
  await test.step("static-platform-cookie-presence", async () => {
    const staticCookies = await page.context().cookies();
    expect(staticCookies.some(cookie => cookie.name === "__Host-hephaestus_web_key")).toBe(true);
  });
  await test.step("static-platform-cookie-security", async () => {
    const platformCookie = (await page.context().cookies()).find(cookie => cookie.name === "__Host-hephaestus_web_key");
    expect(platformCookie?.secure === true && platformCookie?.httpOnly === true).toBe(true);
  });
  await test.step("static-platform-cookie-host-domain", async () => {
    const platformCookie = (await page.context().cookies()).find(cookie => cookie.name === "__Host-hephaestus_web_key");
    const platformHostname = new URL(webUrl).hostname;
    expect(platformCookie !== undefined && !platformCookie.domain.startsWith(".") && platformCookie.domain === platformHostname).toBe(true);
  });
  await test.step("static-cookie-isolation", async () => {
    // Context inventory filtering does not preserve host-only semantics; the
    // actual document request headers above are the isolation proof.
    expect(staticDocumentSentUiCookie).toBe(true);
    expect(staticDocumentSentPlatformCookie).toBe(false);
  });
  await test.step("static-accessibility", async () => {
    const staticAxe = await new AxeBuilder({page}).analyze();
    expect(staticAxe.violations.length).toBe(0);
  });
  await test.step("static-theme", async () => {
    const directStaticUrl = new URL(staticUrl.toString());
    directStaticUrl.search = "";
    directStaticUrl.hash = "";

    await page.emulateMedia({colorScheme: "dark"});
    await page.goto(directStaticUrl.toString());
    await expect(page.locator("body")).toContainText("Static release UI");
    expect(await page.locator("html").getAttribute("data-theme")).toBeNull();
    const darkPaper = await page.locator("body").evaluate(element => getComputedStyle(element).backgroundColor);

    await page.emulateMedia({colorScheme: "light"});
    await expect(page.locator("body")).toContainText("Static release UI");
    expect(await page.locator("html").getAttribute("data-theme")).toBeNull();
    const lightPaper = await page.locator("body").evaluate(element => getComputedStyle(element).backgroundColor);
    expect(darkPaper !== lightPaper).toBe(true);
  });
  await page.screenshot({path: screenshotPath("installed-ui-static-full-page.png"), fullPage: true});

  await page.goto(`/projects/${installed.project_id}`);
  await waitForLiveView(page);
  const managedCardAfterReturn = page.locator(`#installed-ui-${installed.managed_installation_id}`);
  await expect(managedCardAfterReturn).toBeVisible();
  const frame = page.locator("#installed-ui-frame-project");
  const embed = page.locator("#installed-ui-embed-project");
  const frameContent = frame.contentFrame();
  let managedDocumentSentUiCookie = false;
  let managedDocumentSentPlatformCookie = false;
  let managedDocumentStatusClass = 0;
  let managedDocumentStatusCode = 0;
  let managedDocumentHeaders: Record<string, string> = {};
  let managedFrameCspViolation = false;
  await page.evaluate(() => {
    const state = window as typeof window & {__hephManagedFrameCspViolation?: boolean};
    state.__hephManagedFrameCspViolation = false;
    window.addEventListener("securitypolicyviolation", (event: SecurityPolicyViolationEvent) => {
      if (event.effectiveDirective === "frame-src" || event.effectiveDirective === "child-src") {
        state.__hephManagedFrameCspViolation = true;
      }
    });
  });
  const managedDocument = page.waitForResponse(response => {
    try {
      const url = new URL(response.url());
      return url.hostname.startsWith("g-") &&
        url.pathname === "/managed-reference/index.html" &&
        response.request().resourceType() === "document";
    } catch {
      return false;
    }
  }, {timeout: 30_000}).catch(() => null);
  await test.step("managed-launch", async () => {
    await test.step("managed-card-click", async () => {
      await managedCardAfterReturn.getByRole("button", {name: /Launch/}).click();
    });
    await test.step("managed-frame-visible", async () => {
      await expect(frame).toBeVisible();
    });
    try {
      await test.step("managed-frame-route", async () => {
        await expect.poll(async () => {
          const frameHandle = await frame.elementHandle();
          const childFrame = frameHandle ? await frameHandle.contentFrame() : null;
          if (!childFrame) return false;
          const frameUrl = new URL(childFrame.url());
          return /^g-[0-9a-f]{32}\./.test(frameUrl.hostname) && frameUrl.pathname === "/managed-reference/index.html";
        }, {timeout: 30_000}).toBe(true);
      });
    } finally {
      await test.step("managed-frame-csp", async () => {
        managedFrameCspViolation = await page.evaluate(() => Boolean(
          (window as typeof window & {__hephManagedFrameCspViolation?: boolean}).__hephManagedFrameCspViolation,
        ));
        expect(managedFrameCspViolation).toBe(false);
      });
    }
    await test.step("managed-document-response", async () => {
      const response = await managedDocument;
      expect(response).not.toBeNull();
      if (response === null) return;
      managedDocumentStatusCode = response.status();
      managedDocumentStatusClass = Math.floor(response.status() / 100);
      managedDocumentHeaders = response.headers();
      const requestHeaders = await response.request().allHeaders();
      managedDocumentSentUiCookie = requestHasCookie(requestHeaders, "__Host-hephaestus_ui");
      managedDocumentSentPlatformCookie = requestHasCookie(requestHeaders, "__Host-hephaestus_web_key");
    });
    const statusStage = managedDocumentStatusClass === 2 ? "managed-document-2xx" :
      managedDocumentStatusClass === 3 ? "managed-document-3xx" :
        managedDocumentStatusClass === 4 ? "managed-document-4xx" :
          managedDocumentStatusCode === 500 ? "managed-document-500" :
            managedDocumentStatusCode === 502 ? "managed-document-502" :
              managedDocumentStatusCode === 503 ? "managed-document-503" :
                managedDocumentStatusCode === 504 ? "managed-document-504" :
                  managedDocumentStatusClass === 5 ? "managed-document-5xx-other" : "managed-document-other";
    await test.step(statusStage, async () => {
      expect(managedDocumentStatusClass).toBe(2);
    });
    await test.step("managed-document-headers", async () => {
      const headers = managedDocumentHeaders;
      const platformOrigin = new URL(webUrl).origin;
      const contentSecurityPolicy = headers["content-security-policy"] ?? "";
      expect(contentSecurityPolicy.includes("default-src 'none'")).toBe(true);
      expect(contentSecurityPolicy.includes("connect-src 'self'")).toBe(true);
      expect(contentSecurityPolicy.includes("form-action 'none'")).toBe(true);
      expect(contentSecurityPolicy.includes("frame-src 'none'")).toBe(true);
      expect(contentSecurityPolicy.includes(`frame-ancestors ${platformOrigin}`)).toBe(true);
      expect(headers["x-content-type-options"] === "nosniff").toBe(true);
    });
    await test.step("managed-document-content", async () => {
      await expect(frameContent.getByText("Managed service UI")).toBeVisible();
      await expect(frameContent.getByRole("heading", {name: "Managed release reference"})).toBeVisible();
      await expect(frameContent.locator('link[rel="stylesheet"]')).toHaveCount(1);
    });
  });
  await test.step("managed-cookie", async () => {
    const frameHandle = await frame.elementHandle();
    const managedFrame = frameHandle ? await frameHandle.contentFrame() : null;
    expect(managedFrame).toBeTruthy();
    const managedFrameUrl = new URL(managedFrame?.url() ?? "https://invalid.example/");
    const managedCookies = await page.context().cookies(managedFrameUrl.origin);
    const managedUiCookie = managedCookies.find(cookie =>
      cookie.name === "__Host-hephaestus_ui" && cookie.domain === managedFrameUrl.hostname);
    expect(managedUiCookie !== undefined).toBe(true);
    expect(managedUiCookie?.secure === true).toBe(true);
    expect(managedUiCookie?.httpOnly === true).toBe(true);
    expect(managedUiCookie?.sameSite === "Strict").toBe(true);
    expect(managedUiCookie?.path === "/").toBe(true);
    expect(managedUiCookie?.domain === managedFrameUrl.hostname).toBe(true);
  });
  await test.step("managed-cookie-isolation", async () => {
    expect(managedDocumentSentUiCookie).toBe(true);
    expect(managedDocumentSentPlatformCookie).toBe(false);
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
    await test.step("accessibility-status", async () => {
      await expect(embed.locator("[data-ui-status]")).toHaveAttribute("role", "status");
      await expect(embed.locator("[data-ui-status]")).toHaveAttribute("aria-live", "polite");
      await expect(embed.getByRole("button", {name: "Close"})).toBeVisible();
    });
    await test.step("accessibility-frame-attributes", async () => {
      await expect(frame).toHaveAttribute("title", "Installed UI");
      await expect(frame).toHaveAttribute("sandbox", "allow-scripts allow-same-origin");
      await expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
    });
    await test.step("accessibility-platform-csp", async () => {
      const platformPolicy = await page.locator(
        'meta[http-equiv="Content-Security-Policy"]',
      ).getAttribute("content");
      const webOrigin = new URL(webUrl);
      const webPort = webOrigin.port || (webOrigin.protocol === "https:" ? "443" : "80");
      const expectedFrameSource = `https://*.${uiNamespace}${webPort === "443" ? "" : `:${webPort}`}`;
      const frameSourceCount = (platformPolicy ?? "").split(/\s+/)
        .filter(token => token === expectedFrameSource).length;
      expect(platformPolicy?.includes("frame-src 'self'") ?? false).toBe(true);
      expect(frameSourceCount).toBe(1);
    });
    await test.step("accessibility-undeclared-fetch", async () => {
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
    });
    await test.step("accessibility-parent-navigation", async () => {
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
    });
    await test.step("accessibility-platform-axe", async () => {
      const axe = await new AxeBuilder({page}).include("#installed-ui-navigation-project").analyze();
      expect(axe.violations.length).toBe(0);
    });
    await test.step("accessibility-frame-axe", async () => {
      const frameAxe = await new AxeBuilder({page}).include("#installed-ui-frame-project").analyze();
      expect(frameAxe.violations.length).toBe(0);
    });
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
  await test.step("signin-platform", async () => {
    await page.goto("/");
    await expect(page.getByTestId("oidc-login")).toBeVisible();
  });
  await test.step("signin-redirect", async () => {
    await page.getByTestId("oidc-login").click();
    await expect(page).toHaveURL(new RegExp(escapeRegExp(`${oidcUrl}/authorize`)));
  });
  await test.step("signin-account", async () => {
    await page.locator('input[name="login"]').fill(account);
    await page.getByRole("button", {name: /Continue as/}).click();
  });
  await test.step("signin-return", async () => {
    await expect(page).toHaveURL(/\/organizations$/);
    await waitForLiveView(page);
  });
}

async function waitForLiveView(page: import("@playwright/test").Page) {
  await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function requestHasCookie(headers: Record<string, string>, name: string): boolean {
  const cookieHeader = headers.cookie ?? "";
  return cookieHeader.split(";").some(cookie => cookie.trim().startsWith(`${name}=`));
}
