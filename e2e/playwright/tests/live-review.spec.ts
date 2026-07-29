import {expect, test} from "@playwright/test";
import {execFileSync} from "node:child_process";
import {mkdtempSync, rmSync, writeFileSync, mkdirSync} from "node:fs";
import {tmpdir} from "node:os";
import path from "node:path";
import pg from "pg";

const databaseUrl =
  process.env.HEPHAESTUS_E2E_DATABASE_URL ??
  "postgres://postgres:postgres@127.0.0.1:55432/hephaestus";
const repositoryRoot = process.env.HEPHAESTUS_REPOSITORY_ROOT;
const gitUrl = process.env.HEPHAESTUS_GIT_URL ?? "http://127.0.0.1:8080";
const oidcUrl = process.env.HEPHAESTUS_OIDC_URL ?? "http://127.0.0.1:5556";
const secretSentinel = "HEPHAESTUS_BROWSER_SECRET_4d7ccf";

test.describe.serial("release, instance, secret, and live-review product journey", () => {
  test("imports, binds, runs, updates, recovers, and never renders secret values", async ({
    page
  }) => {
    const fixture = await loadFixture();
    const browserMessages: string[] = [];
    page.on("console", message => browserMessages.push(message.text()));

    await page.goto("/");
    await page.getByTestId("oidc-login").click();
    await expect(page).toHaveURL(new RegExp(`${escapeRegExp(oidcUrl)}/authorize`));
    await page.locator('input[name="login"]').fill("reviewer");
    await page.getByRole("button", {name: "Continue as Ada Reviewer"}).click();
    await expect(page).toHaveURL(/\/organizations$/);
    await waitForLiveView(page);

    await page.getByTestId(`organization-${fixture.organizationId}`).click();
    await page.getByTestId(`project-${fixture.projectId}`).click();
    await page.locator("#project-tabs").getByText("Agents").click();
    await waitForLiveView(page);

    const importForm = page.locator(`#import-agent-${fixture.releaseAgents[0]}`);
    await expect(importForm).toBeVisible();
    await importForm.locator('input[name="import[name]"]').fill("browser-reviewer");
    await importForm
      .locator('select[name="import[parameters][review_style]"]')
      .selectOption("strict");
    await importForm
      .locator('input[name="import[parameters][private_hint]"]')
      .fill("not-a-secret-parameter");
    await importForm.getByRole("button", {name: "Import as new instance"}).click();
    await expect(page).toHaveURL(/\/projects\/[^/]+\/agents\/[^/]+$/);
    await waitForLiveView(page);
    const instanceId = page.url().split("/").at(-1)!;

    await page
      .locator("#create-attachment")
      .locator('select[name="attachment[repository_id]"]')
      .selectOption(fixture.repositoryId);
    await page.getByRole("button", {name: "Create attachment"}).click();
    await expect(page.locator("#instance-attachments")).toContainText("agent-workbench");

    await createOrganizationSecretAndGrant(page, fixture);
    await page.goto(`/projects/${fixture.projectId}/settings`);
    await waitForLiveView(page);
    await acceptVisibleGrant(page, "org_token");
    await createProjectSecretGrantAndImport(page, fixture);

    await page.goto(`/projects/${fixture.projectId}/agents/${instanceId}`);
    await waitForLiveView(page);
    await bindSlot(page, "raw_token", "org_token", "raw", true);
    await bindSlot(page, "broker_token", "repo_token", "brokered", false);

    await page.goto(`/projects/${fixture.projectId}/runs`);
    await expect(page.locator("#project-run-stream .empty-copy")).toBeVisible();

    pushCommit(fixture.repositoryId, "first browser run");

    const runRow = page.locator("#project-run-stream [id^='project-run-']").first();
    await expect(runRow).toBeVisible();
    await runRow.click();
    await expect(page.getByTestId("run-timeline")).toContainText("result · completed");
    await expect(page.locator("#run-exact-provenance")).toContainText("Release v1");
    await expect(page.getByTestId("run-timeline")).toContainText("result · completed");
    await expect(page.getByTestId("runtime-metrics")).toContainText("fixture.cpu_ms");
    await expect(page.getByTestId("runtime-metrics")).toContainText("42");
    await expect(page.getByTestId("result-diff")).toContainText(
      "agent reviewed and changed this file"
    );
    await expect(page.getByTestId("review-proposal")).toContainText(
      "Controlled result proposal"
    );

    await page.getByTestId("approve-result").click();
    await expect(page.getByTestId("review-proposal")).toContainText("approved");

    const proposal = await latestProposal(fixture.repositoryId);
    expect(proposal.state).toBe("approved");
    expect(
      git([
        `--git-dir=${path.join(repositoryRoot!, `${fixture.repositoryId}.git`)}`,
        "rev-parse",
        proposal.target_ref
      ])
    ).toBe(proposal.result_commit);

    await page.goto(`/projects/${fixture.projectId}/agents/${instanceId}`);
    await waitForLiveView(page);
    const activeRevision = page.locator("#instance-overview article").first().locator("strong");
    const activeBefore = (await activeRevision.textContent())?.trim();
    await page
      .locator("#create-update")
      .locator('select[name="update[release_agent_id]"]')
      .selectOption(fixture.releaseAgents[1]);
    await page
      .locator("#create-update")
      .locator('select[name="update[parameters][review_style]"]')
      .selectOption("balanced");
    await page
      .locator("#create-update")
      .locator('input[name="update[parameters][private_hint]"]')
      .fill("replacement-sensitive-parameter");
    await page.getByRole("button", {name: "Start reviewed update"}).click();
    await expect(page.locator("#instance-updates")).toContainText("activated");
    await expect(activeRevision).not.toHaveText(activeBefore!);

    await page
      .locator("#create-update")
      .locator('select[name="update[release_agent_id]"]')
      .selectOption(fixture.releaseAgents[2]);
    await page
      .locator("#create-update")
      .locator('select[name="update[parameters][review_style]"]')
      .selectOption("strict");
    await page
      .locator("#create-update")
      .locator('input[name="update[parameters][private_hint]"]')
      .fill("uncertain-update-parameter");
    await page.getByRole("button", {name: "Start reviewed update"}).click();
    await expect(page.locator("#instance-updates")).toContainText("compatibility_unknown");
    await expect(page.getByText("run gate closed")).toBeVisible();
    page.once("dialog", dialog => dialog.accept());
    await page.getByRole("button", {name: "Reject candidate"}).click();
    await expect(page.getByText("run gate open")).toBeVisible();

    await assertNoSecretSentinel(page, browserMessages);
    await verifySecretProvenance(instanceId);
  });

  test("a reviewer can reject a second durable result", async ({page}) => {
    const fixture = await loadFixture();
    await signIn(page);
    await page.goto(`/projects/${fixture.projectId}/runs`);
    await waitForLiveView(page);
    pushCommit(fixture.repositoryId, "second browser run", true);

    const runRows = page
      .locator("#project-run-stream [id^='project-run-']")
      .filter({hasText: "agent-workbench"});
    await expect(runRows).toHaveCount(2);
    await runRows.first().click();
    await expect(page.getByTestId("review-proposal")).toBeVisible();
    await page.getByTestId("reject-result").click();
    await expect(page.getByTestId("review-proposal")).toContainText("rejected");
  });
});

async function signIn(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("oidc-login").click();
  await page.locator('input[name="login"]').fill("reviewer");
  await page.getByRole("button", {name: "Continue as Ada Reviewer"}).click();
  await expect(page).toHaveURL(/\/organizations$/);
  await waitForLiveView(page);
}

async function waitForLiveView(page: import("@playwright/test").Page) {
  await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

async function loadFixture() {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(`
    SELECT organization.id AS organization_id, project.id AS project_id,
           repository.id AS repository_id
    FROM organizations organization
    JOIN projects project ON project.organization_id = organization.id
    JOIN repositories repository ON repository.project_id = project.id
    WHERE organization.name = 'Acme Research'
      AND repository.name = 'agent-workbench'
  `);
  await client.end();
  const releaseClient = new pg.Client({connectionString: databaseUrl});
  await releaseClient.connect();
  const releases = await releaseClient.query(
    `SELECT release_agent.id
     FROM release_agents release_agent
     JOIN releases release ON release.id = release_agent.release_id
     WHERE release.repository_id = $1
     ORDER BY release.version`,
    [result.rows[0].repository_id]
  );
  await releaseClient.end();
  return {
    organizationId: result.rows[0].organization_id,
    projectId: result.rows[0].project_id,
    repositoryId: result.rows[0].repository_id,
    releaseAgents: releases.rows.map(row => row.id)
  };
}

async function createOrganizationSecretAndGrant(
  page: import("@playwright/test").Page,
  fixture: Awaited<ReturnType<typeof loadFixture>>
) {
  await page.goto(`/organizations/${fixture.organizationId}/secrets/new`);
  await waitForLiveView(page);
  const create = page.locator("#create-organization-secret");
  await create.locator('input[name="secret[name]"]').fill("organization_token");
  await create.locator('input[name="secret[value]"]').fill(`${secretSentinel}_org`);
  await create.locator('select[name="secret[modes][]"]').selectOption(["raw"]);
  await expect(create.locator('input[name="secret[name]"]')).toHaveValue("organization_token");
  expect(
    await create
      .locator('select[name="secret[modes][]"]')
      .evaluate(select =>
        Array.from((select as HTMLSelectElement).selectedOptions, option => option.value)
      )
  ).toEqual(["raw"]);
  expect(
    await create.locator(":invalid").evaluateAll(elements =>
      elements.map(element => ({
        name: (element as HTMLInputElement).name,
        message: (element as HTMLInputElement).validationMessage,
        value: (element as HTMLInputElement).type === "password" ? "[REDACTED]" : null
      }))
    )
  ).toEqual([]);
  await create.getByRole("button", {name: "Encrypt and create"}).click();
  await expect(page).toHaveURL(
    new RegExp(`/organizations/${fixture.organizationId}/secrets$`)
  );
  await expect(page.getByText("Organization secret encrypted and stored.")).toBeVisible();
  await expect(page.locator("#organization-secrets")).toContainText("organization_token");
  await expect(page.locator("body")).not.toContainText(secretSentinel);
  await expect(
    page.locator("#owned-secrets-heading").getByTestId("create-organization-secret-link")
  ).toBeVisible();
  await expect(
    page.locator("#bounded-grants-heading").getByTestId("offer-organization-grant-link")
  ).toBeVisible();

  await page.getByTestId("offer-organization-grant-link").click();
  await expect(page).toHaveURL(
    new RegExp(`/organizations/${fixture.organizationId}/secret-grants/new$`)
  );
  const grant = page.locator("#grant-organization-secret");
  await grant.locator('select[name="grant[secret_id]"]').selectOption({index: 1});
  await grant
    .locator('select[name="grant[target]"]')
    .selectOption(`project:${fixture.projectId}`);
  await grant.locator('select[name="grant[modes][]"]').selectOption(["raw"]);
  await grant.locator('select[name="grant[phases][]"]').selectOption(["normal"]);
  await grant.getByRole("button", {name: "Offer exact grant"}).click();
  await expect(page).toHaveURL(
    new RegExp(`/organizations/${fixture.organizationId}/secrets$`)
  );
  await expect(page.locator("#organization-secret-grants")).toContainText(
    "organization_token"
  );
}

async function acceptVisibleGrant(page: import("@playwright/test").Page, alias: string) {
  const form = page.locator('[id^="accept-import-"]').first();
  await expect(form).toBeVisible();
  await form.locator('input[name="secret_import[alias]"]').fill(alias);
  await form.getByRole("button", {name: "Accept live reference"}).click();
  await expect(page.getByText("Live secret reference accepted.")).toBeVisible();
}

async function createProjectSecretGrantAndImport(
  page: import("@playwright/test").Page,
  fixture: Awaited<ReturnType<typeof loadFixture>>
) {
  const create = page.locator("#create-project-secret");
  await create.locator('input[name="secret[name]"]').fill("project_token");
  await create.locator('input[name="secret[value]"]').fill(`${secretSentinel}_project`);
  await create.locator('select[name="secret[modes][]"]').selectOption(["brokered"]);
  await create.getByRole("button", {name: "Encrypt and create"}).click();
  await expect(page.getByText("Secret encrypted and stored.")).toBeVisible();
  await expect(page.locator("#project-secret-stream")).toContainText("project_token");
  await expect(page.locator("body")).not.toContainText(secretSentinel);

  const grant = page.locator("#grant-secret");
  await grant.locator('select[name="grant[secret_id]"]').selectOption({label: "project_token"});
  await grant
    .locator('select[name="grant[target]"]')
    .selectOption(`repository:${fixture.repositoryId}`);
  await grant.locator('select[name="grant[modes][]"]').selectOption(["brokered"]);
  await grant.locator('select[name="grant[phases][]"]').selectOption(["normal", "update"]);
  await grant.locator('input[name="grant[destinations]"]').fill("api.example.com");
  await grant.getByRole("button", {name: "Review and offer grant"}).click();
  await acceptVisibleGrant(page, "repo_token");
}

async function bindSlot(
  page: import("@playwright/test").Page,
  slot: string,
  alias: string,
  mode: "raw" | "brokered",
  confirmRaw: boolean
) {
  const activeRevision = page.locator("#instance-overview article").first().locator("strong");
  const previousRevision = (await activeRevision.textContent())?.trim();
  const form = page.locator(`#bind-secret-${slot}`);
  const importSelect = form.locator('select[name="binding[import_id]"]');
  const importValue = await importSelect
    .locator("option")
    .filter({hasText: alias})
    .getAttribute("value");
  await importSelect.selectOption(importValue!);
  await form.locator('select[name="binding[mode]"]').selectOption(mode);
  await form.locator('select[name="binding[phases][]"]').selectOption(["normal"]);
  if (mode === "raw" || alias === "repo_token") {
    await form.locator('select[name="binding[attachment_ids][]"]').selectOption({index: 0});
  }
  if (confirmRaw) {
    await form.locator('input[type="checkbox"][name="binding[raw_confirmation]"]').check();
  }
  await form.getByRole("button", {name: "Create binding revision"}).click();
  await expect(activeRevision).not.toHaveText(previousRevision!);
  await expect(page.getByText("Secret binding activated")).toBeVisible();
}

async function assertNoSecretSentinel(
  page: import("@playwright/test").Page,
  browserMessages: string[]
) {
  expect(await page.content()).not.toContain(secretSentinel);
  expect(browserMessages.join("\n")).not.toContain(secretSentinel);
  const screenshot = await page.screenshot();
  expect(screenshot.toString("base64")).not.toContain(
    Buffer.from(secretSentinel).toString("base64")
  );
}

async function verifySecretProvenance(instanceId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT count(*)::integer AS count,
            bool_and(provenance.secret_version_id IS NOT NULL) AS exact_versions,
            bool_and(session.runtime_credential_hash IS NOT NULL) AS hashed_credentials
     FROM run_secret_provenance provenance
     JOIN secret_runtime_sessions session ON session.run_id = provenance.run_id
     JOIN runs run ON run.id = provenance.run_id
     WHERE run.instance_id = $1`,
    [instanceId]
  );
  const leakage = await client.query(
    `SELECT COALESCE((
       SELECT string_agg(payload::text, ' ') FROM outbox
     ), '') || COALESCE((
       SELECT string_agg(row_to_json(secret_audit_events)::text, ' ')
       FROM secret_audit_events
     ), '') AS searchable`
  );
  await client.end();
  expect(result.rows[0].count).toBeGreaterThanOrEqual(2);
  expect(result.rows[0].exact_versions).toBe(true);
  expect(result.rows[0].hashed_credentials).toBe(true);
  expect(leakage.rows[0].searchable ?? "").not.toContain(secretSentinel);
}

async function latestProposal(repositoryId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT state, target_ref, result_commit
     FROM review_proposals
     WHERE repository_id = $1
     ORDER BY created_at DESC LIMIT 1`,
    [repositoryId]
  );
  await client.end();
  return result.rows[0];
}

function pushCommit(repositoryId: string, message: string, clone = false) {
  const work = mkdtempSync(path.join(tmpdir(), "hephaestus-ui-e2e-"));
  try {
    const token = execFileSync("curl", ["--fail", "--silent", `${oidcUrl}/test/git-token`], {
      encoding: "utf8"
    }).trim();
    const remote = `${gitUrl}/${repositoryId}`;
    if (clone) {
      git(
        [
          "-c",
          `http.extraHeader=Authorization: Bearer ${token}`,
          "clone",
          remote,
          work
        ],
        process.cwd()
      );
    } else {
      git(["init", "--initial-branch=main", work], process.cwd());
    }
    git(["config", "user.name", "Browser E2E"], work);
    git(["config", "user.email", "browser@example.invalid"], work);
    writeFileSync(path.join(work, "input.txt"), `${message}\n`);
    mkdirSync(path.join(work, "reports"), {recursive: true});
    writeFileSync(path.join(work, "reports/result.txt"), "waiting for agent\n");
    writeFileSync(path.join(work, "agent.toml"), agentConfig());
    git(["add", "."], work);
    git(["commit", "-m", message], work);
    if (!clone) git(["remote", "add", "origin", remote], work);
    git(
      [
        "-c",
        `http.extraHeader=Authorization: Bearer ${token}`,
        "push",
        "origin",
        "HEAD:refs/heads/main"
      ],
      work
    );
  } finally {
    rmSync(work, {recursive: true, force: true});
  }
}

function git(arguments_: string[], cwd = process.cwd()) {
  return execFileSync("git", arguments_, {cwd, encoding: "utf8"}).trim();
}

function agentConfig() {
  return `
version = 1
[agent]
name = "browser-agent"
[guest]
command = "/bin/sh"
arguments = ["-c", "true"]
working_directory = "/workspace/work"
[resources]
vcpus = 1
memory_mib = 128
[root_image]
reference = "fixture-root@sha256:e2e"
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = true
[results]
declared_files = ["reports/result.txt"]
[network]
profile = "disabled"
[triggers]
push = true
refs = ["refs/heads/main"]
`.trimStart();
}
