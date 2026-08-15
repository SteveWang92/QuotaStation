import { describe, expect, it } from "vitest";
import { emptySnapshot, resolveProviderKey } from "../src/workspace";

describe("resolveProviderKey", () => {
  const codex = emptySnapshot("codex", "Codex", "CDX");
  const claude = emptySnapshot("claude", "Claude Code", "CLD");

  it("keeps a preferred provider that is still detected", () => {
    expect(resolveProviderKey([codex, claude], "claude")).toBe("claude");
  });

  it("falls back to the detected provider on a Claude-only machine", () => {
    expect(resolveProviderKey([claude], "codex")).toBe("claude");
  });

  it("returns no provider for an empty workspace", () => {
    expect(resolveProviderKey([], "codex")).toBeUndefined();
  });
});
