import {defineConfig, devices} from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  timeout: 90_000,
  expect: {timeout: 30_000},
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["line"], ["html", {open: "never"}]] : "line",
  use: {
    baseURL: process.env.HEPHAESTUS_WEB_URL ?? "http://127.0.0.1:4000",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    video: "retain-on-failure"
  },
  projects: [
    {
      name: "chromium",
      use: {...devices["Desktop Chrome"]}
    }
  ]
});
