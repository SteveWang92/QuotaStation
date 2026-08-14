import { LOCALE } from "./format";

export type RangePreset = "today" | "3d" | "7d" | "30d" | "custom";

export interface DateRangeSelection {
  preset: RangePreset;
  label: string;
  startDate: string;
  endDate: string;
}

function toLocalDateString(date: Date): string {
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

export function formatRangeDate(value: string): string {
  return new Intl.DateTimeFormat(LOCALE, { day: "numeric", month: "short", year: "numeric" })
    .format(new Date(`${value}T00:00:00`));
}
