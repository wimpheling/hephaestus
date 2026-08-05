# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: e2e/playwright/tests/live-review.spec.ts >> release, instance, secret, and live-review product journey >> loads a project repository once and opens it without an unavailable redirect
- Location: e2e/playwright/tests/live-review.spec.ts:39:3

# Error details

```
Error: page.goto: Protocol error (Page.navigate): Cannot navigate to invalid URL
Call log:
  - navigating to "/", waiting until "load"

```

# Test source

```ts
  502 |     await expect(page.getByTestId("review-proposal")).toContainText("approved");
  503 | 
  504 |     const proposal = await latestProposal(fixture.repositoryId);
  505 |     expect(proposal.state).toBe("approved");
  506 |     expect(
  507 |       git([
  508 |         `--git-dir=${path.join(repositoryRoot!, `${fixture.repositoryId}.git`)}`,
  509 |         "rev-parse",
  510 |         proposal.target_ref
  511 |       ])
  512 |     ).toBe(proposal.result_commit);
  513 | 
  514 |     await page.goto(`/projects/${fixture.projectId}/agents/${instanceId}`);
  515 |     await waitForLiveView(page);
  516 |     const activeRevision = page.locator("#instance-overview article").first().locator("strong");
  517 |     const activeBefore = (await activeRevision.textContent())?.trim();
  518 |     await page
  519 |       .locator("#create-update")
  520 |       .locator('select[name="update[release_agent_id]"]')
  521 |       .selectOption(fixture.releaseAgents[1]);
  522 |     await page
  523 |       .locator("#create-update")
  524 |       .locator('select[name="update[parameters][review_style]"]')
  525 |       .selectOption("balanced");
  526 |     await page
  527 |       .locator("#create-update")
  528 |       .locator('input[name="update[parameters][private_hint]"]')
  529 |       .fill("replacement-sensitive-parameter");
  530 |     await page.getByRole("button", {name: "Start reviewed update"}).click();
  531 |     await expect(page.locator("#instance-updates")).toContainText("activated");
  532 |     await expect(activeRevision).not.toHaveText(activeBefore!);
  533 | 
  534 |     await page
  535 |       .locator("#create-update")
  536 |       .locator('select[name="update[release_agent_id]"]')
  537 |       .selectOption(fixture.releaseAgents[2]);
  538 |     await page
  539 |       .locator("#create-update")
  540 |       .locator('select[name="update[parameters][review_style]"]')
  541 |       .selectOption("strict");
  542 |     await page
  543 |       .locator("#create-update")
  544 |       .locator('input[name="update[parameters][private_hint]"]')
  545 |       .fill("uncertain-update-parameter");
  546 |     await page.getByRole("button", {name: "Start reviewed update"}).click();
  547 |     await expect(page.locator("#instance-updates")).toContainText("compatibility_unknown");
  548 |     await expect(page.getByText("run gate closed")).toBeVisible();
  549 |     page.once("dialog", dialog => dialog.accept());
  550 |     await page.getByRole("button", {name: "Reject candidate"}).click();
  551 |     await expect(page.getByText("run gate open")).toBeVisible();
  552 | 
  553 |     await assertNoSecretSentinel(page, browserMessages);
  554 |     await verifySecretProvenance(instanceId);
  555 |   });
  556 | 
  557 |   test("a reviewer can reject a second durable result", async ({page}) => {
  558 |     const fixture = await loadFixture();
  559 |     await signIn(page);
  560 |     await page.goto(`/projects/${fixture.projectId}/runs`);
  561 |     await waitForLiveView(page);
  562 |     pushCommit(fixture.repositoryId, "second browser run", true);
  563 | 
  564 |     const runRows = page
  565 |       .locator("#project-run-stream [id^='project-run-']")
  566 |       .filter({hasText: "agent-workbench"});
  567 |     await expect(runRows).toHaveCount(2);
  568 |     await runRows.first().click();
  569 |     await expect(page.getByTestId("review-proposal")).toBeVisible();
  570 |     await page.getByTestId("reject-result").click();
  571 |     await expect(page.getByTestId("review-proposal")).toContainText("rejected");
  572 |   });
  573 | 
  574 |   test("confirmation controls are keyboard reachable and preserve focus", async ({page}) => {
  575 |     const fixture = await loadFixture();
  576 |     const instanceId = await latestInstanceId(fixture.projectId);
  577 |     await signIn(page);
  578 |     await page.goto(`/projects/${fixture.projectId}/agents/${instanceId}`);
  579 |     await waitForLiveView(page);
  580 | 
  581 |     const remove = page.getByRole("button", {name: "Remove"}).first();
  582 |     await remove.focus();
  583 |     await expect(remove).toBeFocused();
  584 | 
  585 |     const confirmation = new Promise<string>(resolve => {
  586 |       page.once("dialog", async dialog => {
  587 |         resolve(dialog.message());
  588 |         await dialog.dismiss();
  589 |       });
  590 |     });
  591 | 
  592 |     await remove.press("Enter");
  593 |     await expect(confirmation).resolves.toContain(
  594 |       "Remove this attachment while retaining historical run provenance?"
  595 |     );
  596 |     await expect(remove).toBeFocused();
  597 |     await assertAccessible(page, "main");
  598 |   });
  599 | });
  600 | 
  601 | async function signIn(page: import("@playwright/test").Page, account = "reviewer") {
> 602 |   await page.goto("/");
      |              ^ Error: page.goto: Protocol error (Page.navigate): Cannot navigate to invalid URL
  603 |   await page.getByTestId("oidc-login").click();
  604 |   await page.locator('input[name="login"]').fill(account);
  605 |   await page.getByRole("button", {name: "Continue as Ada Reviewer"}).click();
  606 |   await expect(page).toHaveURL(/\/organizations$/);
  607 |   await waitForLiveView(page);
  608 | }
  609 | 
  610 | async function waitForLiveView(page: import("@playwright/test").Page) {
  611 |   await expect(page.locator("[data-phx-main].phx-connected")).toBeVisible();
  612 | }
  613 | 
  614 | async function assertAccessible(page: import("@playwright/test").Page, selector: string) {
  615 |   const results = await new AxeBuilder({page})
  616 |     .include(selector)
  617 |     .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
  618 |     .analyze();
  619 | 
  620 |   expect(
  621 |     results.violations,
  622 |     results.violations
  623 |       .map(violation => `${violation.id}: ${violation.help} (${violation.nodes.length})`)
  624 |       .join("\n")
  625 |   ).toEqual([]);
  626 | }
  627 | 
  628 | function escapeRegExp(value: string) {
  629 |   return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  630 | }
  631 | 
  632 | async function loadFixture() {
  633 |   const client = new pg.Client({connectionString: databaseUrl});
  634 |   await client.connect();
  635 |   const result = await client.query(`
  636 |     SELECT organization.id AS organization_id, project.id AS project_id,
  637 |            repository.id AS repository_id
  638 |     FROM organizations organization
  639 |     JOIN projects project ON project.organization_id = organization.id
  640 |     JOIN repositories repository ON repository.project_id = project.id
  641 |     WHERE organization.name = 'Acme Research'
  642 |       AND repository.name = 'agent-workbench'
  643 |   `);
  644 |   await client.end();
  645 |   const releaseClient = new pg.Client({connectionString: databaseUrl});
  646 |   await releaseClient.connect();
  647 |   const releases = await releaseClient.query(
  648 |     `SELECT release.id AS release_id, release_agent.id
  649 |      FROM release_agents release_agent
  650 |      JOIN releases release ON release.id = release_agent.release_id
  651 |      WHERE release.repository_id = $1
  652 |      ORDER BY release.version`,
  653 |     [result.rows[0].repository_id]
  654 |   );
  655 |   await releaseClient.end();
  656 |   const builds = await queryBuilds(result.rows[0].repository_id);
  657 |   return {
  658 |     organizationId: result.rows[0].organization_id,
  659 |     projectId: result.rows[0].project_id,
  660 |     repositoryId: result.rows[0].repository_id,
  661 |     releaseIds: releases.rows.map(row => row.release_id),
  662 |     releaseAgents: releases.rows.map(row => row.id),
  663 |     buildIds: builds.map(row => row.id),
  664 |     sourceCommit: builds[0].source_commit
  665 |   };
  666 | }
  667 | 
  668 | async function queryBuilds(repositoryId: string) {
  669 |   const client = new pg.Client({connectionString: databaseUrl});
  670 |   await client.connect();
  671 |   const result = await client.query(
  672 |     `SELECT id, source_commit
  673 |      FROM build_requests
  674 |      WHERE repository_id = $1
  675 |      ORDER BY created_at, id`,
  676 |     [repositoryId]
  677 |   );
  678 |   await client.end();
  679 |   return result.rows as Array<{id: string; source_commit: string}>;
  680 | }
  681 | 
  682 | async function verifiableBuildId() {
  683 |   const client = new pg.Client({connectionString: databaseUrl});
  684 |   await client.connect();
  685 |   const result = await client.query(
  686 |     `SELECT request.id
  687 |        FROM build_requests request
  688 |        JOIN build_executions execution ON execution.build_request_id = request.id
  689 |       WHERE request.state = 'succeeded' AND execution.state = 'drafted'
  690 |       ORDER BY request.created_at DESC, request.id DESC
  691 |       LIMIT 1`
  692 |   );
  693 |   await client.end();
  694 |   expect(result.rows).toHaveLength(1);
  695 |   return result.rows[0].id as string;
  696 | }
  697 | 
  698 | async function seedFailedBuild(fixture: Awaited<ReturnType<typeof loadFixture>>) {
  699 |   const buildId = randomUUID();
  700 |   const client = new pg.Client({connectionString: databaseUrl});
  701 |   await client.connect();
  702 |   await client.query(
```