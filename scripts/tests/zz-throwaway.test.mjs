import assert from "node:assert/strict";
import { it } from "node:test";
it("deliberate failure to prove ci-feedback comments (throwaway)", () => {
  assert.equal(1, 2, "deliberate failure");
});
