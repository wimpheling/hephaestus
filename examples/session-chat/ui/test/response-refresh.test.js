import test from "node:test";
import assert from "node:assert/strict";
import { ResponseRefreshController } from "../src/response-refresh.js";

const HUMAN = "123e4567-e89b-12d3-a456-426614174000";
const RECORD_ID = "123e4567-e89b-12d3-a456-426614174001";

function record(id = RECORD_ID) {
  return { record_id: id };
}

function timerHarness() {
  const timers = [];
  return {
    timers,
    setTimer(callback, delay) {
      const timer = { callback, delay, cancelled: false };
      timers.push(timer);
      return timer;
    },
    clearTimer(timer) {
      timer.cancelled = true;
    },
    async runNext() {
      let timer;
      do timer = timers.shift(); while (timer?.cancelled);
      assert.ok(timer, "expected a scheduled timer");
      timer.callback();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    },
  };
}

function session(records = []) {
  return { records, transcript: records, actorId: HUMAN };
}

test("polling refreshes the current checkout and completes on the correlated response", async () => {
  const timers = timerHarness();
  const statuses = [];
  let remote = session([record()]);
  let fetches = 0;
  const client = {
    async appendHuman(value) { return value; },
    async readSession() { return session([record()]); },
    async fetch() { fetches += 1; return remote; },
    async reconnect() { throw new Error("manual reconnect was not expected"); },
  };
  const controller = new ResponseRefreshController({
    client,
    onStatus: (message, state) => statuses.push({ message, state }),
    pollIntervalMs: 10,
    responseTimeoutMs: 100,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });

  await controller.publish(record());
  assert.equal(statuses.at(-1).state, "waiting");
  await timers.runNext();
  assert.equal(fetches, 1);
  assert.equal(statuses.at(-1).state, "waiting");

  remote = session([
    record(),
    { kind: "assistant_message", in_reply_to: RECORD_ID },
  ]);
  await timers.runNext();
  assert.equal(fetches, 2);
  assert.equal(statuses.at(-1).state, "completed");
  assert.equal(timers.timers.filter((timer) => !timer.cancelled).length, 0);
});

test("serialized publishing keeps concurrent local Git mutations ordered", async () => {
  let releaseFirst;
  const firstAppend = new Promise((resolve) => { releaseFirst = resolve; });
  const calls = [];
  let readCount = 0;
  const client = {
    async appendHuman(value) {
      calls.push(value.record_id);
      if (calls.length === 1) await firstAppend;
      return value;
    },
    async readSession() {
      readCount += 1;
      return session([]);
    },
    async fetch() { return session([]); },
  };
  const timers = timerHarness();
  const controller = new ResponseRefreshController({
    client,
    pollIntervalMs: 10,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  const second = record("123e4567-e89b-12d3-a456-426614174002");
  const firstPromise = controller.publish(record());
  const secondPromise = controller.publish(second);
  await Promise.resolve();
  assert.deepEqual(calls, [RECORD_ID]);
  releaseFirst();
  await firstPromise;
  await Promise.resolve();
  await secondPromise;
  assert.deepEqual(calls, [RECORD_ID, second.record_id]);
  assert.equal(readCount, 2);
  controller.dispose();
});

test("failed publication does not render or start a false response wait", async () => {
  const timers = timerHarness();
  let rendered = 0;
  const controller = new ResponseRefreshController({
    client: {
      async appendHuman() { throw new Error("push rejected"); },
      async readSession() { rendered += 1; return session([]); },
      async fetch() { return session([]); },
    },
    onSession: () => { rendered += 1; },
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  await assert.rejects(controller.publish(record()), /push rejected/);
  assert.equal(rendered, 0);
  assert.equal(timers.timers.filter((timer) => !timer.cancelled).length, 0);
});

test("response polling reports timeout and stops scheduling", async () => {
  const timers = timerHarness();
  const statuses = [];
  let now = 0;
  const controller = new ResponseRefreshController({
    client: {
      async appendHuman(value) { return value; },
      async readSession() { return session([]); },
      async fetch() { return session([]); },
    },
    onStatus: (message, state) => statuses.push({ message, state }),
    pollIntervalMs: 10,
    responseTimeoutMs: 10,
    now: () => now,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  await controller.publish(record());
  await timers.runNext();
  now = 10;
  await timers.runNext();
  assert.equal(statuses.at(-1).state, "timeout");
  assert.equal(timers.timers.filter((timer) => !timer.cancelled).length, 0);
});

test("a failed refresh reports an error and does not schedule another poll", async () => {
  const timers = timerHarness();
  const statuses = [];
  const controller = new ResponseRefreshController({
    client: {
      async appendHuman(value) { return value; },
      async readSession() { return session([]); },
      async fetch() { throw new Error("temporary Git failure"); },
    },
    onStatus: (message, state) => statuses.push({ message, state }),
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  await controller.publish(record());
  await timers.runNext();
  assert.equal(statuses.at(-1).state, "error");
  assert.equal(timers.timers.filter((timer) => !timer.cancelled).length, 0);
});

test("disposing during an in-flight refresh aborts it without rendering afterward", async () => {
  const timers = timerHarness();
  const statuses = [];
  let rendered = 0;
  let aborted = false;
  const controller = new ResponseRefreshController({
    client: {
      async appendHuman(value) { return value; },
      async readSession() { return session([]); },
      async fetch({ signal }) {
        await new Promise((resolve, reject) => {
          signal.addEventListener("abort", () => { aborted = true; reject(new Error("aborted")); }, { once: true });
        });
      },
    },
    onSession: () => { rendered += 1; },
    onStatus: (message, state) => statuses.push({ message, state }),
    responseTimeoutMs: 100,
    setTimer: timers.setTimer,
    clearTimer: timers.clearTimer,
  });
  await controller.publish(record());
  await timers.runNext();
  controller.dispose();
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(aborted, true);
  assert.equal(rendered, 1);
  assert.equal(statuses.some(({ state }) => state === "error" || state === "timeout"), false);
});
