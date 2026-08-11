import { describe, expect, it } from "vitest";
import { createCustomRange, createPresetRange, todayString } from "../src/dateRanges";

// Ranges are built from the machine's own calendar, so the assertions compare calendar
// days rather than fixed dates.
function calendarDaysBetween(startDate: string, endDate: string): number {
  const start = Date.parse(`${startDate}T00:00:00Z`);
  const end = Date.parse(`${endDate}T00:00:00Z`);
  return Math.round((end - start) / 86_400_000);
}

describe("preset ranges", () => {
  it("treats today as a single inclusive day", () => {
    const range = createPresetRange("today");
    expect(range.startDate).toBe(todayString());
    expect(range.endDate).toBe(todayString());
    expect(range.label).toBe("Today");
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

describe("custom ranges", () => {
  it("keeps the requested boundaries and labels both ends", () => {
    const range = createCustomRange("2026-07-01", "2026-07-31");
    expect(range.preset).toBe("custom");
    expect(range.startDate).toBe("2026-07-01");
    expect(range.endDate).toBe("2026-07-31");
    expect(range.label).toContain("–");
  });
});
