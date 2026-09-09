import assert from "node:assert/strict";
import test from "node:test";
import { AsyncQueue, Revision } from "./asyncQueue.ts";

test("same owner saves use the last accepted state and recover after failure", async () => {
  const queue = new AsyncQueue<string>();
  let state = { a: false, b: false };
  const first = queue.enqueue("settings", async () => {
    state = { ...state, a: true };
  });
  const failed = queue.enqueue("settings", async () => {
    throw new Error("disk full");
  });
  const recovered = queue.enqueue("settings", async () => {
    state = { ...state, b: true };
  });
  await assert.rejects(failed, /disk full/);
  await Promise.all([first, recovered, queue.drain()]);
  assert.deepEqual(state, { a: true, b: true });
});

test("independent pods can progress while one pod is waiting", async () => {
  const queue = new AsyncQueue<number>();
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => {
    release = resolve;
  });
  const one = queue.enqueue(1, () => blocked);
  const two = queue.enqueue(2, async () => "saved");
  assert.equal(await two, "saved");
  release();
  await one;
  await queue.drain();
});

test("stale completions cannot clear a newer request", () => {
  const revision = new Revision();
  const first = revision.next();
  const second = revision.next();
  assert.equal(revision.isCurrent(first), false);
  assert.equal(revision.isCurrent(second), true);
});
