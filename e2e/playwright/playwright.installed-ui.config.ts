import base from "./playwright.config.js";

export default {
  ...base,
  testDir: "./cooking-tests",
  testMatch: "cooking-installed-ui.spec.ts",
  // Keep Playwright's automatic error-context artifact outside the mounted
  // evidence tree. Only the two explicit screenshots below are retained.
  outputDir: "/tmp/heph-installed-ui-playwright-private",
  reporter: [["./safe-installed-ui-reporter.mjs"]],
  use: {
    ...base.use,
    screenshot: "off",
    // Handoff fragments and HttpOnly child cookies are request-only material;
    // retaining a Playwright trace on failure would make them durable evidence.
    trace: "off",
    video: "off",
  },
};
