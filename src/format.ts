export function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-AU").format(value);
}

export function formatCurrency(value: number | null): string {
  if (value === null) return "Unavailable";
  return new Intl.NumberFormat("en-AU", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

export function formatTimestamp(value: string | null): string {
  if (!value) return "Never";
  return new Intl.DateTimeFormat("en-AU", {
    dateStyle: "medium",
    timeStyle: "medium",
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
  const { days, hours, minutes } = countdownParts(epochSeconds);
  return days > 0 ? `${days}d ${hours}h ${minutes}m` : `${hours}h ${minutes}m`;
}

/** Same countdown truncated for surfaces that only have room for two units. */
export function formatCompactCountdown(epochSeconds: number | null): string {
  if (epochSeconds === null) return "—";
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
  return new Intl.DateTimeFormat("en-AU", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(epochSeconds * 1_000));
}

function windowParts(durationMins: number) {
  if (durationMins % 1_440 === 0) return { value: durationMins / 1_440, unit: "day" };
  if (durationMins % 60 === 0) return { value: durationMins / 60, unit: "hour" };
  return { value: durationMins, unit: "minute" };
}

export function formatWindowDuration(durationMins: number | null): string {
  if (durationMins === null) return "Window duration unavailable";
  const { value, unit } = windowParts(durationMins);
  return `${value}-${unit} quota window`;
}

/** Badge form of the same duration for the taskbar surface, for example 5H or 7D. */
export function formatWindowBadge(durationMins: number | null, fallback: string): string {
  if (durationMins === null) return fallback.slice(0, 2).toUpperCase();
  const { value, unit } = windowParts(durationMins);
  return `${value}${unit.charAt(0).toUpperCase()}`;
}
