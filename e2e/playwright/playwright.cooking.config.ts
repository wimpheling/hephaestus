import base from "./playwright.config.js";

export default {
  ...base,
  testDir: "./cooking-tests",
  // Keep the legacy Cooking runner's module graph separate from the installed
  // UI runner.  Installed-UI specs validate their namespace/control contract at
  // module load, so a grep for a legacy title must not import them during
  // discovery.
  testMatch: ["cooking-live-review.spec.ts", "cooking-post-operation.spec.ts"],
  outputDir: process.env.HEPHAESTUS_E2E_EVIDENCE_DIR,
};
