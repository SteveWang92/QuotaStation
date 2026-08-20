import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createCustomRange,
  createPresetRange,
  customRangeTooLong,
  hasRolledOver,
  resolveDateRange,
  todayString,
} from "../src/dateRanges";

// Ranges are built from the machine's own calendar, so the assertions compare calendar
// days rather than fixed dates.
function calendarDaysBetween(startDate: string, endDate: string): number {
  const start = Date.parse(`${startDate}T00:00:00Z`);
  const end = Date.parse(`${endDate}T00:00:00Z`);
  return Math.round((end - start) / 86_400_000);
}

afterEach(() => {
  vi.useRealTimers();
});

describe("preset ranges", () => {
  it("treats today as a single inclusive day", () => {
    const range = createPresetRange("today");
    expect(range.startDate).toBe(todayString());
    expect(range.endDate).toBe(todayString());
    expect(range.label).toBe("Today");
  });

  it("recomputes a preset after the process crosses midnight", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 7, 11, 23, 59));
    const range = createPresetRange("today");
    vi.setSystemTime(new Date(2026, 7, 12, 0, 1));
    expect(resolveDateRange(range).startDate).toBe("2026-08-12");
  });

  it("counts the current day as part of a multi-day preset", () => {
    for (const [preset, days] of [["3d", 3], ["7d", 7], ["30d", 30]] as const) {
      const range = createPresetRange(preset);
      expect(range.endDate).toBe(todayString());
      expect(calendarDaysBetween(range.startDate, range.endDate)).toBe(days - 1);
      expect(range.label).toBe(`Last ${days} days`);
    }
  });
});

describe("rollover detection", () => {
  it("reports nothing to redraw while the day holds", () => {
    expect(hasRolledOver(createPresetRange("today"))).toBe(false);
    expect(hasRolledOver(createPresetRange("7d"))).toBe(false);
  });

  it("reports a preset stale once the calendar has moved past it", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 7, 11, 23, 59));
    const today = createPresetRange("today");
    const week = createPresetRange("7d");
    vi.setSystemTime(new Date(2026, 7, 12, 0, 1));
    expect(hasRolledOver(today)).toBe(true);
    expect(hasRolledOver(week)).toBe(true);
  });

  it("leaves a custom range alone, since its boundaries were chosen", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 7, 11, 23, 59));
    const range = createCustomRange("2026-07-01", "2026-07-31");
    vi.setSystemTime(new Date(2026, 7, 12, 0, 1));
    expect(hasRolledOver(range)).toBe(false);
  });
});

describe("custom ranges", () => {
  it("keeps the requested boundaries and labels both ends", () => {
    const range = createCustomRange("2026-07-01", "2026-07-31");
    expect(range.preset).toBe("custom");
    expect(range.startDate).toBe("2026-07-01");
    expect(range.endDate).toBe("2026-07-31");
    expect(range.label).toContain("–");
  });

  it("bounds the number of daily chart marks to one year", () => {
    expect(customRangeTooLong("2025-01-01", "2026-01-01")).toBe(false);
    expect(customRangeTooLong("2025-01-01", "2026-01-02")).toBe(true);
  });
});
