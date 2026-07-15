import { describe, expect, it } from "vitest";

import { SessionAnchors } from "../src/session-anchors.js";

describe("SessionAnchors", () => {
  it("forgets one path and clears all paths", () => {
    const anchors = new SessionAnchors();
    anchors.markAnchored("/one.ts");
    anchors.markAnchored("/two.ts");

    anchors.forget("/one.ts");
    expect(anchors.hasFreshAnchors("/one.ts")).toBe(false);
    expect(anchors.hasFreshAnchors("/two.ts")).toBe(true);

    anchors.clear();
    expect(anchors.hasFreshAnchors("/two.ts")).toBe(false);
  });
});
