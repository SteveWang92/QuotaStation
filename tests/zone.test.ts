import { afterEach, describe, expect, it, vi } from "vitest";
import { calendarHours } from "../src/charts";
import { createPresetRange } from "../src/dateRanges";
import {
  formatClockOffset,
  formatCountdown,
  formatResetTimestamp,
  formatShortMoment,
} from "../src/format";
import { dateOf, instantOf, setClockOffset, setZone } from "../src/zone";

afterEach(() => {
  vi.useRealTimers();
});

describe("the application zone", () => {
  it("writes a reset time in the chosen zone rather than the machine's", () => {
    setZone("Pacific/Auckland");
    // 00:30 on 2 June in Auckland is still 1 June in UTC.
    const instant = Date.UTC(2026, 5, 1, 12, 30) / 1_000;
    expect(formatResetTimestamp(instant)).toMatch(/^2 June? 2026/);
    expect(formatResetTimestamp(instant)).toContain("00:30");
    setZone("Europe/London");
    expect(formatResetTimestamp(instant)).toMatch(/^1 June? 2026/);
    expect(formatResetTimestamp(instant)).toContain("13:30");
  });

  it("dates a moment by the chosen zone's day", () => {
    setZone("Pacific/Auckland");
    vi.useFakeTimers();
    vi.setSystemTime(Date.UTC(2026, 5, 1, 12, 30));
    expect(dateOf(Date.now())).toBe("2026-06-02");
    // Earlier the same Auckland day carries no date.
    expect(formatShortMoment(Date.UTC(2026, 5, 1, 12, 5) / 1_000)).toBe("00:05");
  });

  it("builds the rolling window from the chosen zone's hours", () => {
    setZone("Pacific/Auckland");
    vi.useFakeTimers();
    vi.setSystemTime(Date.UTC(2026, 5, 1, 12, 30));
    const range = createPresetRange("24h");
    expect(range.endHour).toBe("2026-06-02T00:00");
    expect(range.startHour).toBe("2026-06-01T01:00");
  });

  it("gives a day the clocks go back on its repeated hour once", () => {
    setZone("Australia/Sydney");
    // Sydney leaves daylight saving at 03:00 on 5 April 2026, repeating 02:00.
    const hours = calendarHours("2026-04-05", "2026-04-05", Date.UTC(2026, 3, 6));
    expect(hours).toHaveLength(24);
    expect(new Set(hours).size).toBe(24);
  });

  it("gives a day the clocks go forward on only the hours it had", () => {
    setZone("Australia/Sydney");
    // Sydney enters daylight saving at 02:00 on 4 October 2026, skipping 02:00.
    const hours = calendarHours("2026-10-04", "2026-10-04", Date.UTC(2026, 9, 5));
    expect(hours).toHaveLength(23);
    expect(hours).not.toContain("2026-10-04T02:00");
  });

  it("finds the instant a wall-clock hour began at on either side of a change", () => {
    setZone("Australia/Sydney");
    expect(instantOf("2026-04-05T02:00")).toBe(Date.UTC(2026, 3, 4, 15));
    expect(instantOf("2026-04-05T04:00")).toBe(Date.UTC(2026, 3, 4, 18));
  });
});

describe("the corrected clock", () => {
  it("counts a window down against internet time rather than a fast local clock", () => {
    vi.useFakeTimers();
    // This clock reads 12:10 but the true time is 12:00, and the window resets at 12:05.
    vi.setSystemTime(Date.UTC(2026, 5, 1, 12, 10));
    const resetsAt = Date.UTC(2026, 5, 1, 12, 5) / 1_000;
    expect(formatCountdown(resetsAt)).toBe("Expired");
    setClockOffset(-600_000);
    expect(formatCountdown(resetsAt)).toBe("0h 5m");
    setClockOffset(0);
  });

  it("warns about a clock more than two minutes off, and only then", () => {
    expect(formatClockOffset(-420_000)).toBe("This computer's clock is 7 min fast");
    expect(formatClockOffset(180_000)).toBe("This computer's clock is 3 min slow");
    expect(formatClockOffset(-60_000)).toBeNull();
  });
});
