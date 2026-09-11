import {expect, test} from "@playwright/test";
import {readFileSync} from "node:fs";
import {waitForNewUpdateId} from "../tests/helpers/wait-for-new-update-id.js";

type CookingPostFixture = {
  project_id: string;
  instance_id: string;
  completed_run_id: string;
  result_commit: string;
  result_ref: string;
  target_ref: string;
  proposal_id: string;
  pending_run_id: string;
  pending_proposal_id: string;
  retry_source_run_id: string;
  migration_update_id: string;
  abnormal_update_id: string;
  abnormal_candidate_revision_id: string;
  abnormal_release_agent_id: string;
  browser_model_rule_id: string;
  browser_relay_rule_id: string;
  browser_rule_copies: Array<{source_rule_id: string; candidate_rule_id: string}>;
};

const fixturePath = process.env.HEPHAESTUS_COOKING_BROWSER_FIXTURE;
const oidcUrl = process.env.HEPHAESTUS_OIDC_URL ?? "http://127.0.0.1:5556";

test("cooking post-operation controls, provenance, recovery, and denial", async ({page}, testInfo) => {
  const fixture = loadFixture();
  await signIn(page, "reviewer");

  await page.goto(`/runs/${fixture.completed_run_id}`);
  await waitForLiveView(page);
  await expect(page).toHaveURL(new RegExp(`/runs/${escapeRegExp(fixture.completed_run_id)}$`));
  await expect(page.locator("#run-exact-provenance")).toContainText("Release");
  await expect(page.getByTestId("review-proposal")).toContainText(fixture.result_commit.slice(0, 10));
  await expect(page.getByTestId("review-proposal")).toContainText(fixture.target_ref);
  await expect(page.getByTestId("run-timeline")).toContainText("result · completed");
  await expect(page.getByTestId("review-proposal")).toContainText("approved");
  await expect(page.locator("#result-diff")).not.toBeEmpty();
  await page.screenshot({path: testInfo.outputPath("cooking-approved-run.png"), fullPage: true});

  await page.goto(`/runs/${fixture.pending_run_id}`);
  await waitForLiveView(page);
  await expect(page.getByTestId("approve-result")).toBeVisible();
  await page.getByTestId("approve-result").click();
  await expect(page.getByRole("alert")).toContainText("Approval queued");
  await expect(page.getByTestId("review-proposal")).toContainText("approved", {timeout: 30_000});
  await page.screenshot({path: testInfo.outputPath("cooking-browser-approved-pending.png"), fullPage: true});

  await page.goto(`/projects/${fixture.project_id}/agents/${fixture.instance_id}`);
  await waitForLiveView(page);
  const migrationUpdate = page.locator(`#instance-update-${fixture.migration_update_id}`);
  await expect(migrationUpdate).toBeVisible();
  await expect(migrationUpdate).toContainText("activated");
  const abnormalUpdate = page.locator(`#instance-update-${fixture.abnormal_update_id}`);
  await expect(abnormalUpdate).toContainText("rejected");
  await expect(page.locator(`#update-hook-events-${fixture.abnormal_update_id}`)).toBeVisible();
  await expect(abnormalUpdate).toContainText(
    fixture.abnormal_candidate_revision_id.slice(0, 8)
  );
  // Submit one more update using the exact published abnormal candidate. The
  // hook is expected to pause in compatibility_unknown, after which the same
  // page performs the authorized recovery decision.
  await expect(page.locator("#create-update-panel")).toBeVisible();
  const existingUpdateIds = new Set(
    await page.locator("#instance-updates article").evaluateAll(articles =>
      articles.map(article => article.id)
    )
  );
  const updateForm = page.locator("#create-update");
  await updateForm.locator('select[name="update[release_agent_id]"]').selectOption(fixture.abnormal_release_agent_id);
  await updateForm.locator('input[name="update[parameters][model_rule_id]"]').fill(fixture.browser_model_rule_id);
  await updateForm.locator('input[name="update[parameters][relay_rule_id]"]').fill(fixture.browser_relay_rule_id);
  const addRuleCopy = updateForm.getByRole("button", {name: "Add rule copy"});
  for (let index = await updateForm.locator('input[name^="update[brokered_rule_copies]"][name$="[source_rule_id]"]').count(); index < fixture.browser_rule_copies.length; index++) {
    await addRuleCopy.click();
  }
  for (const [index, copy] of fixture.browser_rule_copies.entries()) {
    await updateForm
      .locator(`input[name="update[brokered_rule_copies][${index}][source_rule_id]"]`)
      .fill(copy.source_rule_id);
    await updateForm
      .locator(`input[name="update[brokered_rule_copies][${index}][candidate_rule_id]"]`)
      .fill(copy.candidate_rule_id);
  }
  await updateForm.getByRole("button", {name: "Start reviewed update"}).click();
  await expect(page.getByRole("alert")).toContainText("Candidate update created and reviewed.");
  const browserUpdateId = await waitForNewUpdateId(page, existingUpdateIds);
  const browserUpdate = page.locator(`#${browserUpdateId}`);
  await expect(browserUpdate).toBeVisible({timeout: 60_000});
  await expect(browserUpdate).toContainText("compatibility_unknown", {timeout: 60_000});
  const browserReject = browserUpdate.locator('button[id^="recover-reject-"]');
  await expect(browserReject).toBeVisible();
  page.once("dialog", dialog => dialog.accept());
  await browserReject.click();
  await expect(page.getByRole("alert")).toContainText("Authorized recovery decision recorded");
  await expect(page.locator(`#${browserUpdateId}`)).toContainText("rejected", {timeout: 30_000});

  // Exercise retry against the explicitly request-backed normal source. The
  // mailbox run above remains the independent approval/recovery source.
  await page.goto(`/runs/${fixture.retry_source_run_id}`);
  await waitForLiveView(page);
  await page.getByTestId("retry-run").click();
  await expect(page.getByRole("alert")).toContainText("Retry queued");
  await page.screenshot({path: testInfo.outputPath("cooking-update-recovery.png"), fullPage: true});

  // The local OIDC outsider is authenticated but has no project access.
  // A direct route request must not expose the completed run.
  await page.context().clearCookies();
  await signIn(page, "outsider");
  await page.goto(`/runs/${fixture.completed_run_id}`);
  await expect(page).toHaveURL(/\/organizations$/);
  await expect(page.getByRole("alert")).toContainText("not found or access was revoked");
  await page.screenshot({path: testInfo.outputPath("cooking-outsider-denied.png"), fullPage: true});
});

function loadFixture(): CookingPostFixture {
  if (!fixturePath) throw new Error("HEPHAESTUS_COOKING_BROWSER_FIXTURE is required");
  const fixture = JSON.parse(readFileSync(fixturePath, "utf8")) as CookingPostFixture;
  for (const key of [
    "project_id",
    "instance_id",
    "completed_run_id",
    "pending_run_id",
    "pending_proposal_id",
    "retry_source_run_id",
    "result_commit",
    "result_ref",
    "target_ref",
    "proposal_id",
    "migration_update_id",
    "abnormal_update_id",
    "abnormal_candidate_revision_id",
    "abnormal_release_agent_id",
    "browser_model_rule_id",
    "browser_relay_rule_id",
    "browser_rule_copies"
  ] as const) {
    if (!fixture[key]) throw new Error(`cooking post fixture is missing ${key}`);
  }
  return fixture;
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
