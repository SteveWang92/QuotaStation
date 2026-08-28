import { describe, expect, it } from "vitest";
import { groupChartMarkers } from "../src/components/TrendChart";

describe("chart markers", () => {
  it("keeps every restart that lands in the same bucket", () => {
    const grouped = groupChartMarkers([
      { id: "primary-1", bucket: "2026-08-29", label: "Five-hour restart", tone: "muted" },
      {
        id: "secondary-1",
        bucket: "2026-08-29",
        label: "Weekly early restart",
        tone: "warning",
      },
    ]);

    expect(grouped.get("2026-08-29")?.map((marker) => marker.label)).toEqual([
      "Five-hour restart",
      "Weekly early restart",
    ]);
  });
});
