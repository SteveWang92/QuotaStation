import { afterEach, describe, expect, it, vi } from "vitest";
import {
  formatCompactCountdown,
  formatCountdown,
  formatCurrency,
  formatEarlyBy,
  formatResetTimestamp,
  formatRevision,
  formatTimestamp,
  formatWindowBadge,
} from "../src/format";

// Assertions stay locale independent: the surfaces format with whatever locale the
// machine reports, so only the values QuotaStation itself decides are pinned here. The
// clock is one of those decisions — every timestamp is 24-hour — but the machine's time
// zone is not, so it is asserted through the absence of a meridiem rather than an hour.

const NOW = Date.UTC(2026, 7, 11, 12, 0, 0);

function inSeconds(minutes: number): number {
  return (NOW + minutes * 60_000) / 1_000;
}

afterEach(() => {
  vi.useRealTimers();
});

function freezeClock() {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
}

describe("countdowns", () => {
  it("truncates towards the elapsed minute so surfaces never disagree", () => {
    freezeClock();
    const resetsAt = inSeconds(125) + 59 / 60;
    expect(formatCountdown(resetsAt)).toBe("2h 5m");
    expect(formatCompactCountdown(resetsAt)).toBe("2h 5m");
  });

  it("drops minutes only once a countdown spans days", () => {
    freezeClock();
    const resetsAt = inSeconds(2 * 1_440 + 185);
    expect(formatCountdown(resetsAt)).toBe("2d 3h 5m");
    expect(formatCompactCountdown(resetsAt)).toBe("2d 3h");
  });

  it("marks a window that has already reset as expired", () => {
    freezeClock();
    expect(formatCountdown(inSeconds(-90))).toBe("Expired");
    expect(formatCompactCountdown(inSeconds(-90))).toBe("Expired");
  });

  it("marks an unknown reset time on both surfaces", () => {
    expect(formatCountdown(null)).toBe("Unknown");
    expect(formatCompactCountdown(null)).toBe("—");
  });
});

describe("quota window durations", () => {
  it("badges a duration in its largest whole unit for the taskbar", () => {
    expect(formatWindowBadge(300, "Primary window")).toBe("5H");
    expect(formatWindowBadge(10_080, "Weekly window")).toBe("7D");
  });

  it("falls back to the window label when the provider reports no duration", () => {
    expect(formatWindowBadge(null, "Weekly window")).toBe("WE");
  });
});

describe("clock times", () => {
  it("writes every timestamp on a 24-hour clock", () => {
    // 22:00 UTC and 10:00 UTC: whatever the machine's offset, one of the two lands in the
    // afternoon, so a 12-hour formatter would have to mark it.
    for (const utcHour of [22, 10]) {
      const instant = Date.UTC(2026, 7, 11, utcHour, 0, 0);
      expect(formatTimestamp(new Date(instant).toISOString())).not.toMatch(/[ap]\.?m\.?/i);
      expect(formatResetTimestamp(instant / 1_000)).not.toMatch(/[ap]\.?m\.?/i);
    }
  });
});

describe("how early a window restarted", () => {
  it("keeps a fraction of a day so a reset days early stays legible", () => {
    expect(formatEarlyBy(Math.round(4.8 * 86_400))).toBe("4.8 days early");
    expect(formatEarlyBy(Math.round(6.36 * 86_400))).toBe("6.4 days early");
    expect(formatEarlyBy(Math.round(12.4 * 86_400))).toBe("12 days early");
  });

  it("falls back to hours below a day, where the polling interval dominates", () => {
    expect(formatEarlyBy(3 * 3_600)).toBe("3 hours early");
    expect(formatEarlyBy(7_500)).toBe("2 hours early");
    expect(formatEarlyBy(2_000)).toBe("under an hour early");
  });
});

describe("provenance and missing values", () => {
  it("shortens a revision and names an absent one", () => {
    expect(formatRevision("033c1f7631f603fc939fdc85163e8203f0084f83")).toBe("033c1f7631f6");
    expect(formatRevision("")).toBe("unavailable");
  });

  it("reports missing amounts and timestamps instead of rendering zero", () => {
    expect(formatCurrency(null)).toBe("Unavailable");
    expect(formatTimestamp(null)).toBe("Never");
    expect(formatResetTimestamp(null)).toBe("Reset time unknown");
  });
});
