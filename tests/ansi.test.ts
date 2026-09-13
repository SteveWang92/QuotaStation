import { describe, expect, it } from "vitest";
import { ansiRuns } from "../src/ansi";

const ESC = String.fromCharCode(27);

describe("status line preview colours", () => {
  it("keeps each reading's threshold colour and drops the escape codes", () => {
    expect(ansiRuns(`CDX 5h ${ESC}[33m72%${ESC}[0m · 7d ${ESC}[31m91%${ESC}[0m`)).toEqual([
      { text: "CDX 5h ", level: null },
      { text: "72%", level: "warning" },
      { text: " · 7d ", level: null },
      { text: "91%", level: "critical" },
    ]);
  });
});
