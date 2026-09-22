import { formatAxisHour, hourDate, LOCALE } from "./format";
import { addDays, dateOf, HOUR_MS, hourOf, instantOf, today } from "./zone";

export type RangePreset = "24h" | "today" | "3d" | "7d" | "30d" | "all" | "custom";

/** How many hours the rolling window covers, counting the hour in progress as one. */
export const WINDOW_HOURS = 24;

/** A daily SVG stays bounded while still allowing a full year to be inspected at once. */
export const MAX_CUSTOM_RANGE_DAYS = 366;

/**
 * Up to this many days, the charts are drawn hour by hour.
 *
 * Three columns describe three days without saying anything about them: the work that
 * filled an afternoon and the work spread evenly across a day draw the same bar. Past
 * three days the hours outnumber the pixels, and the daily shape is the readable one.
 */
export const MAX_HOURLY_RANGE_DAYS = 3;

export interface DateRangeSelection {
  preset: RangePreset;
  label: string;
  startDate: string;
  endDate: string;
  /**
   * The inclusive hour bounds of a rolling window, as `YYYY-MM-DDTHH:00`.
   *
   * Only the 24-hour preset carries them, and carrying them is what makes it a different
   * question from "today": the calendar presets are answered from whole stored days, and
   * both of the days a rolling window touches are partial.
   */
  startHour?: string;
  endHour?: string;
}

/** A calendar day in the application zone, which is how every stored date is dated. */
export function toLocalDateString(date: Date): string {
  return dateOf(date.getTime());
}

export function todayString(): string {
  return today();
}

/** The hour a moment falls in, in the application zone, which is how every stored hourly
 * bucket is keyed. */
export function toLocalHourString(date: Date): string {
  return hourOf(date.getTime());
}

/** A window of [`WINDOW_HOURS`] hours ending with the hour that opened at `endHourStart`. */
function windowEndingAt(endHourStart: number) {
  const start = endHourStart - (WINDOW_HOURS - 1) * HOUR_MS;
  return {
    startDate: dateOf(start),
    endDate: dateOf(endHourStart),
    startHour: hourOf(start),
    endHour: hourOf(endHourStart),
  };
}

/** The last [`WINDOW_HOURS`] hours, ending with the hour in progress. */
function createWindowRange(): DateRangeSelection {
  return {
    preset: "24h",
    label: `Last ${WINDOW_HOURS} hours`,
    ...windowEndingAt(instantOf(hourOf(Date.now()))),
  };
}

export function createPresetRange(preset: Exclude<RangePreset, "custom">): DateRangeSelection {
  if (preset === "24h") return createWindowRange();
  if (preset === "all") return createAllRange(todayString());
  const days = preset === "today" ? 1 : Number.parseInt(preset, 10);
  const end = today();
  return {
    preset,
    label: preset === "today" ? "Today" : `Last ${days} days`,
    startDate: addDays(end, -(days - 1)),
    endDate: end,
  };
}

/** Every recorded usage day through today; the core supplies the first stored date. */
export function createAllRange(startDate: string): DateRangeSelection {
  return {
    preset: "all",
    label: "All time",
    startDate,
    endDate: todayString(),
  };
}

export function createCustomRange(startDate: string, endDate: string): DateRangeSelection {
  return {
    preset: "custom",
    label: `${formatRangeDate(startDate)} – ${formatRangeDate(endDate)}`,
    startDate,
    endDate,
  };
}

/** Whether an inclusive custom range is too large to render one mark per calendar day. */
export function customRangeTooLong(startDate: string, endDate: string): boolean {
  return rangeLengthInDays(startDate, endDate) > MAX_CUSTOM_RANGE_DAYS;
}

/** How many calendar days an inclusive range covers. */
export function rangeLengthInDays(startDate: string, endDate: string): number {
  const start = Date.parse(`${startDate}T00:00:00Z`);
  const end = Date.parse(`${endDate}T00:00:00Z`);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return 0;
  return Math.floor((end - start) / 86_400_000) + 1;
}

/** Whether a range is short enough to be read hour by hour. */
export function isHourlyRange(startDate: string, endDate: string): boolean {
  const length = rangeLengthInDays(startDate, endDate);
  return length > 0 && length <= MAX_HOURLY_RANGE_DAYS;
}

/** Recomputes calendar presets at query time so a tray process can cross midnight safely. */
export function resolveDateRange(selection: DateRangeSelection): DateRangeSelection {
  if (selection.preset === "custom") return selection;
  if (selection.preset === "all") return createAllRange(selection.startDate);
  return createPresetRange(selection.preset);
}

/**
 * Whether a preset now covers different days than the ones already on display. A window
 * that stays open overnight is otherwise still showing yesterday under today's heading,
 * because nothing about the data changed — only the calendar did.
 */
export function hasRolledOver(selection: DateRangeSelection): boolean {
  const resolved = resolveDateRange(selection);
  return (
    resolved.startDate !== selection.startDate ||
    resolved.endDate !== selection.endDate ||
    // The rolling window moves every hour rather than every midnight.
    resolved.startHour !== selection.startHour ||
    resolved.endHour !== selection.endHour
  );
}

/** A calendar date is a label rather than an instant, so it is written out in UTC, where
 * the label and the instant agree. */
export function formatRangeDate(value: string): string {
  return new Intl.DateTimeFormat(LOCALE, {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  }).format(new Date(`${value}T00:00:00Z`));
}

/** An hour bucket named in full, for example 3 Aug 2026, 14:00. */
export function formatRangeHour(value: string): string {
  return `${formatRangeDate(hourDate(value))}, ${formatAxisHour(value)}`;
}

/**
 * The period of the same length immediately before `selection`, which is what every figure
 * on the dashboard is compared against. A range is inclusive of both ends, so the previous
 * period ends the day before this one starts.
 */
export function previousPeriod(selection: DateRangeSelection): {
  startDate: string;
  endDate: string;
  startHour?: string;
  endHour?: string;
} {
  if (selection.startHour !== undefined) {
    return windowEndingAt(instantOf(selection.startHour) - HOUR_MS);
  }
  const length = rangeLengthInDays(selection.startDate, selection.endDate);
  const previousEnd = addDays(selection.startDate, -1);
  return { startDate: addDays(previousEnd, -(length - 1)), endDate: previousEnd };
}
