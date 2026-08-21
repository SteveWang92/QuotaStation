/**
 * The arithmetic behind the dashboard charts, kept away from the drawing so it can be
 * reasoned about and tested on its own.
 *
 * Every chart on the dashboard shares one x-axis shape — a run of consecutive local
 * calendar days — because they are all read against the same date range. The functions
 * here produce that axis, the value scale under it, and the geometry the marks are drawn
 * from; nothing here knows about SVG.
 */

/** Every calendar day from `startDate` to `endDate` inclusive, as `YYYY-MM-DD`. */
export function calendarDays(startDate: string, endDate: string): string[] {
  const start = Date.parse(`${startDate}T00:00:00Z`);
  const end = Date.parse(`${endDate}T00:00:00Z`);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return [];
  const days: string[] = [];
  for (let day = start; day <= end; day += 86_400_000) {
    days.push(new Date(day).toISOString().slice(0, 10));
  }
  return days;
}

/**
 * Lines up sparse records against the full run of days.
 *
 * The core stores only the days a provider was used on, which is what the totals and the
 * active-day average are counted from. A chart has to show the gaps as gaps, so the axis
 * comes from the calendar and a day with no record contributes `undefined`.
 */
export function alignToDays<T extends { date: string }>(
  records: T[],
  days: string[],
): Array<T | undefined> {
  const byDate = new Map(records.map((record) => [record.date, record]));
  return days.map((day) => byDate.get(day));
}

/**
 * A rounded axis maximum and the tick values below it.
 *
 * Ticks land on 1, 2 or 5 times a power of ten so the reader is comparing against numbers
 * worth reading. An empty range still returns a usable axis, because a chart with no data
 * yet is drawn rather than hidden.
 */
export function axisScale(maxValue: number, tickCount = 4): { max: number; ticks: number[] } {
  if (!Number.isFinite(maxValue) || maxValue <= 0) {
    return { max: 1, ticks: [0, 1] };
  }
  const rawStep = maxValue / tickCount;
  const magnitude = 10 ** Math.floor(Math.log10(rawStep));
  const normalized = rawStep / magnitude;
  const step = (normalized <= 1 ? 1 : normalized <= 2 ? 2 : normalized <= 5 ? 5 : 10) * magnitude;
  const max = Math.ceil(maxValue / step) * step;
  const ticks: number[] = [];
  for (let tick = 0; tick <= max + step / 2; tick += step) ticks.push(Number(tick.toFixed(10)));
  return { max, ticks };
}

/**
 * Where a column sits inside the plot.
 *
 * Bars are capped rather than filling their band: the leftover is the air that keeps a
 * dense range readable. The band itself stays the full width so the hit target, which is
 * the band and not the painted bar, never becomes a pinpoint.
 */
export function bandGeometry(width: number, count: number, maxBarWidth = 24) {
  const band = count === 0 ? width : width / count;
  const barWidth = Math.max(1, Math.min(maxBarWidth, band - Math.min(8, band * 0.3)));
  return {
    band,
    barWidth,
    center: (index: number) => band * (index + 0.5),
    left: (index: number) => band * (index + 0.5) - barWidth / 2,
  };
}

/**
 * A rectangle whose top corners are rounded and whose base is square, which is the shape
 * every column ends in: the data-end is rounded, the baseline is not. The radius shrinks
 * on a short bar so a two-pixel value never renders as a lozenge.
 */
export function columnPath(
  x: number,
  y: number,
  width: number,
  height: number,
  radius = 4,
): string {
  const r = Math.max(0, Math.min(radius, width / 2, height));
  const bottom = y + height;
  return [
    `M${x} ${bottom}`,
    `L${x} ${y + r}`,
    r > 0 ? `Q${x} ${y} ${x + r} ${y}` : "",
    `L${x + width - r} ${y}`,
    r > 0 ? `Q${x + width} ${y} ${x + width} ${y + r}` : "",
    `L${x + width} ${bottom}`,
    "Z",
  ]
    .filter(Boolean)
    .join(" ");
}

/**
 * Stacks one day's series values into drawable segments, bottom-up, with a surface gap
 * between neighbours.
 *
 * The gap is taken out of the segment below the join rather than drawn over it, so nothing
 * is painted twice and a segment too small to survive the gap simply is not drawn.
 */
export function stackSegments(
  values: number[],
  scale: (value: number) => number,
  gap = 2,
): Array<{ index: number; top: number; height: number } | null> {
  let runningTotal = 0;
  const drawn: Array<{ index: number; top: number; height: number } | null> = [];
  const positive = values.map((value) => (Number.isFinite(value) && value > 0 ? value : 0));
  const lastDrawnIndex = positive.reduce((last, value, index) => (value > 0 ? index : last), -1);
  positive.forEach((value, index) => {
    if (value <= 0) {
      drawn.push(null);
      return;
    }
    const base = scale(runningTotal);
    runningTotal += value;
    const top = scale(runningTotal);
    const height = base - top - (index === lastDrawnIndex ? 0 : gap);
    drawn.push(height > 0.5 ? { index, top, height } : null);
  });
  return drawn;
}

/**
 * The polyline through a series, skipping the days it has no reading for.
 *
 * A missing day is a gap in the line rather than a zero: a quota window nobody observed
 * that day did not sit at zero, and joining across it would draw a slope that never
 * happened.
 */
export function linePath(points: Array<{ x: number; y: number } | null>): string {
  let path = "";
  let penDown = false;
  for (const point of points) {
    if (point === null) {
      penDown = false;
      continue;
    }
    path += `${penDown ? "L" : "M"}${point.x.toFixed(2)} ${point.y.toFixed(2)} `;
    penDown = true;
  }
  return path.trim();
}

/**
 * How many x labels a plot can carry without them colliding: every nth day, chosen so the
 * labels keep a comfortable distance whatever the range length.
 */
export function labelStride(count: number, width: number, minSpacing = 74): number {
  if (count <= 1 || width <= 0) return 1;
  return Math.max(1, Math.ceil(count / Math.max(1, Math.floor(width / minSpacing))));
}
