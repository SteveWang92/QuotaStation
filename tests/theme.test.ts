import { describe, expect, it } from "vitest";
import { statusColor } from "../src/theme";
import type { CompactStatus } from "../src/types";

const status = (level: CompactStatus["level"]): CompactStatus => ({ level, label: level });

describe("theme", () => {
  it("reads stale data as a warning and an unreadable provider as critical", () => {
    expect(statusColor(status("healthy"))).toBe("var(--status-healthy)");
    expect(statusColor(status("warning"))).toBe("var(--status-warning)");
    expect(statusColor(status("stale"))).toBe("var(--status-warning)");
    expect(statusColor(status("critical"))).toBe("var(--status-critical)");
    expect(statusColor(status("unavailable"))).toBe("var(--status-critical)");
  });
});
