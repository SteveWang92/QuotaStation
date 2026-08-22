import { describe, expect, it } from "vitest";
import { quotaColor, statusColor } from "../src/theme";
import type { CompactStatus, LimitWindow } from "../src/types";

function window(statusLevel: LimitWindow["statusLevel"]): LimitWindow {
  return {
    kind: "primary",
    label: "5-hour window",
    statusLevel,
    usedPercent: 50,
    windowDurationMins: 300,
    resetsAt: null,
    source: "status_line",
    observedAt: 0,
    freshness: "fresh",
  };
}

const status = (level: CompactStatus["level"]): CompactStatus => ({ level, label: level });

describe("theme", () => {
  it("draws a window in the token for its own level rather than a fixed colour", () => {
    expect(quotaColor(window("healthy"))).toBe("var(--status-healthy)");
    expect(quotaColor(window("warning"))).toBe("var(--status-warning)");
    expect(quotaColor(window("critical"))).toBe("var(--status-critical)");
  });

  it("reads stale data as a warning and an unreadable provider as critical", () => {
    expect(statusColor(status("healthy"))).toBe("var(--status-healthy)");
    expect(statusColor(status("warning"))).toBe("var(--status-warning)");
    expect(statusColor(status("stale"))).toBe("var(--status-warning)");
    expect(statusColor(status("critical"))).toBe("var(--status-critical)");
    expect(statusColor(status("unavailable"))).toBe("var(--status-critical)");
  });
});
