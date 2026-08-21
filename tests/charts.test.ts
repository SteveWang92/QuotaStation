import { describe, expect, it } from "vitest";
import {
  alignToDays,
  axisScale,
  bandGeometry,
  calendarDays,
  columnPath,
  labelStride,
  linePath,
  stackSegments,
} from "../src/charts";

describe("the day axis", () => {
  it("covers both ends of the range", () => {
    expect(calendarDays("2026-08-14", "2026-08-17")).toEqual([
      "2026-08-14",
      "2026-08-15",
      "2026-08-16",
      "2026-08-17",
    ]);
  });

  it("crosses a month boundary", () => {
    expect(calendarDays("2026-07-31", "2026-08-01")).toEqual(["2026-07-31", "2026-08-01"]);
  });

  it("has no days at all when the range is backwards", () => {
    expect(calendarDays("2026-08-17", "2026-08-14")).toEqual([]);
  });

  it("leaves a day with no record empty rather than zero", () => {
    const days = calendarDays("2026-08-14", "2026-08-16");
    const aligned = alignToDays([{ date: "2026-08-15", tokens: 12 }], days);
    expect(aligned).toEqual([undefined, { date: "2026-08-15", tokens: 12 }, undefined]);
  });
});

describe("the value axis", () => {
  it("rounds up to a step a reader can count in", () => {
    expect(axisScale(3_700)).toEqual({ max: 4_000, ticks: [0, 1_000, 2_000, 3_000, 4_000] });
  });

  it("still produces an axis when there is nothing to plot", () => {
    expect(axisScale(0).max).toBe(1);
  });

  it("never cuts the largest value off", () => {
    for (const value of [1, 7, 93, 1_234, 987_654, 5_000_000]) {
      expect(axisScale(value).max).toBeGreaterThanOrEqual(value);
    }
  });
});

describe("column geometry", () => {
  it("caps a bar rather than filling its band", () => {
    const wide = bandGeometry(1_000, 3);
    expect(wide.band).toBeCloseTo(333.33, 1);
    expect(wide.barWidth).toBe(24);
  });

  it("leaves the hit target the full band even when the bar is a sliver", () => {
    const dense = bandGeometry(200, 200);
    expect(dense.band).toBe(1);
    expect(dense.barWidth).toBeLessThanOrEqual(1);
  });

  it("rounds the data-end and squares the baseline", () => {
    const path = columnPath(0, 10, 20, 40, 4);
    expect(path.startsWith("M0 50")).toBe(true);
    expect(path).toContain("Q");
    expect(path.endsWith("Z")).toBe(true);
  });

  it("shrinks the corner rather than rounding a sliver into a lozenge", () => {
    // The corner may never take more than the bar's own height, or the top of a two-pixel
    // bar curves further than the bar is tall.
    expect(columnPath(0, 10, 20, 0.5, 4)).toContain("Q0 10 0.5 10");
    expect(columnPath(0, 10, 3, 40, 4)).toContain("Q0 10 1.5 10");
  });
});

describe("stacking", () => {
  const scale = (value: number) => 100 - value;

  it("separates neighbours with a surface gap and leaves the top segment whole", () => {
    const segments = stackSegments([40, 40], scale, 2);
    expect(segments[0]).toEqual({ index: 0, top: 60, height: 38 });
    expect(segments[1]).toEqual({ index: 1, top: 20, height: 40 });
  });

  it("draws nothing for a category with no tokens", () => {
    expect(stackSegments([0, 30], scale, 2)[0]).toBeNull();
  });

  it("drops a segment the gap would leave invisible", () => {
    expect(stackSegments([0.4, 30], scale, 2)[0]).toBeNull();
  });
});

describe("lines", () => {
  it("breaks rather than joining across a day with no reading", () => {
    const path = linePath([{ x: 0, y: 10 }, null, { x: 20, y: 30 }]);
    expect(path).toBe("M0.00 10.00 M20.00 30.00");
  });

  it("thins the axis labels until they stop colliding", () => {
    expect(labelStride(7, 700)).toBe(1);
    expect(labelStride(90, 700)).toBeGreaterThan(1);
    expect(labelStride(1, 700)).toBe(1);
  });
});
