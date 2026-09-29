pub type MailboxTimeoutEvidence = (
    i32,
    uuid::Uuid,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);
pub const GOLDEN_AGENT: &str = r#"#!/bin/sh
  set -eu
  if test -r /run/hephaestus/mailbox-event.json; then
      if grep -q '"route":"/gateway/golden-proof"' /run/hephaestus/mailbox-event.json; then
          test "$(cat /run/hephaestus/mailbox-body)" = "gateway-real-mailbox-body"
      else
          grep -q '"route":"/mailbox/golden-proof"' /run/hephaestus/mailbox-event.json
          test "$(cat /run/hephaestus/mailbox-body)" = "golden-real-mailbox-body"
      fi
      printf 'mailbox-body-ok\n' > /var/lib/hephaestus/golden-state
      exit 0
  fi
  if test -x /usr/libexec/hephaestus/integration-check \
      && test -r /run/hephaestus-secrets/.runtime-credential; then
      exec /usr/libexec/hephaestus/integration-check --brokered-https-e2e
  fi
  test -r /workspace/repo/input.txt
  test -r /run/hephaestus/parameters.json
  test -r /run/hephaestus/context.json
  if printf 'forbidden\n' > /release/write-must-fail 2>/dev/null; then
      exit 91
  fi
  if printf 'forbidden\n' > /workspace/repo/write-must-fail 2>/dev/null; then
      exit 92
  fi
  if printf 'forbidden\n' > /run/hephaestus/write-must-fail 2>/dev/null; then
      exit 93
  fi
  printf 'state-ok\n' > /var/lib/hephaestus/golden-state
  test "$(cat /var/lib/hephaestus/golden-state)" = "state-ok"
  printf 'agent edit\n' > /workspace/work/input.txt
  printf 'durable report\n' > /workspace/work/reports/result.txt
  "#;
