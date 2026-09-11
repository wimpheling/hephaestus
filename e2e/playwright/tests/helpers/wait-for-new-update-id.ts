import {expect, type Page} from "@playwright/test";

export async function waitForNewUpdateId(
  page: Page,
  existingUpdateIds: Set<string>
): Promise<string> {
  let observedUpdateId = "";
  await expect.poll(async () => {
    const ids = await page.locator("#instance-updates article").evaluateAll(articles =>
      articles.map(article => article.id)
    );
    observedUpdateId = ids.find(id => !existingUpdateIds.has(id)) ?? "";
    return observedUpdateId;
  }, {timeout: 60_000}).toMatch(/.+/);
  return observedUpdateId;
}
