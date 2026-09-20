import assert from "node:assert/strict";
import test from "node:test";
import {SafeInstalledUiReporter} from "../safe-installed-ui-reporter.mjs";

function recorder() {
  const chunks = [];
  return {
    output: {write: chunk => chunks.push(chunk)},
    text: () => chunks.join(""),
  };
}

test("safe reporter emits only fixed IDs and bounded result fields", () => {
  const sink = recorder();
  const reporter = new SafeInstalledUiReporter({output: sink.output});
  const fragment = "a".repeat(43);
  const bootstrapUrl = `https://generation.example/_heph/bootstrap#${fragment}`;
  assert.equal(fragment.length, 43);
  const cookie = "__Host-hephaestus_ui=secret-cookie-value";

  reporter.onBegin({}, {allTests: () => [{title: "cooking installed UI TLS full-page and managed iframe smoke"}]});
  reporter.onStepBegin(
    {title: "cooking installed UI TLS full-page and managed iframe smoke"},
    {},
    {title: "signin-redirect"},
  );
  reporter.onStepBegin(
    {title: "cooking installed UI TLS full-page and managed iframe smoke"},
    {},
    {title: `dynamic-step-${fragment}`},
  );
  reporter.onStepEnd(
    {title: "cooking installed UI TLS full-page and managed iframe smoke"},
    {},
    {title: `dynamic-step-${fragment}`, error: new Error(cookie)},
  );
  reporter.onStepEnd(
    {title: "cooking installed UI TLS full-page and managed iframe smoke"},
    {},
    {title: "static-ui-cookie-presence", error: undefined},
  );
  reporter.onTestEnd(
    {
      title: `malicious title ${fragment} ${cookie}`,
      location: {file: `/tmp/${fragment}.ts`},
    },
    {
      status: "failed",
      duration: 12.8,
      retry: 0,
      error: {message: `request URL ${bootstrapUrl}; cookie ${cookie}`},
      attachments: [{name: "trace", path: `${bootstrapUrl}/${cookie}.zip`}],
    },
  );
  reporter.onError({message: `raw failure ${bootstrapUrl} ${cookie}`});
  reporter.onStdErr(Buffer.from(`stderr ${bootstrapUrl} ${cookie}`));
  reporter.onStdOut(`stdout ${bootstrapUrl} ${cookie}`);
  reporter.onEnd({status: "failed"});

  const output = sink.text();
  assert.doesNotMatch(output, new RegExp(fragment));
  assert.equal(output.includes(cookie), false);
  assert.equal(output.includes("/_heph/bootstrap#"), false);
  assert.equal(output.includes("malicious title"), false);
  assert.equal(output.includes("request URL"), false);
  assert.equal(output.includes("/tmp/"), false);

  const records = output.trim().split("\n").map(JSON.parse);
  assert.deepEqual(records, [
    {event: "run_started", test_count: 1},
    {event: "stage", stage_id: "signin_redirect", status: "pending"},
    {event: "stage", stage_id: "static_ui_cookie_presence", status: "passed"},
    {event: "test", test_id: "unknown_test", status: "failed", duration_ms: 12, retry: 0},
    {event: "run_finished", status: "failed", counts: {passed: 0, failed: 1, skipped: 0, other: 0}},
  ]);
});

test("known installed test title maps to a fixed ID", () => {
  const sink = recorder();
  const reporter = new SafeInstalledUiReporter({output: sink.output});
  reporter.onTestEnd(
    {title: "cooking installed UI TLS full-page and managed iframe smoke"},
    {status: "passed", duration: 1, retry: 0},
  );

  assert.deepEqual(JSON.parse(sink.text()), {
    event: "test",
    test_id: "installed_ui_tls_smoke",
    status: "passed",
    duration_ms: 1,
    retry: 0,
  });
});
