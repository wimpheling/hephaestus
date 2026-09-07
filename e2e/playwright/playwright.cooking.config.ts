import base from "./playwright.config.js";

export default {
  ...base,
  testDir: "./cooking-tests",
  outputDir: process.env.HEPHAESTUS_E2E_EVIDENCE_DIR,
};
