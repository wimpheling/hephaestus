const TEST_IDS = new Map([
  ["cooking installed UI TLS full-page and managed iframe smoke", "installed_ui_tls_smoke"],
]);

const STAGE_IDS = new Map([
  ["signin", "signin"],
  ["static-launch", "static_launch"],
  ["static-cookie", "static_cookie"],
  ["static-accessibility", "static_accessibility"],
  ["static-theme", "static_theme"],
  ["managed-launch", "managed_launch"],
  ["managed-cookie", "managed_cookie"],
  ["managed-identity", "managed_identity"],
  ["theme", "theme"],
  ["accessibility", "accessibility"],
  ["close", "close"],
]);

const SAFE_STATUSES = new Set(["passed", "failed", "skipped", "interrupted", "timedOut"]);

/**
 * Minimal reporter for the installed UI proof.
 *
 * It intentionally has no stdout/stderr, error, attachment, URL, or raw
 * title handling. The installed UI test must use the fixed test title above
 * and may use only the fixed test.step labels in STAGE_IDS.
 */
export class SafeInstalledUiReporter {
  constructor(options = {}) {
    this.output = options.output ?? process.stdout;
    this.counts = {passed: 0, failed: 0, skipped: 0, other: 0};
  }

  onBegin(_config, suite) {
    this.write({event: "run_started", test_count: suite.allTests().length});
  }

  onStepEnd(_test, _result, step) {
    const stageId = STAGE_IDS.get(step.title);
    if (!stageId) return;
    this.write({event: "stage", stage_id: stageId, status: step.error ? "failed" : "passed"});
  }

  onTestEnd(test, result) {
    const testId = TEST_IDS.get(test.title) ?? "unknown_test";
    const status = normalizeStatus(result.status);
    if (status === "passed") this.counts.passed += 1;
    else if (status === "failed") this.counts.failed += 1;
    else if (status === "skipped") this.counts.skipped += 1;
    else this.counts.other += 1;

    this.write({
      event: "test",
      test_id: testId,
      status,
      duration_ms: finiteNonNegative(result.duration),
      retry: finiteNonNegative(result.retry),
    });
  }

  onEnd(result) {
    this.write({event: "run_finished", status: normalizeStatus(result.status), counts: this.counts});
  }

  // Deliberately discard all untrusted worker output and errors.
  onError() {}
  onStdErr() {}
  onStdOut() {}

  write(record) {
    this.output.write(`${JSON.stringify(record)}\n`);
  }
}

function normalizeStatus(status) {
  if (SAFE_STATUSES.has(status)) return status === "timedOut" ? "timed_out" : status;
  return "other";
}

function finiteNonNegative(value) {
  return Number.isFinite(value) && value >= 0 ? Math.floor(value) : 0;
}

export default SafeInstalledUiReporter;
