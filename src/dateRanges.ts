import { LOCALE } from "./format";

export type RangePreset = "today" | "3d" | "7d" | "30d" | "custom";

export interface DateRangeSelection {
  preset: RangePreset;
  label: string;
  startDate: string;
  endDate: string;
}

/** A calendar day in the machine's own time zone, which is how every stored date is dated. */
export function toLocalDateString(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function todayString(): string {
  return toLocalDateString(new Date());
}

export function createPresetRange(preset: Exclude<RangePreset, "custom">): DateRangeSelection {
  const days = preset === "today" ? 1 : Number.parseInt(preset, 10);
  const end = new Date();
  const start = new Date(end);
  start.setDate(end.getDate() - days + 1);
  return {
    preset,
    label: preset === "today" ? "Today" : `Last ${days} days`,
    startDate: toLocalDateString(start),
    endDate: toLocalDateString(end),
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

/** Recomputes calendar presets at query time so a tray process can cross midnight safely. */
export function resolveDateRange(selection: DateRangeSelection): DateRangeSelection {
  return selection.preset === "custom" ? selection : createPresetRange(selection.preset);
}

/**
 * Whether a preset now covers different days than the ones already on display. A window
 * that stays open overnight is otherwise still showing yesterday under today's heading,
 * because nothing about the data changed — only the calendar did.
 */
export function hasRolledOver(selection: DateRangeSelection): boolean {
  const resolved = resolveDateRange(selection);
  return resolved.startDate !== selection.startDate || resolved.endDate !== selection.endDate;
}

export function formatRangeDate(value: string): string {
  return new Intl.DateTimeFormat(LOCALE, { day: "numeric", month: "short", year: "numeric" })
    .format(new Date(`${value}T00:00:00`));
}

/**
 * The period of the same length immediately before `selection`, which is what every figure
 * on the dashboard is compared against. A range is inclusive of both ends, so the previous
 * period ends the day before this one starts.
 */
export function previousPeriod(selection: DateRangeSelection): { startDate: string; endDate: string } {
  const start = new Date(`${selection.startDate}T00:00:00`);
  const end = new Date(`${selection.endDate}T00:00:00`);
  const length = Math.round((end.getTime() - start.getTime()) / 86_400_000) + 1;
  const previousEnd = new Date(start);
  previousEnd.setDate(start.getDate() - 1);
  const previousStart = new Date(previousEnd);
  previousStart.setDate(previousEnd.getDate() - length + 1);
  return { startDate: toLocalDateString(previousStart), endDate: toLocalDateString(previousEnd) };
}
