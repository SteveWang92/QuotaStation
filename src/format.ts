import type { LimitResetEvent, ResetDetection } from "./types";
import { currentZone, dateOf, now, today } from "./zone";

/**
 * The interface is English-only, so every surface formats numbers and dates the same way
 * instead of following whatever locale the machine reports. This single constant is the
 * place to revisit once the interface offers a language choice of its own.
 *
 * Clock times are always 24-hour. Quota windows restart at arbitrary times of day and the
 * surfaces sit beside countdowns, so an am/pm marker is one more thing to read before two
 * timestamps can be compared.
 *
 * Every instant is written out in the application zone (see `zone.ts`); a calendar date,
 * which is a label rather than an instant, is written out in UTC so no zone can move it.
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
    timeZone: currentZone(),
  }).format(new Date(value));
}

function countdownParts(epochSeconds: number) {
  const totalMinutes = Math.floor(Math.max(0, epochSeconds * 1_000 - now()) / 60_000);
  return {
    days: Math.floor(totalMinutes / 1_440),
    hours: Math.floor((totalMinutes % 1_440) / 60),
    minutes: totalMinutes % 60,
  };
}

export function formatCountdown(epochSeconds: number | null): string {
  if (epochSeconds === null) return "Unknown";
  if (epochSeconds * 1_000 <= now()) return "Expired";
  const { days, hours, minutes } = countdownParts(epochSeconds);
  return days > 0 ? `${days}d ${hours}h ${minutes}m` : `${hours}h ${minutes}m`;
}

/** Same countdown truncated for surfaces that only have room for two units. */
export function formatCompactCountdown(epochSeconds: number | null): string {
  if (epochSeconds === null) return "—";
  if (epochSeconds * 1_000 <= now()) return "Expired";
  const { days, hours, minutes } = countdownParts(epochSeconds);
  return days > 0 ? `${days}d ${hours}h` : `${hours}h ${minutes}m`;
}

/**
 * How far this computer's clock is off, in words, once it is off by more than two minutes;
 * nothing below that, where a countdown is still right to the minute it shows.
 */
export function formatClockOffset(offsetMs: number): string | null {
  const minutes = Math.round(Math.abs(offsetMs) / 60_000);
  if (Math.abs(offsetMs) <= 120_000) return null;
  // The offset is what has to be added to this clock, so a negative one means it is ahead.
  return `This computer's clock is ${minutes} min ${offsetMs < 0 ? "fast" : "slow"}`;
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
    timeZone: currentZone(),
  }).format(new Date(epochSeconds * 1_000));
}

/**
 * A past moment at panel width: the clock alone for today, the day in front of it before
 * that. The date is what a weekly window's restart needs and a five-hour one never does.
 */
export function formatShortMoment(epochSeconds: number): string {
  const moment = new Date(epochSeconds * 1_000);
  const sameDay = dateOf(moment.getTime()) === today();
  return new Intl.DateTimeFormat(LOCALE, {
    day: sameDay ? undefined : "numeric",
    month: sameDay ? undefined : "short",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: currentZone(),
  }).format(moment);
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

function detectionDevice(detection: ResetDetection): string {
  return detection.deviceName ?? "An earlier record";
}

/** Each device that detected a restart, named once, the one whose timing it carries first. */
export function formatDetectedBy(event: LimitResetEvent): string {
  return [...new Set(event.detections.map(detectionDevice))].join(", ");
}

/**
 * "Seen on N devices" for a restart more than one device detected, and nothing for the
 * ordinary case of one: a tooltip that always says it would be read as saying nothing.
 */
export function formatSeenOn(event: LimitResetEvent): string | null {
  const devices = new Set(event.detections.map(detectionDevice)).size;
  return devices > 1 ? `Seen on ${devices} devices` : null;
}

/**
 * How far the devices' timing of a restart disagreed, once it is more than the second or
 * two of rounding every provider's expiry carries.
 */
export function formatAnchorSpread(event: LimitResetEvent): string | null {
  if (event.anchorSpreadSeconds <= 60) return null;
  return `±${Math.round(event.anchorSpreadSeconds / 60)} min`;
}

/**
 * Every device's detection of a restart, one per line, for a tooltip. A device that judged
 * the restart differently is kept and said so rather than outvoted.
 */
export function describeDetections(event: LimitResetEvent): string {
  const lines = event.detections.map(
    (detection) =>
      `${detectionDevice(detection)} · ${detection.source === "live" ? "live read" : "rollout log"} · ${formatResetTimestamp(detection.anchoredAt)} · ${detection.classification}`,
  );
  const dissent = event.detections.find(
    (detection) => detection.classification !== event.classification,
  );
  if (dissent) lines.push(`Another device recorded this as ${dissent.classification}`);
  return lines.join("\n");
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
  const magnitude = Math.abs(value);
  if (magnitude >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)}B`;
  if (magnitude >= 1_000_000)
    return `${(value / 1_000_000).toFixed(magnitude >= 10_000_000 ? 0 : 1)}M`;
  if (magnitude >= 1_000) return `${(value / 1_000).toFixed(magnitude >= 10_000 ? 0 : 1)}K`;
  return formatNumber(Math.round(value));
}

/** Same idea for money: the axis says $4.2K, the figure beside it still says $4,231.09. */
export function formatCompactCurrency(value: number): string {
  if (Math.abs(value) >= 1_000) return `$${formatCompactNumber(value)}`;
  return `$${value.toFixed(Math.abs(value) >= 10 ? 0 : 2)}`;
}

/**
 * How a total moved against the period of the same length before it. `null` means the
 * comparison cannot be made — there was nothing before to compare with — which reads
 * differently from no change at all.
 */
export function formatDelta(current: number, previous: number): string | null {
  if (previous <= 0) return null;
  const change = ((current - previous) / previous) * 100;
  if (Math.abs(change) < 0.05) return "0%";
  const rounded = Math.abs(change) >= 100 ? change.toFixed(0) : change.toFixed(1);
  return `${change > 0 ? "+" : ""}${rounded}%`;
}

/**
 * A past moment written out with its day, for example 20 Sep 14:32. Sessions are listed
 * against each other rather than against now, so the day is always there to read.
 */
export function formatDayAndTime(value: string): string {
  return new Intl.DateTimeFormat(LOCALE, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: currentZone(),
  }).format(new Date(value));
}

/** How long something ran, at table width: 2h 14m, 48m, or 36s under a minute. */
export function formatDuration(milliseconds: number): string {
  const seconds = Math.max(0, Math.round(milliseconds / 1_000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** A calendar day at chart-axis length, for example 3 Aug. */
export function formatAxisDate(value: string): string {
  return new Intl.DateTimeFormat(LOCALE, {
    day: "numeric",
    month: "short",
    timeZone: "UTC",
  }).format(new Date(`${value}T00:00:00Z`));
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
