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

test.describe.serial("live review golden path", () => {
  test("push appears live, result is inspected, and approval fast-forwards", async ({
    page
  }) => {
    const fixture = await loadFixture();

    await page.goto("/");
    await page.getByTestId("oidc-login").click();
    await expect(page).toHaveURL(/127\.0\.0\.1:5556\/authorize/);
    await page.locator('input[name="login"]').fill("reviewer");
    await page.getByRole("button", {name: "Continue as Ada Reviewer"}).click();
    await expect(page).toHaveURL(/\/organizations$/);

    await page.getByTestId(`organization-${fixture.organizationId}`).click();
    await page.getByTestId(`repository-${fixture.repositoryId}`).click();
    await expect(page.locator("#runs .empty-state")).toContainText("No runs yet");

    pushCommit(fixture.repositoryId, "first browser run");

    const runRow = page.locator('[data-testid^="run-"]').first();
    await expect(runRow).toBeVisible();
    await runRow.click();
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
  });

  test("a reviewer can reject a second durable result", async ({page}) => {
    const fixture = await loadFixture();
    await signIn(page);
    await page.goto(`/repositories/${fixture.repositoryId}`);
    pushCommit(fixture.repositoryId, "second browser run", true);

    const runRows = page.locator('[data-testid^="run-"]');
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
}

async function loadFixture() {
  const client = new pg.Client({connectionString: databaseUrl});
  await client.connect();
  const result = await client.query(`
    SELECT organization.id AS organization_id, repository.id AS repository_id
    FROM organizations organization
    JOIN projects project ON project.organization_id = organization.id
    JOIN repositories repository ON repository.project_id = project.id
    WHERE organization.name = 'Acme Research'
      AND repository.name = 'agent-workbench'
  `);
  await client.end();
  return {
    organizationId: result.rows[0].organization_id,
    repositoryId: result.rows[0].repository_id
  };
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
