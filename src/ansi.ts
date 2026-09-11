/** The threshold colours the status line prints, by the level each one stands for. */
export type AnsiLevel = "healthy" | "warning" | "critical";

export interface AnsiRun {
  text: string;
  level: AnsiLevel | null;
}

const LEVELS: Record<string, AnsiLevel> = { "32": "healthy", "33": "warning", "31": "critical" };

const SGR = new RegExp(`${String.fromCharCode(27)}\\[(\\d*)m`);

/**
 * The status line as the core printed it, split into runs of text and the level each run
 * was coloured for. The core is the only renderer of the line; this only turns its colour
 * codes into something the settings page can theme.
 */
export function ansiRuns(line: string): AnsiRun[] {
  const runs: AnsiRun[] = [];
  let level: AnsiLevel | null = null;
  // Splitting on a pattern with one group alternates text and the code between texts.
  line.split(SGR).forEach((part, index) => {
    if (index % 2 === 1) {
      level = LEVELS[part] ?? null;
    } else if (part) {
      runs.push({ text: part, level });
    }
  });
  return runs;
}
