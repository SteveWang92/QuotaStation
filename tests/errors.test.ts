import { describe, expect, it } from "vitest";
import { errorMessage } from "../src/errors";

describe("command failures", () => {
  it("keeps the message the core sent", () => {
    // Tauri rejects commands with a plain string.
    expect(errorMessage("state not managed for field `state`")).toBe(
      "state not managed for field `state`",
    );
    expect(errorMessage(new Error("Local storage write failed"))).toBe(
      "Local storage write failed",
    );
  });
});
