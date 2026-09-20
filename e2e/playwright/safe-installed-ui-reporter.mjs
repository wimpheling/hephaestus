const TEST_IDS = new Map([
  ["cooking installed UI TLS full-page and managed iframe smoke", "installed_ui_tls_smoke"],
]);

const STAGE_IDS = new Map([
  ["signin", "signin"],
  ["signin-platform", "signin_platform"],
  ["signin-redirect", "signin_redirect"],
  ["signin-account", "signin_account"],
  ["signin-return", "signin_return"],
  ["static-launch", "static_launch"],
  ["static-ui-cookie-presence", "static_ui_cookie_presence"],
  ["static-ui-cookie-secure", "static_ui_cookie_secure"],
  ["static-ui-cookie-http-only", "static_ui_cookie_http_only"],
  ["static-ui-cookie-same-site", "static_ui_cookie_same_site"],
  ["static-ui-cookie-path", "static_ui_cookie_path"],
  ["static-ui-cookie-host-domain", "static_ui_cookie_host_domain"],
  ["static-platform-cookie-presence", "static_platform_cookie_presence"],
  ["static-platform-cookie-security", "static_platform_cookie_security"],
  ["static-platform-cookie-host-domain", "static_platform_cookie_host_domain"],
  ["static-cookie-isolation", "static_cookie_isolation"],
  ["static-accessibility", "static_accessibility"],
  ["static-theme", "static_theme"],
  ["managed-launch", "managed_launch"],
  ["managed-card-click", "managed_card_click"],
  ["managed-frame-visible", "managed_frame_visible"],
  ["managed-frame-route", "managed_frame_route"],
  ["managed-frame-csp", "managed_frame_csp"],
  ["managed-document-response", "managed_document_response"],
  ["managed-document-2xx", "managed_document_2xx"],
  ["managed-document-3xx", "managed_document_3xx"],
  ["managed-document-4xx", "managed_document_4xx"],
  ["managed-document-500", "managed_document_500"],
  ["managed-document-502", "managed_document_502"],
  ["managed-document-503", "managed_document_503"],
  ["managed-document-504", "managed_document_504"],
  ["managed-document-5xx-other", "managed_document_5xx_other"],
  ["managed-document-other", "managed_document_other"],
  ["managed-document-headers", "managed_document_headers"],
  ["managed-document-content", "managed_document_content"],
  ["managed-cookie", "managed_cookie"],
  ["managed-cookie-isolation", "managed_cookie_isolation"],
  ["managed-identity", "managed_identity"],
  ["lifecycle-disable", "lifecycle_disable"],
  ["lifecycle-managed-ready", "lifecycle_managed_ready"],
  ["lifecycle-disable-complete", "lifecycle_disable_complete"],
  ["lifecycle-stale-baseline-200", "lifecycle_stale_baseline_200"],
  ["lifecycle-stale-baseline-other", "lifecycle_stale_baseline_other"],
  ["lifecycle-stale-baseline-cookie", "lifecycle_stale_baseline_cookie"],
  ["lifecycle-stale-fetch-response", "lifecycle_stale_fetch_response"],
  ["lifecycle-stale-fetch-401", "lifecycle_stale_fetch_401"],
  ["lifecycle-stale-fetch-403", "lifecycle_stale_fetch_403"],
  ["lifecycle-stale-fetch-404", "lifecycle_stale_fetch_404"],
  ["lifecycle-stale-fetch-410", "lifecycle_stale_fetch_410"],
  ["lifecycle-stale-fetch-200", "lifecycle_stale_fetch_200"],
  ["lifecycle-stale-fetch-other", "lifecycle_stale_fetch_other"],
  ["lifecycle-stale-request-observed", "lifecycle_stale_request_observed"],
  ["lifecycle-stale-cookie-present", "lifecycle_stale_cookie_present"],
  ["lifecycle-disabled-card", "lifecycle_disabled_card"],
  ["lifecycle-reactivate-complete", "lifecycle_reactivate_complete"],
  ["lifecycle-old-generation-fetch-response", "lifecycle_old_generation_fetch_response"],
  ["lifecycle-old-generation-fetch-401", "lifecycle_old_generation_fetch_401"],
  ["lifecycle-old-generation-fetch-403", "lifecycle_old_generation_fetch_403"],
  ["lifecycle-old-generation-fetch-404", "lifecycle_old_generation_fetch_404"],
  ["lifecycle-old-generation-fetch-410", "lifecycle_old_generation_fetch_410"],
  ["lifecycle-old-generation-fetch-200", "lifecycle_old_generation_fetch_200"],
  ["lifecycle-old-generation-fetch-other", "lifecycle_old_generation_fetch_other"],
  ["lifecycle-old-generation-request-observed", "lifecycle_old_generation_request_observed"],
  ["lifecycle-old-generation-cookie-present", "lifecycle_old_generation_cookie_present"],
  ["lifecycle-old-generation-denied", "lifecycle_old_generation_denied"],
  ["lifecycle-old-generation-denial-verified", "lifecycle_old_generation_denial_verified"],
  ["lifecycle-reactivated-launch", "lifecycle_reactivated_launch"],
  ["lifecycle-reactivated-frame-visible", "lifecycle_reactivated_frame_visible"],
  ["lifecycle-reactivated-frame-route", "lifecycle_reactivated_frame_route"],
  ["lifecycle-reactivated-generation-different", "lifecycle_reactivated_generation_different"],
  ["lifecycle-reactivated-document-response", "lifecycle_reactivated_document_response"],
  ["lifecycle-reactivated-document-url", "lifecycle_reactivated_document_url"],
  ["lifecycle-reactivated-document-2xx", "lifecycle_reactivated_document_2xx"],
  ["lifecycle-reactivated-document-other", "lifecycle_reactivated_document_other"],
  ["lifecycle-reactivated-bootstrap-fragment", "lifecycle_reactivated_bootstrap_fragment"],
  ["lifecycle-reactivated-cookie-present", "lifecycle_reactivated_cookie_present"],
  ["lifecycle-reactivated-identity", "lifecycle_reactivated_identity"],
  ["lifecycle-reactivated-frame-ready", "lifecycle_reactivated_frame_ready"],
  ["lifecycle-new-generation-ready", "lifecycle_new_generation_ready"],
  ["lifecycle-new-generation-verified", "lifecycle_new_generation_verified"],
  ["lifecycle-logout-baseline-response", "lifecycle_logout_baseline_response"],
  ["lifecycle-logout-baseline-200", "lifecycle_logout_baseline_200"],
  ["lifecycle-logout-baseline-other", "lifecycle_logout_baseline_other"],
  ["lifecycle-logout-baseline-cookie", "lifecycle_logout_baseline_cookie"],
  ["lifecycle-parent-revoke-ready", "lifecycle_parent_revoke_ready"],
  ["lifecycle-parent-revoke-permitted", "lifecycle_parent_revoke_permitted"],
  ["lifecycle-logout-click", "lifecycle_logout_click"],
  ["lifecycle-logout-signed-out", "lifecycle_logout_signed_out"],
  ["lifecycle-logout-fetch-response", "lifecycle_logout_fetch_response"],
  ["lifecycle-logout-fetch-401", "lifecycle_logout_fetch_401"],
  ["lifecycle-logout-fetch-403", "lifecycle_logout_fetch_403"],
  ["lifecycle-logout-fetch-404", "lifecycle_logout_fetch_404"],
  ["lifecycle-logout-fetch-410", "lifecycle_logout_fetch_410"],
  ["lifecycle-logout-fetch-200", "lifecycle_logout_fetch_200"],
  ["lifecycle-logout-fetch-other", "lifecycle_logout_fetch_other"],
  ["lifecycle-logout-request-observed", "lifecycle_logout_request_observed"],
  ["lifecycle-logout-cookie-present", "lifecycle_logout_cookie_present"],
  ["lifecycle-parent-revoked-denied", "lifecycle_parent_revoked_denied"],
  ["lifecycle-parent-revocation-verified", "lifecycle_parent_revocation_verified"],
  ["lifecycle-reauth-launch", "lifecycle_reauth_launch"],
  ["lifecycle-reauth-frame-visible", "lifecycle_reauth_frame_visible"],
  ["lifecycle-reauth-frame-route", "lifecycle_reauth_frame_route"],
  ["lifecycle-reauth-generation-same", "lifecycle_reauth_generation_same"],
  ["lifecycle-reauth-document-response", "lifecycle_reauth_document_response"],
  ["lifecycle-reauth-document-2xx", "lifecycle_reauth_document_2xx"],
  ["lifecycle-reauth-document-other", "lifecycle_reauth_document_other"],
  ["lifecycle-reauth-document-url", "lifecycle_reauth_document_url"],
  ["lifecycle-reauth-bootstrap-fragment", "lifecycle_reauth_bootstrap_fragment"],
  ["lifecycle-reauth-cookie-present", "lifecycle_reauth_cookie_present"],
  ["lifecycle-reauth-identity", "lifecycle_reauth_identity"],
  ["lifecycle-reauth-frame-ready", "lifecycle_reauth_frame_ready"],
  ["lifecycle-parent-new-child-ready", "lifecycle_parent_new_child_ready"],
  ["lifecycle-removed-baseline-response", "lifecycle_removed_baseline_response"],
  ["lifecycle-removed-baseline-200", "lifecycle_removed_baseline_200"],
  ["lifecycle-removed-baseline-other", "lifecycle_removed_baseline_other"],
  ["lifecycle-removed-baseline-cookie", "lifecycle_removed_baseline_cookie"],
  ["lifecycle-remove-ready", "lifecycle_remove_ready"],
  ["lifecycle-remove-complete", "lifecycle_remove_complete"],
  ["lifecycle-removed-fetch-response", "lifecycle_removed_fetch_response"],
  ["lifecycle-removed-fetch-401", "lifecycle_removed_fetch_401"],
  ["lifecycle-removed-fetch-403", "lifecycle_removed_fetch_403"],
  ["lifecycle-removed-fetch-404", "lifecycle_removed_fetch_404"],
  ["lifecycle-removed-fetch-410", "lifecycle_removed_fetch_410"],
  ["lifecycle-removed-fetch-200", "lifecycle_removed_fetch_200"],
  ["lifecycle-removed-fetch-other", "lifecycle_removed_fetch_other"],
  ["lifecycle-removed-request-observed", "lifecycle_removed_request_observed"],
  ["lifecycle-removed-cookie-present", "lifecycle_removed_cookie_present"],
  ["lifecycle-removed-host-denied", "lifecycle_removed_host_denied"],
  ["lifecycle-removed-card-absent", "lifecycle_removed_card_absent"],
  ["lifecycle-stale-cookie-denied", "lifecycle_stale_cookie_denied"],
  ["theme", "theme"],
  ["accessibility", "accessibility"],
  ["accessibility-status", "accessibility_status"],
  ["accessibility-frame-attributes", "accessibility_frame_attributes"],
  ["accessibility-platform-csp", "accessibility_platform_csp"],
  ["accessibility-undeclared-fetch", "accessibility_undeclared_fetch"],
  ["accessibility-parent-navigation", "accessibility_parent_navigation"],
  ["accessibility-platform-axe", "accessibility_platform_axe"],
  ["accessibility-frame-axe", "accessibility_frame_axe"],
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

  onStepBegin(_test, _result, step) {
    const stageId = STAGE_IDS.get(step.title);
    if (!stageId) return;
    this.write({event: "stage", stage_id: stageId, status: "pending"});
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
