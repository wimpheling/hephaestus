import {AxeBuilder} from "@axe-core/playwright";
import {expect, test} from "@playwright/test";
import {execFileSync} from "node:child_process";
import {randomUUID} from "node:crypto";
import {mkdtempSync, rmSync, writeFileSync, mkdirSync} from "node:fs";
import {tmpdir} from "node:os";
import path from "node:path";
import pg from "pg";

declare global {
  interface Window {
    liveSocket: {connect(): void; disconnect(): void};
  }
}

const databaseUrl =
  process.env.HEPHAESTUS_E2E_DATABASE_URL ??
  "postgres://postgres:postgres@127.0.0.1:55432/hephaestus";
const repositoryRoot = process.env.HEPHAESTUS_REPOSITORY_ROOT;
const gitUrl = process.env.HEPHAESTUS_GIT_URL ?? "http://127.0.0.1:8080";
const oidcUrl = process.env.HEPHAESTUS_OIDC_URL ?? "http://127.0.0.1:5556";
const secretSentinel = "HEPHAESTUS_BROWSER_SECRET_4d7ccf";
const fixtureBuilderReference =
  "registry.example/hephaestus/ubuntu-native@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const registryAuthority = "registry.e2e.invalid";
const registryPrivateSentinel = "registry-private-callback-never-render";
let browserJourneyBuild: {repositoryId: string; id: string} | undefined;
let registryJourney:
  | {
      builderId: string;
      projectId: string;
      publicationId: string;
      immutableReference: string;
      evidenceReferences: string[];
    }
  | undefined;

test.describe.serial("release, instance, secret, and live-review product journey", () => {
  test("creates a repository, pushes agent.toml, and publishes its built release", async ({
    page
  }) => {
    const fixture = await loadFixture();
    await page.context().clearCookies();
    await signIn(page);

    const suffix = Date.now().toString(36);
    const projectName = `browser-build-${suffix}`;
    const repositoryName = `release-source-${suffix}`;

    await page.goto(`/organizations/${fixture.organizationId}/projects/new`);
    await waitForLiveView(page);
    await page
      .locator("#create-project-form")
      .locator('input[name="project[name]"]')
      .fill(projectName);
    await page
      .locator("#create-project-form")
      .locator('textarea[name="project[description]"]')
      .fill("Real browser build journey fixture");
    await page.getByRole("button", {name: "Create project"}).click();
    await expect(page).toHaveURL(/\/projects\/[0-9a-f-]+$/);
    const projectId = page.url().split("/").at(-1)!;
    await waitForLiveView(page);

    await page.locator("#create-repository-link").click();
    await waitForLiveView(page);
    await page
      .locator("#create-repository-form")
      .locator('input[name="repository[name]"]')
      .fill(repositoryName);
    await page.getByRole("button", {name: "Create repository"}).click();
    await expect(page).toHaveURL(/\/repositories\/[0-9a-f-]+$/);
    const repositoryId = page.url().split("/").at(-1)!;

    const sourceCommit = pushCommit(repositoryId, "build browser release", false, true);
    const build = await waitForBuild(repositoryId, sourceCommit);
    browserJourneyBuild = {repositoryId, id: build.id};
    const release = await waitForDraftRelease(build.id);
    writeJourneyEvidence({
      organizationId: fixture.organizationId,
      projectId,
      repositoryId,
      buildId: build.id,
      releaseId: release.id,
      releaseAgentId: release.release_agent_id,
      sourceCommit
    });

    await page.goto(`/repositories/${repositoryId}/builds/${build.id}`);
    await waitForLiveView(page);
    await expect(page.getByText("Agent release build", {exact: true})).toBeVisible();
    await expect(page.locator("#build-provenance")).toContainText(sourceCommit);
    await expect(page.getByRole("main").getByText("succeeded", {exact: true}).first()).toBeVisible();
    await captureJourneyScreenshot(page, "01-build-detail.png");

    await page.goto(`/repositories/${repositoryId}/releases/${release.id}`);
    await waitForLiveView(page);
    const review = page.locator("#release-draft-review");
    await expect(review).toBeVisible();
    await review.locator('input[name="release[version]"]').fill("v1.0.0");
    await review.getByRole("button", {name: "Save draft version"}).click();
    page.once("dialog", dialog => dialog.accept());
    await review.getByRole("button", {name: "Publish release"}).click();
    await expect(page.locator("#release-draft-review")).toHaveCount(0);
    await expect(page.getByRole("main").getByText("published", {exact: true})).toBeVisible();
    await captureJourneyScreenshot(page, "02-published-release.png");

    await page.goto(`/projects/${projectId}/agents`);
    await waitForLiveView(page);
    const importForm = page.locator(`#import-agent-${release.release_agent_id}`);
    await expect(importForm).toBeVisible();
    await importForm.locator('input[name="import[name]"]').fill("browser-built-agent");
    await importForm.getByRole("button", {name: "Import as new instance"}).click();
    await expect(page).toHaveURL(/\/projects\/[0-9a-f-]+\/agents\/[0-9a-f-]+$/);
    await waitForLiveView(page);
    await expect(page.getByRole("main")).toContainText("browser-built-agent");
    await captureJourneyScreenshot(page, "03-imported-agent.png");
  });

  test("requires authentication for the catalog and project authorization for repository builders", async ({
    page,
    request
  }) => {
    const fixture = await loadFixture();
    await signIn(page);
    await ensureFixtureBuilderCatalogImage();
    await page.goto("/builders");
    await waitForLiveView(page);
    await expect(page.getByRole("main")).toContainText("Browser fixture Ubuntu builder");
    await page.goto(`/projects/${fixture.projectId}/builders`);
    await waitForLiveView(page);
    await expect(page.locator("#project-builder-list")).toContainText(
      "No repository builders were discovered in committed configuration."
    );

    await page.context().clearCookies();
    await signIn(page, "outsider");
    await page.goto("/builders");
    await waitForLiveView(page);
    await expect(page.getByRole("main")).toContainText("Browser fixture Ubuntu builder");

    await page.goto(`/projects/${fixture.projectId}/builders`);
    await waitForLiveView(page);
    await expect(page).toHaveURL(/\/organizations$/);

    const anonymousCatalog = await request.get("/builders", {maxRedirects: 0});
    expect(anonymousCatalog.status()).toBe(302);
    expect(anonymousCatalog.headers().location).toBe("/login");
  });

  test("fixture-backed: shows an authorized verified registry publication without transport internals", async ({
    page
  }) => {
    const fixture = await loadFixture();
    registryJourney = await seedVerifiedRegistryJourney(fixture);

    await signIn(page);
    await page.goto(`/projects/${registryJourney.projectId}/builders`);
    await waitForLiveView(page);

    const builder = page.locator(`#project-builder-${registryJourney.builderId}`);
    await expect(builder).toBeVisible();
    await expect(builder).toContainText("Browser registry builder");
    await expect(builder).toContainText("ready");
    await expect(builder).toContainText("verified");
    await expect(builder).toContainText("unavailable");
    await expect(builder).toContainText(registryJourney.immutableReference);
    await expect(builder).toContainText("amd64");
    await expect(builder).toContainText("not_required");
    for (const reference of registryJourney.evidenceReferences) {
      await expect(builder).toContainText(reference);
    }

    // The browser only receives the registry's safe immutable projection, not
    // preparation attestation internals or any publication transport material.
    await expect(page.locator("body")).not.toContainText(registryPrivateSentinel);
    await assertAccessible(page, "main");
  });

  test("fixture-backed: denies an outsider the project registry projection", async ({page}) => {
    const journey = registryJourney;
    expect(journey).toBeDefined();

    await signIn(page, "outsider");
    await page.goto(`/projects/${journey!.projectId}/builders`);
    await waitForLiveView(page);

    await expect(page).toHaveURL(/\/organizations$/);
    await expect(page.locator("body")).not.toContainText(journey!.immutableReference);
    for (const reference of journey!.evidenceReferences) {
      await expect(page.locator("body")).not.toContainText(reference);
    }
  });

  test("fixture-backed: refreshes approval and missing registry diagnostics across an event-stream reconnect", async ({
    page
  }) => {
    const journey = registryJourney;
    expect(journey).toBeDefined();

    await signIn(page);
    await page.goto(`/projects/${journey!.projectId}/builders`);
    await waitForLiveView(page);
    const builder = page.locator(`#project-builder-${journey!.builderId}`);
    const eventsBeforeApproval = await countRegistryPublicationChangedEvents(
      journey!.publicationId
    );
    await approveRegistryPublication(journey!.publicationId);
    await expect
      .poll(() => countRegistryPublicationChangedEvents(journey!.publicationId))
      .toBe(eventsBeforeApproval + 1);
    await expect(builder).toContainText("approved");
    await expect(builder).toContainText("available");

    // Commit the lifecycle change while the page's event stream is disconnected.
    // Reconnecting must replay it from the durable product-event cursor and refresh
    // the projection rather than relying on periodic UI polling.
    const eventsBefore = await countRegistryPublicationChangedEvents(journey!.publicationId);
    await page.evaluate(() => window.liveSocket.disconnect());
    await markRegistryPublicationMissing(journey!.publicationId);
    await expect
      .poll(() => countRegistryPublicationChangedEvents(journey!.publicationId))
      .toBe(eventsBefore + 1);
    await page.evaluate(() => window.liveSocket.connect());
    await waitForLiveView(page);

    await expect(builder).toContainText("missing");
    await expect(builder).toContainText("unavailable");
    await expect(builder).toContainText(journey!.immutableReference);
    for (const reference of journey!.evidenceReferences) {
      await expect(builder).toContainText(reference);
    }
  });

  test("reviews and publishes seeded draft releases through the durable UI workflow", async ({
    page
  }) => {
    const fixture = await loadFixture();
    await signIn(page);

    for (const releaseId of fixture.releaseIds) {
      await page.goto(`/repositories/${fixture.repositoryId}/releases/${releaseId}`);
      await waitForLiveView(page);

      const review = page.locator("#release-draft-review");
      await expect(review).toBeVisible();
      const version = review.locator('input[name="release[version]"]');
      const currentVersion = await version.inputValue();
      const chosenVersion = currentVersion || "v1.0.0";
      await version.fill(chosenVersion);
      await review.getByRole("button", {name: "Save draft version"}).click();
      await expect(review).toBeVisible();

      page.once("dialog", dialog => dialog.accept());
      await review.getByRole("button", {name: "Publish release"}).click();
      await expect(page.locator("#release-draft-review")).toHaveCount(0);
      await expect(page.locator("#release-page-state")).toHaveCount(0);
      await expect(page.getByRole("main").getByText("published", {exact: true})).toBeVisible();
    }
  });

  test("shows build history, build detail, and published release provenance", async ({
    page
  }) => {
    const fixture = await loadFixture();
    await signIn(page);

    await page.goto(`/repositories/${fixture.repositoryId}/builds`);
    await waitForLiveView(page);
    await expect(page.getByRole("heading", {name: "Build history"})).toBeVisible();
    await expect(page.locator("#builds article")).toHaveCount(fixture.buildIds.length);
    await expect(page.locator("#builds")).toContainText("succeeded");

    await page.goto(
      `/repositories/${fixture.repositoryId}/builds/${fixture.buildIds[0]}`
    );
    await waitForLiveView(page);
    await expect(page.getByText("Agent release build", {exact: true})).toBeVisible();
    await expect(page.locator("#build-provenance")).toContainText("refs/heads/main");
    await expect(page.getByRole("main").getByText("succeeded", {exact: true}).first()).toBeVisible();
    await expect(page.locator("#build-logs")).toContainText("No logs were returned.");

    await page.goto(`/repositories/${fixture.repositoryId}/releases`);
    await waitForLiveView(page);
    await expect(page.locator("#releases article")).toHaveCount(fixture.releaseIds.length);
    await expect(page.locator("#releases")).toContainText("published");

    await page.goto(
      `/repositories/${fixture.repositoryId}/releases/${fixture.releaseIds[0]}`
    );
    await waitForLiveView(page);
    await expect(page.locator("#release-provenance")).toBeVisible();
    await expect(page.locator("#release-artifacts article")).toHaveCount(1);
    await expect(page.locator("#release-agents article")).toHaveCount(1);
  });

  test("shows a completed immutable-input verification mismatch", async ({page}) => {
    const fixture = await loadFixture();
    await signIn(page);

    const successfulBuild = browserJourneyBuild ?? {
      repositoryId: fixture.repositoryId,
      id: await verifiableBuildId()
    };
    const eventCount = await countBuildChangedEvents(successfulBuild.id);
    await seedVerificationMismatch(successfulBuild.id);
    await expect.poll(() => countBuildChangedEvents(successfulBuild.id)).toBe(eventCount + 1);

    await page.goto(`/repositories/${successfulBuild.repositoryId}/builds/${successfulBuild.id}`);
    await waitForLiveView(page);
    const verifications = page.locator("#build-verifications");
    await expect(verifications).toContainText("Verification mismatch");
    await expect(verifications).toContainText(
      "The rebuilt artifact manifest differs from the immutable release manifest."
    );
    await expect(verifications).toContainText("expected/agent.wasm");
    await expect(verifications).toContainText("actual/agent.wasm");
  });

  test("shows failed retries, verification requests, and LiveView reconnect recovery", async ({
    page
  }) => {
    const fixture = await loadFixture();
    const failedBuildId = await seedFailedBuild(fixture);
    await signIn(page);

    await page.goto(`/repositories/${fixture.repositoryId}/builds/${failedBuildId}`);
    await waitForLiveView(page);
    await expect(
      page.getByRole("main").getByText("failed", {exact: true}).first()
    ).toBeVisible();
    await expect(page.locator("#build-provenance")).toContainText("fixture_build_failed");
    await expect(page.getByRole("button", {name: "Retry attempt"})).toBeVisible();
    await page.getByRole("button", {name: "Retry attempt"}).click();
    await expect(page.getByText("Build retry queued.")).toBeVisible();
    await expect.poll(() => countOutboxEvents(failedBuildId, "build.retry_requested.v1")).toBe(1);

    const successfulBuild = browserJourneyBuild ?? {
      repositoryId: fixture.repositoryId,
      id: await verifiableBuildId()
    };
    await page.goto(`/repositories/${successfulBuild.repositoryId}/builds/${successfulBuild.id}`);
    await waitForLiveView(page);
    await expect(
      page.getByRole("button", {name: "Rebuild for verification"})
    ).toBeVisible();

    page.once("dialog", dialog => dialog.accept());
    await page.getByRole("button", {name: "Rebuild for verification"}).click();
    await expect(page.getByText("Verification rebuild queued.")).toBeVisible();
    await expect
      .poll(() => countOutboxEvents(successfulBuild.id, "build.verify_requested.v1"))
      .toBe(1);

    await page.evaluate(() => window.liveSocket.disconnect());
    await page.evaluate(() => window.liveSocket.connect());
    await waitForLiveView(page);
    await expect(page.locator("#build-provenance")).toContainText(successfulBuild.id);
  });

  test("ready, empty, form, and error states are accessible", async ({
    page
  }) => {
    const fixture = await loadFixture();
    await signIn(page);

    await expect(page.getByRole("main")).toBeVisible();
    await expect(page.getByRole("heading", {level: 1})).toBeVisible();
    await assertAccessible(page, "main");

    await page.goto(`/projects/${fixture.projectId}/runs`);
    await waitForLiveView(page);
    const projectNavigation = page.getByRole("navigation", {name: "Project"});
    await expect(projectNavigation.getByRole("link", {name: "Runs"})).toHaveAttribute(
      "aria-current",
      "page"
    );
    await expect(page.locator("#project-run-stream")).toContainText(
      "No exact runs have been created."
    );
    await assertAccessible(page, "main");

    const agentsLink = projectNavigation.getByRole("link", {name: "Agents"});
    await agentsLink.focus();
    await expect(agentsLink).toBeFocused();
    await agentsLink.press("Enter");
    await expect(page).toHaveURL(new RegExp(`/projects/${fixture.projectId}/agents$`));
    await waitForLiveView(page);

    await page.goto(`/organizations/${fixture.organizationId}/secrets/new`);
    await waitForLiveView(page);
    const secretForm = page.locator("#create-organization-secret");
    await expect(secretForm.getByLabel("Secret name")).toBeVisible();
    await expect(secretForm.getByLabel("New value")).toHaveAttribute("type", "password");
    await expect(secretForm.getByLabel("Allowed delivery modes")).toBeVisible();
    await assertAccessible(page, "#create-organization-secret");

    await page.goto("/runs/00000000-0000-0000-0000-000000000000");
    await expect(page).toHaveURL(/\/organizations$/);
    await waitForLiveView(page);
    await expect(page.getByRole("alert")).toContainText("Run not found or access was revoked.");
    await assertAccessible(page, "#flash-error");
  });

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

  test("confirmation controls are keyboard reachable and preserve focus", async ({page}) => {
    const fixture = await loadFixture();
    const instanceId = await latestInstanceId(fixture.projectId);
    await signIn(page);
    await page.goto(`/projects/${fixture.projectId}/agents/${instanceId}`);
    await waitForLiveView(page);

    const remove = page.getByRole("button", {name: "Remove"}).first();
    await remove.focus();
    await expect(remove).toBeFocused();

    const confirmation = new Promise<string>(resolve => {
      page.once("dialog", async dialog => {
        resolve(dialog.message());
        await dialog.dismiss();
      });
    });

    await remove.press("Enter");
    await expect(confirmation).resolves.toContain(
      "Remove this attachment while retaining historical run provenance?"
    );
    await expect(remove).toBeFocused();
    await assertAccessible(page, "main");
  });
});

async function signIn(page: import("@playwright/test").Page, account = "reviewer") {
  await page.goto("/");
  await page.getByTestId("oidc-login").click();
  await page.locator('input[name="login"]').fill(account);
  await page.getByRole("button", {name: "Continue as Ada Reviewer"}).click();
  await expect(page).toHaveURL(/\/organizations$/);
  await waitForLiveView(page);
}

async function waitForLiveView(page: import("@playwright/test").Page) {
  await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
}

async function assertAccessible(page: import("@playwright/test").Page, selector: string) {
  const results = await new AxeBuilder({page})
    .include(selector)
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();

  expect(
    results.violations,
    results.violations
      .map(violation => `${violation.id}: ${violation.help} (${violation.nodes.length})`)
      .join("\n")
  ).toEqual([]);
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
    `SELECT release.id AS release_id, release_agent.id
     FROM release_agents release_agent
     JOIN releases release ON release.id = release_agent.release_id
     WHERE release.repository_id = $1
     ORDER BY release.version`,
    [result.rows[0].repository_id]
  );
  await releaseClient.end();
  const builds = await queryBuilds(result.rows[0].repository_id);
  return {
    organizationId: result.rows[0].organization_id,
    projectId: result.rows[0].project_id,
    repositoryId: result.rows[0].repository_id,
    releaseIds: releases.rows.map(row => row.release_id),
    releaseAgents: releases.rows.map(row => row.id),
    buildIds: builds.map(row => row.id),
    sourceCommit: builds[0].source_commit
  };
}

async function queryBuilds(repositoryId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT id, source_commit
     FROM build_requests
     WHERE repository_id = $1
     ORDER BY created_at, id`,
    [repositoryId]
  );
  await client.end();
  return result.rows as Array<{id: string; source_commit: string}>;
}

async function verifiableBuildId() {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT request.id
       FROM build_requests request
       JOIN build_executions execution ON execution.build_request_id = request.id
      WHERE request.state = 'succeeded' AND execution.state = 'drafted'
      ORDER BY request.created_at DESC, request.id DESC
      LIMIT 1`
  );
  await client.end();
  expect(result.rows).toHaveLength(1);
  return result.rows[0].id as string;
}

async function seedFailedBuild(fixture: Awaited<ReturnType<typeof loadFixture>>) {
  const buildId = randomUUID();
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  await client.query(
    `INSERT INTO build_requests
       (id, repository_id, source_commit, source_ref, build_definition_hash,
        state, build_trigger, agent_key, build_declaration, build_policy,
        declared_artifacts, started_at, completed_at)
     VALUES ($1, $2, $3, 'refs/heads/main', $4, 'failed', 'manual',
             'browser-reviewer', '{}'::jsonb, '{}'::jsonb, '[]'::jsonb,
             now() - interval '1 second', now())`,
    [buildId, fixture.repositoryId, fixture.sourceCommit, Buffer.alloc(32, 17)]
  );
  await client.query(
    `INSERT INTO build_executions
       (build_request_id, vm_id, release_id, release_agent_id, release_version,
        state, failure_code, logs, started_at, completed_at)
     VALUES ($1, $2, $3, $4, 'fixture-failed', 'failed', 'fixture_build_failed',
             '[{"stream":"stderr","text":"fixture build failed"}]'::jsonb,
             now() - interval '1 second', now())`,
    [buildId, `fixture-failed-${buildId}`, randomUUID(), randomUUID()]
  );
  await client.end();
  return buildId;
}

async function seedVerificationMismatch(buildId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  await client.query(
    `INSERT INTO build_verifications
       (id, build_request_id, state, expected_manifest, actual_manifest,
        failure_code, created_at, completed_at)
     VALUES ($1, $2, 'failed', $3::jsonb, $4::jsonb, 'manifest_mismatch',
             now() - interval '1 second', now())`,
    [
      randomUUID(),
      buildId,
      JSON.stringify([{path: "expected/agent.wasm", content_hash: "expected"}]),
      JSON.stringify([{path: "actual/agent.wasm", content_hash: "actual"}])
    ]
  );
  await client.end();
}

async function seedVerifiedRegistryJourney(
  fixture: Awaited<ReturnType<typeof loadFixture>>
) {
  // The normal UI/RPC/event path is real; only Zot's external OCI graph is
  // represented here by already-verified control-plane evidence.
  const builderId = randomUUID();
  const namespaceId = randomUUID();
  const publicationId = randomUUID();
  const outputDigest = `sha256:${"b".repeat(64)}`;
  const contextDigest = `sha256:${"c".repeat(64)}`;
  const platformDigest = `sha256:${"d".repeat(64)}`;
  const evidenceDigests = ["e", "f", "1"].map(character =>
    `sha256:${character.repeat(64)}`
  );
  const repositoryPath =
    `projects/${fixture.projectId}/repository-builders/${builderId}`;
  const immutableReference = `${registryAuthority}/${repositoryPath}@${outputDigest}`;
  const evidenceReferences = evidenceDigests.map(
    digest => `${registryAuthority}/${repositoryPath}@${digest}`
  );
  const builderKey = `e2e-registry-${builderId.slice(0, 8)}`;
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  try {
    await client.query("BEGIN");
    await client.query(
      `INSERT INTO project_builder_definitions
         (id, project_id, source_repository_id, key, display_name, source_revision,
          dockerfile_path, context_path, context_digest, approved_base_image_reference,
          status, oci_image_reference, oci_image_digest, provenance)
       VALUES ($1, $2, $3, $4, 'Browser registry builder', $5,
               'builders/browser/Dockerfile', '.', $6, $7,
               'ready', $8, $9, $10::jsonb)`,
      [
        builderId,
        fixture.projectId,
        fixture.repositoryId,
        builderKey,
        fixture.sourceCommit,
        contextDigest,
        "fixture-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        immutableReference,
        outputDigest,
        JSON.stringify({
          source_revision: fixture.sourceCommit,
          context_digest: contextDigest,
          attestation_reference: registryPrivateSentinel,
          sbom_reference: registryPrivateSentinel
        })
      ]
    );
    await client.query(
      `INSERT INTO registry_namespaces
         (id, repository_path, owner_kind, owner_id, project_id)
       VALUES ($1, $2, 'repository_builder', $3, $4)`,
      [namespaceId, repositoryPath, builderId, fixture.projectId]
    );
    await client.query(
      `INSERT INTO registry_publications
         (id, namespace_id, owner_kind, owner_id, project_id, registry_authority,
          expected_digest, expected_media_type, expected_size, policy_version)
       VALUES ($1, $2, 'repository_builder', $3, $4, $5,
               $6, 'application/vnd.oci.image.index.v1+json', 1024, 'browser-e2e/v1')`,
      [
        publicationId,
        namespaceId,
        builderId,
        fixture.projectId,
        registryAuthority,
        outputDigest
      ]
    );
    await client.query(
      `INSERT INTO registry_publication_platforms
         (publication_id, digest, size, media_type, operating_system, architecture)
       VALUES ($1, $2, 512, 'application/vnd.oci.image.manifest.v1+json', 'linux', 'amd64')`,
      [publicationId, platformDigest]
    );
    for (const [index, kind] of ["sbom", "provenance", "scan"].entries()) {
      await client.query(
        `INSERT INTO registry_publication_evidence
           (publication_id, kind, subject_digest, digest, size, media_type, artifact_type)
         VALUES ($1, $2, $3, $4, 256, 'application/vnd.oci.artifact.manifest.v1+json', $5)`,
        [
          publicationId,
          kind,
          outputDigest,
          evidenceDigests[index],
          `application/vnd.hephaestus.${kind}.v1`
        ]
      );
    }
    await client.query(
      "UPDATE registry_publications SET state = 'verified', verified_at = now() WHERE id = $1",
      [publicationId]
    );
    await client.query("COMMIT");
  } catch (error) {
    await client.query("ROLLBACK");
    throw error;
  } finally {
    await client.end();
  }
  return {
    builderId,
    projectId: fixture.projectId,
    publicationId,
    immutableReference,
    evidenceReferences
  };
}

async function approveRegistryPublication(publicationId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  try {
    const result = await client.query(
      "UPDATE registry_publications SET state = 'approved', approved_at = now() WHERE id = $1 AND state = 'verified'",
      [publicationId]
    );
    expect(result.rowCount).toBe(1);
  } finally {
    await client.end();
  }
}

async function markRegistryPublicationMissing(publicationId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  try {
    const result = await client.query(
      "UPDATE registry_publications SET state = 'missing' WHERE id = $1 AND state = 'approved'",
      [publicationId]
    );
    expect(result.rowCount).toBe(1);
  } finally {
    await client.end();
  }
}

async function countRegistryPublicationChangedEvents(publicationId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  try {
    const result = await client.query(
      `SELECT count(*)::integer AS count
         FROM application_events
        WHERE aggregate_type = 'registry_publication'
          AND aggregate_id = $1
          AND event_type = 'registry.publication_changed'`,
      [publicationId]
    );
    return result.rows[0].count as number;
  } finally {
    await client.end();
  }
}

async function countOutboxEvents(buildId: string, eventType: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT count(*)::integer AS count
       FROM outbox
      WHERE aggregate_id = $1 AND event_type = $2`,
    [buildId, eventType]
  );
  await client.end();
  return result.rows[0].count as number;
}

async function countBuildChangedEvents(buildId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT count(*)::integer AS count
       FROM application_events
      WHERE aggregate_type = 'build' AND aggregate_id = $1 AND event_type = 'build.changed'`,
    [buildId]
  );
  await client.end();
  return result.rows[0].count as number;
}

async function ensureFixtureBuilderCatalogImage() {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  await client.query(
    `INSERT INTO builder_images
       (id, key, display_name, image_reference, toolchains, architectures,
        preparation_state, availability_state, network_ceiling, max_vcpus,
        max_memory_mib, dependency_policy, provenance, platform_policy_version)
     VALUES ($1, 'e2e-ubuntu-native', 'Browser fixture Ubuntu builder', $2,
             '[{"name":"shell","version":"fixture"}]'::jsonb,
             ARRAY['amd64'], 'ready', 'available', 'disabled', 2, 512,
             'vendored_offline', '{"source":"browser-fixture"}'::jsonb,
             'browser-e2e/v1')
     ON CONFLICT (key) DO NOTHING`,
    [randomUUID(), fixtureBuilderReference]
  );
  await client.end();
}

async function latestInstanceId(projectId: string) {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT id
     FROM agent_instances
     WHERE project_id = $1
     ORDER BY created_at DESC, id DESC
     LIMIT 1`,
    [projectId]
  );
  await client.end();
  return result.rows[0].id as string;
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

function pushCommit(
  repositoryId: string,
  message: string,
  clone = false,
  includeBuild = false
) {
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
    writeFileSync(path.join(work, "agent.toml"), agentConfig(includeBuild));
    git(["add", "."], work);
    git(["commit", "-m", message], work);
    const sourceCommit = git(["rev-parse", "HEAD"], work);
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
    return sourceCommit;
  } finally {
    rmSync(work, {recursive: true, force: true});
  }
}

function git(arguments_: string[], cwd = process.cwd()) {
  return execFileSync("git", arguments_, {cwd, encoding: "utf8"}).trim();
}

function agentConfig(includeBuild = false) {
  if (includeBuild) return buildAgentConfig();

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
reference = "fixture-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
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

function buildAgentConfig() {
  return `
version = 2
[agent]
name = "browser-built-agent"
key = "browser-built-agent"
[build]
command = "/bin/sh"
arguments = ["-c", "mkdir -p /workspace/output/reports && printf 'built browser artifact\\n' > /workspace/output/reports/result.txt"]
working_directory = "/workspace/source"
root_image = "fixture-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 128
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "reports/result.txt"
kind = "file"
media_type = "text/plain"
[guest]
command = "bin/browser-built-agent"
arguments = []
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 128
[root_image]
reference = "fixture-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
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

async function waitForBuild(repositoryId: string, sourceCommit: string) {
  await expect
    .poll(
      async () => {
        const client = new pg.Client({connectionString: databaseUrl});
        await client.connect();
        const result = await client.query(
          `SELECT id, state
           FROM build_requests
           WHERE repository_id = $1 AND source_commit = $2
           ORDER BY created_at DESC LIMIT 1`,
          [repositoryId, sourceCommit]
        );
        await client.end();
        return result.rows[0] ?? null;
      },
      {timeout: 90_000, intervals: [250, 500, 1_000, 2_000]}
    )
    .toEqual(expect.objectContaining({state: "succeeded"}));

  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT id, state
     FROM build_requests
     WHERE repository_id = $1 AND source_commit = $2
     ORDER BY created_at DESC LIMIT 1`,
    [repositoryId, sourceCommit]
  );
  await client.end();
  return result.rows[0] as {id: string; state: string};
}

async function waitForDraftRelease(buildId: string) {
  await expect
    .poll(
      async () => {
        const client = new pg.Client({connectionString: databaseUrl});
        await client.connect();
        const result = await client.query(
          `SELECT release.id, release_agent.id AS release_agent_id, release.state
           FROM releases release
           JOIN release_agents release_agent ON release_agent.release_id = release.id
           WHERE release.build_request_id = $1
           ORDER BY release.created_at DESC LIMIT 1`,
          [buildId]
        );
        await client.end();
        return result.rows[0] ?? null;
      },
      {timeout: 90_000, intervals: [250, 500, 1_000, 2_000]}
    )
    .toEqual(expect.objectContaining({state: "draft"}));

  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(
    `SELECT release.id, release_agent.id AS release_agent_id, release.state
     FROM releases release
     JOIN release_agents release_agent ON release_agent.release_id = release.id
     WHERE release.build_request_id = $1
     ORDER BY release.created_at DESC LIMIT 1`,
    [buildId]
  );
  await client.end();
  return result.rows[0] as {id: string; release_agent_id: string; state: string};
}

function writeJourneyEvidence(ids: Record<string, string>) {
  const directory = process.env.HEPHAESTUS_E2E_EVIDENCE_DIR;
  if (!directory) return;
  mkdirSync(directory, {recursive: true});
  writeFileSync(
    path.join(directory, "real-build-journey-ids.json"),
    `${JSON.stringify(ids, null, 2)}\n`
  );
}

async function captureJourneyScreenshot(
  page: import("@playwright/test").Page,
  filename: string
) {
  const directory = process.env.HEPHAESTUS_E2E_EVIDENCE_DIR;
  if (!directory) return;
  mkdirSync(directory, {recursive: true});
  await page.screenshot({path: path.join(directory, filename), fullPage: true});
}
