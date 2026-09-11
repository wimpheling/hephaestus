import {expect, test} from "@playwright/test";
import {waitForNewUpdateId} from "./helpers/wait-for-new-update-id.js";

test("browser helper preserves an observed update ID across a stream reset", async ({page}) => {
  await page.setContent(
    '<section id="instance-updates"><article id="instance-update-existing"></article><article id="instance-update-new">compatibility_unknown</article></section>'
  );
  await page.evaluate(() => {
    const updates = document.querySelector("#instance-updates");
    const transient = document.querySelector("#instance-update-new");
    if (!(updates instanceof HTMLElement) || !(transient instanceof HTMLElement)) {
      throw new Error("stream reset fixture did not mount");
    }
    Object.defineProperty(transient, "id", {
      configurable: true,
      get() {
        Object.defineProperty(transient, "id", {
          value: "instance-update-new",
          configurable: true
        });
        updates.replaceChildren();
        return "instance-update-new";
      }
    });
  });

  const observed = await waitForNewUpdateId(page, new Set(["instance-update-existing"]));
  expect(observed).toBe("instance-update-new");
  await page.evaluate(() => {
    const updates = document.querySelector("#instance-updates");
    if (!(updates instanceof HTMLElement)) throw new Error("stream reset fixture disappeared");
    const replacement = document.createElement("article");
    replacement.id = "instance-update-new";
    replacement.textContent = "compatibility_unknown";
    updates.append(replacement);
  });
  await expect(page.locator(`#${observed}`)).toBeVisible();
  await expect(page.locator(`#${observed}`)).toContainText("compatibility_unknown");
});
