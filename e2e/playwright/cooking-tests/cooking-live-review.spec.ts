import {expect, test} from "@playwright/test";
import {readFileSync} from "node:fs";

type CookingFixture = {
  project_id: string;
  repository_id: string;
  release_id: string;
  release_agent_id: string;
  instance_id: string;
  mailbox_id: string;
  gateway_id: string;
  inbound_import_id: string;
  inbound_secret_version_id: string;
  inbound_selection: string;
  parameters: Record<string, string | number | boolean>;
};

const fixturePath = process.env.HEPHAESTUS_COOKING_BROWSER_FIXTURE;
const oidcUrl = process.env.HEPHAESTUS_OIDC_URL ?? "http://127.0.0.1:5556";

test("cooking release install, mailbox, gateway configure, and binding", async ({page}, testInfo) => {
  const fixture = loadFixture();
  await page.goto("/");
  await page.getByTestId("oidc-login").click();
  await expect(page).toHaveURL(new RegExp(escapeRegExp(`${oidcUrl}/authorize`)));
  await page.locator('input[name="login"]').fill(process.env.HEPHAESTUS_COOKING_BROWSER_ACCOUNT ?? "reviewer");
  await page.getByRole("button", {name: /Continue as/}).click();
  await expect(page).toHaveURL(/\/organizations$/);
  await waitForLiveView(page);

  await page.goto(`/repositories/${fixture.repository_id}/releases/${fixture.release_id}`);
  await waitForLiveView(page);
  const install = page.locator("#install-release-gateways");
  await expect(install).toBeVisible();
  await install.click();
  await expect(page.getByRole("alert")).toContainText("Release gateways installed");

  // The golden scenario already owns and authorizes this exact instance. Use
  // it for the browser journey so the test does not create an orphaned second
  // instance or bind the configured gateway to a different mailbox.
  await page.goto(`/projects/${fixture.project_id}/agents/${fixture.instance_id}`);
  await waitForLiveView(page);
  await page.locator("#create-instance-mailbox").click();
  await expect(page.getByRole("alert").filter({hasText: "Mailbox ready:"})).toBeVisible();
  await expect(page.getByRole("alert").filter({hasText: fixture.mailbox_id})).toBeVisible();

  await page.goto(`/projects/${fixture.project_id}/gateways/${fixture.gateway_id}`);
  await waitForLiveView(page);
  await expect(page.locator("#configure-gateway-form")).toBeVisible();
  for (const [name, value] of Object.entries(fixture.parameters)) {
    await page.locator(`#configure-parameter-${name}`).fill(String(value));
  }
  const secret = page.locator('select[name="configure[secret_selections][webhook]"]');
  await secret.selectOption(fixture.inbound_selection);
  const existingRevisionIds = new Set(
    await page.locator("#gateway-revision-list article").evaluateAll(articles =>
      articles.map(article => article.id)
    )
  );
  await page.getByRole("button", {name: "Create immutable revision"}).click();
  // Fail at the configure response instead of waiting for the revision poll
  // when the server rejected the request.
  const configurationError = page
    .getByRole("alert")
    .filter({hasText: "Gateway configuration was not accepted."});
  await expect.poll(async () => {
    if (await configurationError.count()) return "configuration-error";
    const ids = await page.locator("#gateway-revision-list article").evaluateAll(articles =>
      articles.map(article => article.id)
    );
    return ids.find(id => !existingRevisionIds.has(id)) ?? "pending";
  }, {timeout: 30_000, intervals: [100, 250, 500]}).not.toBe("pending");
  await expect(configurationError).toHaveCount(0);
  const newRevisionId = (await page.locator("#gateway-revision-list article").evaluateAll(articles =>
    articles.map(article => article.id)
  )).find(id => !existingRevisionIds.has(id));
  if (!newRevisionId) throw new Error("browser-created gateway revision has no stable DOM id");
  await expect(page.locator(`#${newRevisionId}`)).toBeVisible();

  await expect(page.locator("#gateway-binding-form")).toBeVisible();
  await page.locator('select[name="gateway_binding[slot_key]"]').selectOption({label: "cooking_requests"});
  await page.locator('select[name="gateway_binding[mailbox_id]"]').selectOption(fixture.mailbox_id);
  // Producer identity is the mailbox-local deduplication scope and cannot be
  // reused by a later immutable gateway revision.
  await page
    .locator('input[name="gateway_binding[producer_id]"]')
    .fill(`cooking-browser-${newRevisionId}`);
  await page.getByRole("button", {name: "Create mailbox binding"}).click();
  await expect(page.getByRole("alert")).toContainText("Mailbox binding");
  const matchingBindings = page
    .locator("#gateway-binding-list article")
    .filter({hasText: fixture.mailbox_id});
  await expect(matchingBindings).toHaveCount(1);
  await expect(matchingBindings).toContainText(fixture.mailbox_id);
  await page.screenshot({path: testInfo.outputPath("cooking-installed-gateway.png"), fullPage: true});
});

function loadFixture(): CookingFixture {
  if (!fixturePath) throw new Error("HEPHAESTUS_COOKING_BROWSER_FIXTURE is required");
  const fixture = JSON.parse(readFileSync(fixturePath, "utf8")) as CookingFixture;
  for (const key of [
    "project_id",
    "repository_id",
    "release_id",
    "release_agent_id",
    "instance_id",
    "mailbox_id",
    "gateway_id",
    "inbound_import_id",
    "inbound_secret_version_id",
    "inbound_selection",
    "parameters"
  ] as const) {
    if (!fixture[key]) throw new Error(`cooking fixture is missing ${key}`);
  }
  for (const key of ["inbound_placeholder", "alice_provider_id", "bob_provider_id"] as const) {
    if (!(key in fixture.parameters)) throw new Error(`cooking fixture parameter is missing ${key}`);
  }
  return fixture;
}

async function waitForLiveView(page: import("@playwright/test").Page) {
  await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
