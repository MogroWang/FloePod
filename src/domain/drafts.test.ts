import assert from "node:assert/strict";
import test from "node:test";
import { createDrafts } from "./drafts.ts";

test("acknowledging a saved value preserves newer edits including an A-B-A sequence", () => {
  const drafts = createDrafts<string, number>();
  const first = drafts.preview("1:opacity", 0.7);
  drafts.preview("1:opacity", 0.8);
  const last = drafts.preview("1:opacity", 0.7);
  drafts.accept("1:opacity", first);
  assert.equal(drafts.get("1:opacity"), 0.7);
  drafts.accept("1:opacity", last);
  assert.equal(drafts.get("1:opacity"), undefined);
});

test("a failed save and edits in other pods do not erase a draft", () => {
  const drafts = createDrafts<string, string>();
  drafts.preview("1:barColor", "#123456");
  const other = drafts.preview("2:barColor", "#abcdef");
  drafts.accept("2:barColor", other);
  assert.equal(drafts.get("1:barColor"), "#123456");
});
