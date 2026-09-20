import {expect, test} from "@playwright/test";
import {AxeBuilder} from "@axe-core/playwright";
import {existsSync, readFileSync, writeFileSync} from "node:fs";
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
const controlDirectory = process.env.HEPHAESTUS_INSTALLED_UI_CONTROL_DIR;
if (!webUrl) throw new Error("HEPHAESTUS_WEB_URL is required");
if (!uiNamespace) throw new Error("HEPHAESTUS_UI_NAMESPACE is required");
if (controlDirectory !== "/run/heph-control") {
  throw new Error("HEPHAESTUS_INSTALLED_UI_CONTROL_DIR must be /run/heph-control");
}

test("cooking installed UI TLS full-page and managed iframe smoke", async ({page}) => {
  // Lifecycle barriers and the Phoenix refresh interval share the controller's
  // bounded 360-second verification window.
  test.setTimeout(360_000);
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
  const managedFrameBeforeDisable = await frame.elementHandle();
  const managedContentFrameBeforeDisable = managedFrameBeforeDisable
    ? await managedFrameBeforeDisable.contentFrame()
    : null;
  expect(managedContentFrameBeforeDisable).not.toBeNull();
  if (managedContentFrameBeforeDisable === null) return;
  const managedDocumentUrl = new URL(
    "/managed-reference/index.html",
    managedContentFrameBeforeDisable.url(),
  ).toString();
  const stalePage = await page.context().newPage();
  let logoutPage: import("@playwright/test").Page | null = null;
  let removedPage: import("@playwright/test").Page | null = null;
  try {
    let baselineStatusCode = 0;
    let baselineUiCookiePresent = false;
    await test.step("lifecycle-stale-baseline-response", async () => {
      const response = await stalePage.goto(managedDocumentUrl, {
        timeout: 30_000,
        waitUntil: "networkidle",
      });
      baselineStatusCode = response?.status() ?? 0;
      if (response) {
        baselineUiCookiePresent = requestHasCookie(
          await response.request().allHeaders(),
          "__Host-hephaestus_ui",
        );
      }
    });
    await test.step(
      baselineStatusCode === 200 ? "lifecycle-stale-baseline-200" : "lifecycle-stale-baseline-other",
      async () => {
        expect(baselineStatusCode).toBe(200);
      },
    );
    await test.step("lifecycle-stale-baseline-cookie", async () => {
      expect(baselineUiCookiePresent).toBe(true);
    });
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
    await test.step("lifecycle-disable", async () => {
      await test.step("lifecycle-managed-ready", async () => {
        writeFileSync(join(controlDirectory, "managed-ready"), "ready\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });
      await test.step("lifecycle-disable-complete", async () => {
        await expect.poll(
          () => existsSync(join(controlDirectory, "disable-complete")),
          {timeout: 30_000},
        ).toBe(true);
      });
      let staleStatusCode = 0;
      let staleRequestObserved = false;
      let staleUiCookiePresent = false;
      let staleRequestHeaders: Record<string, string> = {};
      await test.step("lifecycle-stale-fetch-response", async () => {
        // A disabled generation host is retired before child authentication,
        // so the exact old host is expected to return 404 after disable.
        const [request, response] = await Promise.all([
          stalePage.waitForRequest(
            candidate => candidate.url() === managedDocumentUrl && candidate.resourceType() === "fetch",
            {timeout: 30_000},
          ),
          stalePage.waitForResponse(
            candidate => candidate.url() === managedDocumentUrl && candidate.request().resourceType() === "fetch",
            {timeout: 30_000},
          ),
          stalePage.evaluate(async url => {
            const candidate = await fetch(url, {cache: "no-store", credentials: "same-origin"});
            return candidate.status;
          }, managedDocumentUrl),
        ]);
        staleStatusCode = response.status();
        staleRequestObserved = true;
        staleRequestHeaders = await request.allHeaders();
      });
      const staleStatusStage = staleStatusCode === 401 ? "lifecycle-stale-fetch-401" :
        staleStatusCode === 403 ? "lifecycle-stale-fetch-403" :
          staleStatusCode === 404 ? "lifecycle-stale-fetch-404" :
            staleStatusCode === 410 ? "lifecycle-stale-fetch-410" :
              staleStatusCode === 200 ? "lifecycle-stale-fetch-200" : "lifecycle-stale-fetch-other";
      await test.step("lifecycle-stale-request-observed", async () => {
        expect(staleRequestObserved).toBe(true);
      });
      await test.step("lifecycle-stale-cookie-present", async () => {
        staleUiCookiePresent = requestHasCookie(staleRequestHeaders, "__Host-hephaestus_ui");
        expect(staleUiCookiePresent).toBe(true);
      });
      await test.step(staleStatusStage, async () => {
        expect(staleStatusCode).toBe(404);
      });
      await page.reload({waitUntil: "domcontentloaded", timeout: 30_000});
      await waitForLiveView(page);
      await test.step("lifecycle-disabled-card", async () => {
        const disabledCard = page.locator(`#installed-ui-${installed.managed_installation_id}`);
        await expect(disabledCard).toBeVisible();
        await expect(disabledCard).toContainText("Disabled");
        await expect(disabledCard).toContainText("Unavailable");
        await expect(disabledCard.getByRole("button", {name: /Launch/})).toHaveCount(0);
      });
      await test.step("lifecycle-stale-cookie-denied", async () => {
        writeFileSync(join(controlDirectory, "stale-cookie-denied"), "denied\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });
      await test.step("lifecycle-reactivate-complete", async () => {
        await expect.poll(
          () => existsSync(join(controlDirectory, "reactivate-complete")),
          {timeout: 30_000},
        ).toBe(true);
      });

      let oldGenerationStatusCode = 0;
      let oldGenerationRequestObserved = false;
      let oldGenerationUiCookiePresent = false;
      await test.step("lifecycle-old-generation-fetch-response", async () => {
        const [request, response] = await Promise.all([
          stalePage.waitForRequest(
            candidate => candidate.url() === managedDocumentUrl && candidate.resourceType() === "fetch",
            {timeout: 30_000},
          ),
          stalePage.waitForResponse(
            candidate => candidate.url() === managedDocumentUrl && candidate.request().resourceType() === "fetch",
            {timeout: 30_000},
          ),
          stalePage.evaluate(async url => {
            const candidate = await fetch(url, {cache: "no-store", credentials: "same-origin"});
            return candidate.status;
          }, managedDocumentUrl),
        ]);
        oldGenerationStatusCode = response.status();
        oldGenerationRequestObserved = true;
        oldGenerationUiCookiePresent = requestHasCookie(
          await request.allHeaders(),
          "__Host-hephaestus_ui",
        );
      });
      const oldGenerationStatusStage = oldGenerationStatusCode === 401 ? "lifecycle-old-generation-fetch-401" :
        oldGenerationStatusCode === 403 ? "lifecycle-old-generation-fetch-403" :
          oldGenerationStatusCode === 404 ? "lifecycle-old-generation-fetch-404" :
            oldGenerationStatusCode === 410 ? "lifecycle-old-generation-fetch-410" :
              oldGenerationStatusCode === 200 ? "lifecycle-old-generation-fetch-200" :
                "lifecycle-old-generation-fetch-other";
      await test.step("lifecycle-old-generation-request-observed", async () => {
        expect(oldGenerationRequestObserved).toBe(true);
      });
      await test.step("lifecycle-old-generation-cookie-present", async () => {
        expect(oldGenerationUiCookiePresent).toBe(true);
      });
      await test.step(oldGenerationStatusStage, async () => {
        expect(oldGenerationStatusCode).toBe(404);
      });
      await test.step("lifecycle-old-generation-denied", async () => {
        writeFileSync(join(controlDirectory, "old-generation-denied-after-reactivate"), "denied\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });
      await test.step("lifecycle-old-generation-denial-verified", async () => {
        await expect.poll(
          () => existsSync(join(controlDirectory, "old-generation-denial-verified")),
          {timeout: 30_000},
        ).toBe(true);
      });

      await page.reload({waitUntil: "domcontentloaded", timeout: 30_000});
      await waitForLiveView(page);
      const reactivatedCard = page.locator(`#installed-ui-${installed.managed_installation_id}`);
      await expect(reactivatedCard).toBeVisible();
      const reactivatedDocument = page.waitForResponse(response => {
        try {
          const url = new URL(response.url());
          return url.hostname.startsWith("g-") &&
            url.pathname === "/managed-reference/index.html" &&
            response.request().resourceType() === "document";
        } catch {
          return false;
        }
      }, {timeout: 30_000});
      await test.step("lifecycle-reactivated-launch", async () => {
        await reactivatedCard.getByRole("button", {name: /Launch/}).click();
      });
      await test.step("lifecycle-reactivated-frame-visible", async () => {
        await expect(frame).toBeVisible();
      });
      let reactivatedFrameUrl = "";
      await test.step("lifecycle-reactivated-frame-route", async () => {
        await expect.poll(async () => {
          const frameHandle = await frame.elementHandle();
          const childFrame = frameHandle ? await frameHandle.contentFrame() : null;
          if (!childFrame) return false;
          reactivatedFrameUrl = childFrame.url();
          const url = new URL(reactivatedFrameUrl);
          return /^g-[0-9a-f]{32}\./.test(url.hostname) &&
            url.pathname === "/managed-reference/index.html";
        }, {timeout: 30_000}).toBe(true);
      });
      await test.step("lifecycle-reactivated-generation-different", async () => {
        expect(new URL(reactivatedFrameUrl).hostname).not.toBe(new URL(managedDocumentUrl).hostname);
      });
      let reactivatedDocumentStatusCode = 0;
      let reactivatedDocumentResponseUrl = "";
      let reactivatedDocumentUiCookiePresent = false;
      await test.step("lifecycle-reactivated-document-response", async () => {
        const response = await reactivatedDocument;
        reactivatedDocumentStatusCode = response.status();
        reactivatedDocumentResponseUrl = response.url();
        reactivatedDocumentUiCookiePresent = requestHasCookie(
          await response.request().allHeaders(),
          "__Host-hephaestus_ui",
        );
      });
      await test.step(
        reactivatedDocumentStatusCode >= 200 && reactivatedDocumentStatusCode < 300
          ? "lifecycle-reactivated-document-2xx"
          : "lifecycle-reactivated-document-other",
        async () => {
          expect(reactivatedDocumentStatusCode).toBe(200);
        },
      );
      await test.step("lifecycle-reactivated-document-url", async () => {
        expect(reactivatedDocumentResponseUrl === reactivatedFrameUrl).toBe(true);
      });
      await test.step("lifecycle-reactivated-bootstrap-fragment", async () => {
        expect(new URL(reactivatedFrameUrl).hash).toBe("");
      });
      await test.step("lifecycle-reactivated-cookie-present", async () => {
        expect(reactivatedDocumentUiCookiePresent).toBe(true);
      });
      await test.step("lifecycle-reactivated-identity", async () => {
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
      await test.step("lifecycle-reactivated-frame-ready", async () => {
        await expect(embed.locator("[data-ui-status]")).toHaveText("Ready");
      });
      await test.step("lifecycle-new-generation-ready", async () => {
        writeFileSync(join(controlDirectory, "new-generation-ready"), "ready\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });

      const parentLogoutPage = await page.context().newPage();
      logoutPage = parentLogoutPage;
      let logoutBaselineStatusCode = 0;
      let logoutBaselineUiCookiePresent = false;
      await test.step("lifecycle-logout-baseline-response", async () => {
        const response = await parentLogoutPage.goto(reactivatedFrameUrl, {
          timeout: 30_000,
          waitUntil: "networkidle",
        });
        logoutBaselineStatusCode = response?.status() ?? 0;
        if (response) {
          logoutBaselineUiCookiePresent = requestHasCookie(
            await response.request().allHeaders(),
            "__Host-hephaestus_ui",
          );
        }
      });
      await test.step(
        logoutBaselineStatusCode === 200
          ? "lifecycle-logout-baseline-200"
          : "lifecycle-logout-baseline-other",
        async () => {
          expect(logoutBaselineStatusCode).toBe(200);
        },
      );
      await test.step("lifecycle-logout-baseline-cookie", async () => {
        expect(logoutBaselineUiCookiePresent).toBe(true);
      });
      await test.step("lifecycle-parent-revoke-ready", async () => {
        writeFileSync(join(controlDirectory, "parent-revoke-ready"), "ready\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });
      await test.step("lifecycle-parent-revoke-permitted", async () => {
        await expect.poll(
          () => existsSync(join(controlDirectory, "parent-revoke-permitted")),
          {timeout: 30_000},
        ).toBe(true);
      });
      await test.step("lifecycle-logout-click", async () => {
        await page.getByRole("link", {name: "Sign out"}).click();
      });
      await test.step("lifecycle-logout-signed-out", async () => {
        await expect(page).toHaveURL(/\/$/);
        await expect(page.getByTestId("oidc-login")).toBeVisible();
      });

      let logoutStatusCode = 0;
      let logoutRequestObserved = false;
      let logoutUiCookiePresent = false;
      await test.step("lifecycle-logout-fetch-response", async () => {
        const [request, response] = await Promise.all([
          parentLogoutPage.waitForRequest(
            candidate => candidate.url() === reactivatedFrameUrl && candidate.resourceType() === "fetch",
            {timeout: 30_000},
          ),
          parentLogoutPage.waitForResponse(
            candidate => candidate.url() === reactivatedFrameUrl && candidate.request().resourceType() === "fetch",
            {timeout: 30_000},
          ),
          parentLogoutPage.evaluate(async url => {
            const candidate = await fetch(url, {cache: "no-store", credentials: "same-origin"});
            return candidate.status;
          }, reactivatedFrameUrl),
        ]);
        logoutStatusCode = response.status();
        logoutRequestObserved = true;
        logoutUiCookiePresent = requestHasCookie(
          await request.allHeaders(),
          "__Host-hephaestus_ui",
        );
      });
      const logoutStatusStage = logoutStatusCode === 401 ? "lifecycle-logout-fetch-401" :
        logoutStatusCode === 403 ? "lifecycle-logout-fetch-403" :
          logoutStatusCode === 404 ? "lifecycle-logout-fetch-404" :
            logoutStatusCode === 410 ? "lifecycle-logout-fetch-410" :
              logoutStatusCode === 200 ? "lifecycle-logout-fetch-200" :
                "lifecycle-logout-fetch-other";
      await test.step("lifecycle-logout-request-observed", async () => {
        expect(logoutRequestObserved).toBe(true);
      });
      await test.step("lifecycle-logout-cookie-present", async () => {
        expect(logoutUiCookiePresent).toBe(true);
      });
      await test.step(logoutStatusStage, async () => {
        expect(logoutStatusCode).toBe(401);
      });
      await test.step("lifecycle-parent-revoked-denied", async () => {
        writeFileSync(join(controlDirectory, "parent-revoked-denied"), "denied\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });
      await test.step("lifecycle-parent-revocation-verified", async () => {
        await expect.poll(
          () => existsSync(join(controlDirectory, "parent-revocation-verified")),
          {timeout: 30_000},
        ).toBe(true);
      });

      await test.step("lifecycle-reauth-launch", async () => {
        await signIn(page, "reviewer");
        await page.goto(`/projects/${installed.project_id}`);
        await waitForLiveView(page);
      });
      const reauthCard = page.locator(`#installed-ui-${installed.managed_installation_id}`);
      const reauthDocument = page.waitForResponse(response => {
        try {
          const url = new URL(response.url());
          return url.hostname.startsWith("g-") &&
            url.pathname === "/managed-reference/index.html" &&
            response.request().resourceType() === "document";
        } catch {
          return false;
        }
      }, {timeout: 30_000});
      await test.step("lifecycle-reauth-frame-visible", async () => {
        await reauthCard.getByRole("button", {name: /Launch/}).click();
        await expect(frame).toBeVisible();
      });
      let reauthenticatedFrameUrl = "";
      await test.step("lifecycle-reauth-frame-route", async () => {
        await expect.poll(async () => {
          const frameHandle = await frame.elementHandle();
          const childFrame = frameHandle ? await frameHandle.contentFrame() : null;
          if (!childFrame) return false;
          reauthenticatedFrameUrl = childFrame.url();
          const url = new URL(reauthenticatedFrameUrl);
          return /^g-[0-9a-f]{32}\./.test(url.hostname) &&
            url.pathname === "/managed-reference/index.html";
        }, {timeout: 30_000}).toBe(true);
      });
      await test.step("lifecycle-reauth-generation-same", async () => {
        expect(new URL(reauthenticatedFrameUrl).hostname).toBe(new URL(reactivatedFrameUrl).hostname);
      });
      let reauthStatusCode = 0;
      let reauthResponseUrl = "";
      let reauthUiCookiePresent = false;
      await test.step("lifecycle-reauth-document-response", async () => {
        const response = await reauthDocument;
        reauthStatusCode = response.status();
        reauthResponseUrl = response.url();
        reauthUiCookiePresent = requestHasCookie(
          await response.request().allHeaders(),
          "__Host-hephaestus_ui",
        );
      });
      await test.step(
        reauthStatusCode >= 200 && reauthStatusCode < 300
          ? "lifecycle-reauth-document-2xx"
          : "lifecycle-reauth-document-other",
        async () => {
          expect(reauthStatusCode).toBe(200);
        },
      );
      await test.step("lifecycle-reauth-document-url", async () => {
        expect(reauthResponseUrl === reauthenticatedFrameUrl).toBe(true);
      });
      await test.step("lifecycle-reauth-bootstrap-fragment", async () => {
        expect(new URL(reauthenticatedFrameUrl).hash).toBe("");
      });
      await test.step("lifecycle-reauth-cookie-present", async () => {
        expect(reauthUiCookiePresent).toBe(true);
      });
      await test.step("lifecycle-reauth-identity", async () => {
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
      await test.step("lifecycle-reauth-frame-ready", async () => {
        await expect(embed.locator("[data-ui-status]")).toHaveText("Ready");
      });
      // The release generation remains current across parent logout; the new
      // bootstrap creates a fresh child session on the same generation host.
      reactivatedFrameUrl = reauthenticatedFrameUrl;
      await test.step("lifecycle-parent-new-child-ready", async () => {
        writeFileSync(join(controlDirectory, "parent-new-child-ready"), "ready\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });
      await test.step("lifecycle-new-generation-verified", async () => {
        await expect.poll(
          () => existsSync(join(controlDirectory, "new-generation-verified")),
          {timeout: 30_000},
        ).toBe(true);
      });

      const removalPage = await page.context().newPage();
      removedPage = removalPage;
      let removalBaselineStatusCode = 0;
      let removalBaselineUiCookiePresent = false;
      await test.step("lifecycle-removed-baseline-response", async () => {
        const response = await removalPage.goto(reactivatedFrameUrl, {
          timeout: 30_000,
          waitUntil: "networkidle",
        });
        removalBaselineStatusCode = response?.status() ?? 0;
        if (response) {
          removalBaselineUiCookiePresent = requestHasCookie(
            await response.request().allHeaders(),
            "__Host-hephaestus_ui",
          );
        }
      });
      await test.step(
        removalBaselineStatusCode === 200
          ? "lifecycle-removed-baseline-200"
          : "lifecycle-removed-baseline-other",
        async () => {
          expect(removalBaselineStatusCode).toBe(200);
        },
      );
      await test.step("lifecycle-removed-baseline-cookie", async () => {
        expect(removalBaselineUiCookiePresent).toBe(true);
      });
      await test.step("lifecycle-remove-ready", async () => {
        writeFileSync(join(controlDirectory, "remove-ready"), "ready\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });
      await test.step("lifecycle-remove-complete", async () => {
        await expect.poll(
          () => existsSync(join(controlDirectory, "remove-complete")),
          {timeout: 30_000},
        ).toBe(true);
      });

      let removedStatusCode = 0;
      let removedRequestObserved = false;
      let removedUiCookiePresent = false;
      await test.step("lifecycle-removed-fetch-response", async () => {
        const [request, response] = await Promise.all([
          removalPage.waitForRequest(
            candidate => candidate.url() === reactivatedFrameUrl && candidate.resourceType() === "fetch",
            {timeout: 30_000},
          ),
          removalPage.waitForResponse(
            candidate => candidate.url() === reactivatedFrameUrl && candidate.request().resourceType() === "fetch",
            {timeout: 30_000},
          ),
          removalPage.evaluate(async url => {
            const candidate = await fetch(url, {cache: "no-store", credentials: "same-origin"});
            return candidate.status;
          }, reactivatedFrameUrl),
        ]);
        removedStatusCode = response.status();
        removedRequestObserved = true;
        removedUiCookiePresent = requestHasCookie(
          await request.allHeaders(),
          "__Host-hephaestus_ui",
        );
      });
      const removedStatusStage = removedStatusCode === 401 ? "lifecycle-removed-fetch-401" :
        removedStatusCode === 403 ? "lifecycle-removed-fetch-403" :
          removedStatusCode === 404 ? "lifecycle-removed-fetch-404" :
            removedStatusCode === 410 ? "lifecycle-removed-fetch-410" :
              removedStatusCode === 200 ? "lifecycle-removed-fetch-200" :
                "lifecycle-removed-fetch-other";
      await test.step("lifecycle-removed-request-observed", async () => {
        expect(removedRequestObserved).toBe(true);
      });
      await test.step("lifecycle-removed-cookie-present", async () => {
        expect(removedUiCookiePresent).toBe(true);
      });
      await test.step(removedStatusStage, async () => {
        expect(removedStatusCode).toBe(404);
      });
      await test.step("lifecycle-removed-host-denied", async () => {
        writeFileSync(join(controlDirectory, "removed-host-denied"), "denied\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });

      await page.reload({waitUntil: "domcontentloaded", timeout: 30_000});
      await waitForLiveView(page);
      await test.step("lifecycle-removed-card-absent", async () => {
        await expect(page.locator(`#installed-ui-${installed.managed_installation_id}`)).toHaveCount(0);
        await expect(page.locator(`#installed-ui-${installed.static_installation_id}`)).toBeVisible();
        writeFileSync(join(controlDirectory, "removed-card-absent"), "absent\n", {
          encoding: "utf8",
          mode: 0o600,
          flag: "wx",
        });
      });
    });
  } finally {
    await (logoutPage as import("@playwright/test").Page | null)?.close();
    await (removedPage as import("@playwright/test").Page | null)?.close();
    await stalePage.close();
  }
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
