/**
 * The interface is English-only, so every surface formats numbers and dates the same way
 * instead of following whatever locale the machine reports. This single constant is the
 * place to revisit once the interface offers a language choice of its own.
 *
 * Clock times are always 24-hour. Quota windows restart at arbitrary times of day and the
 * surfaces sit beside countdowns, so an am/pm marker is one more thing to read before two
 * timestamps can be compared.
 */
export const LOCALE = "en-AU";

export function formatNumber(value: number): string {
  return new Intl.NumberFormat(LOCALE).format(value);
}

export function formatCurrency(value: number | null): string {
  if (value === null) return "Unavailable";
  return new Intl.NumberFormat(LOCALE, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

export function formatTimestamp(value: string | null): string {
  if (!value) return "Never";
  return new Intl.DateTimeFormat(LOCALE, {
    dateStyle: "medium",
    timeStyle: "medium",
    hour12: false,
  }).format(new Date(value));
}

function countdownParts(epochSeconds: number) {
  const totalMinutes = Math.floor(Math.max(0, epochSeconds * 1_000 - Date.now()) / 60_000);
  return {
    days: Math.floor(totalMinutes / 1_440),
    hours: Math.floor((totalMinutes % 1_440) / 60),
    minutes: totalMinutes % 60,
  };
}

export function formatCountdown(epochSeconds: number | null): string {
  if (epochSeconds === null) return "Unknown";
  if (epochSeconds * 1_000 <= Date.now()) return "Expired";
  const { days, hours, minutes } = countdownParts(epochSeconds);
  return days > 0 ? `${days}d ${hours}h ${minutes}m` : `${hours}h ${minutes}m`;
}

/** Same countdown truncated for surfaces that only have room for two units. */
export function formatCompactCountdown(epochSeconds: number | null): string {
  if (epochSeconds === null) return "—";
  if (epochSeconds * 1_000 <= Date.now()) return "Expired";
  const { days, hours, minutes } = countdownParts(epochSeconds);
  return days > 0 ? `${days}d ${hours}h` : `${hours}h ${minutes}m`;
}

/**
 * The core owns the parser and pricing revisions, so the renderer never carries a
 * copy of them; an empty value simply means no snapshot has arrived yet.
 */
export function formatRevision(value: string): string {
  return value.length === 0 ? "unavailable" : value.slice(0, 12);
}

export function formatResetTimestamp(epochSeconds: number | null): string {
  if (epochSeconds === null) return "Reset time unknown";
  return new Intl.DateTimeFormat(LOCALE, {
    dateStyle: "medium",
    timeStyle: "short",
    hour12: false,
  }).format(new Date(epochSeconds * 1_000));
}

/**
 * How far ahead of its published expiry a window restarted. Whole days carry the point
 * on their own; anything shorter is the polling interval and reads better in hours.
 */
export function formatEarlyBy(seconds: number): string {
  if (seconds >= 86_400) {
    const days = seconds / 86_400;
    return `${days.toFixed(days >= 10 ? 0 : 1)} days early`;
  }
  const hours = Math.round(seconds / 3_600);
  return hours <= 1 ? "under an hour early" : `${hours} hours early`;
}

function windowParts(durationMins: number) {
  if (durationMins % 1_440 === 0) return { value: durationMins / 1_440, unit: "day" };
  if (durationMins % 60 === 0) return { value: durationMins / 60, unit: "hour" };
  return { value: durationMins, unit: "minute" };
}

/** Badge form of a window's duration for the taskbar surface, for example 5H or 7D. */
export function formatWindowBadge(durationMins: number | null, fallback: string): string {
  if (durationMins === null) return fallback.slice(0, 2).toUpperCase();
  const { value, unit } = windowParts(durationMins);
  return `${value}${unit.charAt(0).toUpperCase()}`;
}

/**
 * The axis and stat-tile form of a token count. Charts have room for four characters, not
 * for eleven, and an axis of exact figures is read as a wall of digits rather than a scale.
 */
export function formatCompactNumber(value: number): string {
  if (!Number.isFinite(value)) return "—";
  const magnitude = Math.abs(value);
  if (magnitude >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)}B`;
  if (magnitude >= 1_000_000)
    return `${(value / 1_000_000).toFixed(magnitude >= 10_000_000 ? 0 : 1)}M`;
  if (magnitude >= 1_000) return `${(value / 1_000).toFixed(magnitude >= 10_000 ? 0 : 1)}K`;
  return formatNumber(Math.round(value));
}

/** Same idea for money: the axis says $4.2K, the figure beside it still says $4,231.09. */
export function formatCompactCurrency(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (Math.abs(value) >= 1_000) return `$${formatCompactNumber(value)}`;
  return `$${value.toFixed(Math.abs(value) >= 10 ? 0 : 2)}`;
}

/**
 * How a total moved against the period of the same length before it. `null` means the
 * comparison cannot be made — there was nothing before to compare with — which reads
 * differently from no change at all.
 */
export function formatDelta(current: number, previous: number): string | null {
  if (!Number.isFinite(current) || !Number.isFinite(previous) || previous <= 0) return null;
  const change = ((current - previous) / previous) * 100;
  if (Math.abs(change) < 0.05) return "0%";
  const rounded = Math.abs(change) >= 100 ? change.toFixed(0) : change.toFixed(1);
  return `${change > 0 ? "+" : ""}${rounded}%`;
}

/** A calendar day at chart-axis length, for example 3 Aug. */
export function formatAxisDate(value: string): string {
  return new Intl.DateTimeFormat(LOCALE, { day: "numeric", month: "short" }).format(
    new Date(`${value}T00:00:00`),
  );
}

/**
 * The clock part of an hour bucket, for example 14:00. The axis carries the date
 * separately, on the first label of each day, so the hours between it stay short.
 */
export function formatAxisHour(value: string): string {
  return `${value.slice(11, 13)}:00`;
}

/** The calendar day an hour bucket belongs to. */
export function hourDate(value: string): string {
  return value.slice(0, 10);
}
