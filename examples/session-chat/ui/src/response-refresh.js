const DEFAULT_POLL_INTERVAL_MS = 1_000;
const DEFAULT_RESPONSE_TIMEOUT_MS = 30_000;

/**
 * Serializes local Git operations and waits for release-owned responses.
 *
 * Refreshes use the current checkout so polling does not create an unbounded
 * collection of reconnect directories. A manual reconnect remains available
 * to replace that checkout when the user explicitly asks for one.
 */
export class ResponseRefreshController {
  constructor({
    client,
    onSession,
    onStatus,
    pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
    responseTimeoutMs = DEFAULT_RESPONSE_TIMEOUT_MS,
    now = () => Date.now(),
    setTimer = (...args) => globalThis.setTimeout(...args),
    clearTimer = (timer) => globalThis.clearTimeout(timer),
  }) {
    this.client = client;
    this.onSession = onSession;
    this.onStatus = onStatus;
    this.pollIntervalMs = pollIntervalMs;
    this.responseTimeoutMs = responseTimeoutMs;
    this.now = now;
    this.setTimer = setTimer;
    this.clearTimer = clearTimer;
    this.operation = Promise.resolve();
    this.pending = new Map();
    this.pollTimer = undefined;
    this.activeAbortController = undefined;
    this.disposed = false;
  }

  enqueue(operation) {
    if (this.disposed) return Promise.reject(new Error("session chat is closed"));
    const next = this.operation.then(() => {
      if (this.disposed) throw new Error("session chat is closed");
      return operation();
    });
    this.operation = next.catch(() => undefined);
    return next;
  }

  async publish(record) {
    const result = await this.enqueue(async () => {
      const accepted = await this.client.appendHuman(record);
      const session = await this.client.readSession();
      this.observe(session);
      return accepted;
    });
    this.waitFor(record.record_id);
    return result;
  }

  refresh({ reconnect = false, signal } = {}) {
    return this.enqueue(async () => {
      const session = reconnect
        ? await this.client.reconnect()
        : await this.client.fetch({ signal });
      this.observe(session);
      return session;
    });
  }

  waitFor(recordId) {
    if (this.disposed || this.pending.has(recordId)) return;
    this.pending.set(recordId, this.now());
    this.onStatus?.("Waiting for the assistant response…", "waiting");
    this.schedulePoll(0);
  }

  observe(session) {
    if (this.disposed) return;
    this.onSession?.(session);
    const completed = [...this.pending.keys()].filter((recordId) =>
      session.records.some(
        (record) =>
          record.kind === "assistant_message" && record.in_reply_to === recordId,
      ),
    );
    for (const recordId of completed) this.pending.delete(recordId);
    if (completed.length > 0) {
      this.onStatus?.("Assistant response received", "completed");
    } else if (this.pending.size === 0) {
      this.onStatus?.(`Connected as user:${session.actorId}`, "ready");
    }
    if (this.pending.size === 0) this.cancelPoll();
  }

  schedulePoll(delay) {
    if (this.disposed || this.pending.size === 0 || this.pollTimer !== undefined) return;
    this.pollTimer = this.setTimer(() => {
      this.pollTimer = undefined;
      void this.poll();
    }, delay);
  }

  async poll() {
    if (this.disposed || this.pending.size === 0) return;
    const expired = [...this.pending.entries()].filter(
      ([, startedAt]) => this.now() - startedAt >= this.responseTimeoutMs,
    );
    for (const [recordId] of expired) this.pending.delete(recordId);
    if (expired.length > 0) {
      this.onStatus?.("The assistant response timed out; reconnect to check again", "timeout");
    }
    if (this.pending.size === 0) return;

    const remaining = Math.min(
      ...[...this.pending.values()].map((startedAt) => this.responseTimeoutMs - (this.now() - startedAt)),
    );
    const abortController = typeof AbortController === "function" ? new AbortController() : undefined;
    this.activeAbortController = abortController;
    let deadlineTimer;
    if (abortController) deadlineTimer = this.setTimer(() => abortController.abort(), Math.max(0, remaining));
    try {
      await this.refresh({ signal: abortController?.signal });
    } catch (error) {
      if (this.disposed) return;
      if (abortController?.signal.aborted) {
        this.pending.clear();
        this.onStatus?.("The assistant response timed out; reconnect to check again", "timeout");
        return;
      }
      this.pending.clear();
      this.onStatus?.(
        error instanceof Error ? `Response refresh failed: ${error.message}` : "Response refresh failed",
        "error",
      );
      return;
    } finally {
      if (deadlineTimer !== undefined) this.clearTimer(deadlineTimer);
      if (this.activeAbortController === abortController) this.activeAbortController = undefined;
    }
    this.schedulePoll(this.pollIntervalMs);
  }

  cancelPoll() {
    if (this.pollTimer === undefined) return;
    this.clearTimer(this.pollTimer);
    this.pollTimer = undefined;
  }

  dispose() {
    this.disposed = true;
    this.pending.clear();
    this.cancelPoll();
    this.activeAbortController?.abort();
    this.activeAbortController = undefined;
  }
}

export { DEFAULT_POLL_INTERVAL_MS, DEFAULT_RESPONSE_TIMEOUT_MS };
