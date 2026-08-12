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
  formatWindowDuration,
} from "../src/format";

// Assertions stay locale independent: the surfaces format with whatever locale the
// machine reports, so only the values QuotaStation itself decides are pinned here.

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

  it("clamps a window that has already reset", () => {
    freezeClock();
    expect(formatCountdown(inSeconds(-90))).toBe("0h 0m");
    expect(formatCompactCountdown(inSeconds(-90))).toBe("0h 0m");
  });

  it("marks an unknown reset time on both surfaces", () => {
    expect(formatCountdown(null)).toBe("Unknown");
    expect(formatCompactCountdown(null)).toBe("—");
  });
});

describe("quota window durations", () => {
  it("describes a duration in its largest whole unit", () => {
    expect(formatWindowDuration(300)).toBe("5-hour quota window");
    expect(formatWindowDuration(10_080)).toBe("7-day quota window");
    expect(formatWindowDuration(90)).toBe("90-minute quota window");
  });

  it("badges the same duration for the taskbar", () => {
    expect(formatWindowBadge(300, "Primary window")).toBe("5H");
    expect(formatWindowBadge(10_080, "Weekly window")).toBe("7D");
  });

  it("falls back to the window label when the provider reports no duration", () => {
    expect(formatWindowDuration(null)).toBe("Window duration unavailable");
    expect(formatWindowBadge(null, "Weekly window")).toBe("WE");
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
