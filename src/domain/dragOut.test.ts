import assert from "node:assert/strict";
import test from "node:test";
import { dragOut } from "./dragOut.ts";

function scenario(dropped: boolean, failure?: "prepare" | "drag" | "finalize" | "cancel") {
  const calls: string[] = [];
  const step = async (name: string) => {
    calls.push(name);
    if (name === failure) throw Error(name);
  };
  return {
    calls,
    effects: {
      setActive: async (active: boolean) => {
        calls.push(String(active));
      },
      prepare: async () => {
        await step("prepare");
        return "single-use-token";
      },
      drag: async (mode: "copy" | "move") => {
        calls.push(mode);
        await step("drag");
        return dropped;
      },
      finalize: async (token: string) => {
        assert.equal(token, "single-use-token");
        await step("finalize");
      },
      cancel: async (token: string) => {
        assert.equal(token, "single-use-token");
        await step("cancel");
      },
      cleanupFailed: () => {
        calls.push("cleanup-failed");
      },
    },
  };
}
test("copy never prepares or consumes a cut token", async () => {
  const s = scenario(true);
  assert.equal(await dragOut("copy", s.effects), "copied");
  assert.deepEqual(s.calls, ["true", "copy", "drag", "false"]);
});
test("move only finalizes after destination acknowledgement", async () => {
  for (const dropped of [false, true]) {
    const s = scenario(dropped);
    assert.equal(await dragOut("move", s.effects), dropped ? "moved" : "cancelled");
    assert.deepEqual(s.calls, [
      "true",
      "prepare",
      "move",
      "drag",
      dropped ? "finalize" : "cancel",
      "false",
    ]);
  }
});
test("destination success is not reported as a failed transfer when source cleanup fails", async () => {
  const s = scenario(true, "finalize");
  assert.equal(await dragOut("move", s.effects), "source-cleanup-failed");
  assert.deepEqual(s.calls.slice(-4), ["finalize", "cleanup-failed", "cancel", "false"]);
});
test("failed preparation or native drag always restores activity and cancels issued tokens", async () => {
  for (const failure of ["prepare", "drag"] as const) {
    const s = scenario(false, failure);
    await assert.rejects(dragOut("move", s.effects), new RegExp(failure));
    assert.equal(s.calls[s.calls.length - 1], "false");
    assert.equal(s.calls.includes("cancel"), failure === "drag");
    assert.equal(s.calls.includes("finalize"), false);
  }
});
test("failed token cancellation still restores panel activity", async () => {
  const s = scenario(false, "cancel");
  assert.equal(await dragOut("move", s.effects), "cancelled");
  assert.deepEqual(s.calls.slice(-3), ["cancel", "cleanup-failed", "false"]);
});
