/**
 * The zone every displayed time and every date and hour bucket follows: the one chosen in
 * Settings, or the Windows zone, as the core resolved it. The core keys every stored day
 * and hour in this zone, so the renderer has to read the calendar in it too, and a `Date`'s
 * own getters only ever read the machine's zone.
 *
 * Until the settings arrive it is `undefined`, which `Intl` reads as the machine's zone —
 * the same answer the core gives while nothing is chosen.
 */
let zone: string | undefined;

export function setZone(name: string): void {
  zone = name;
}

/** The zone to hand `Intl.DateTimeFormat` as its `timeZone`. */
export function currentZone(): string | undefined {
  return zone;
}

export const HOUR_MS = 3_600_000;
const DAY_MS = 86_400_000;

function parts(epochMs: number) {
  const values: Record<string, string> = {};
  for (const part of new Intl.DateTimeFormat("en-CA", {
    timeZone: zone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  }).formatToParts(new Date(epochMs))) {
    values[part.type] = part.value;
  }
  return values;
}

/** The calendar day an instant falls on, as `YYYY-MM-DD`. */
export function dateOf(epochMs: number): string {
  const { year, month, day } = parts(epochMs);
  return `${year}-${month}-${day}`;
}

/** The hour an instant falls in, as `YYYY-MM-DDTHH:00` — how every hourly bucket is keyed. */
export function hourOf(epochMs: number): string {
  return `${dateOf(epochMs)}T${parts(epochMs).hour}:00`;
}

export function today(): string {
  return dateOf(Date.now());
}

/** A calendar date moved by whole days. Dates are labels, so no zone enters this. */
export function addDays(date: string, days: number): string {
  return new Date(Date.parse(`${date}T00:00:00Z`) + days * DAY_MS).toISOString().slice(0, 10);
}

/** How far the zone's clock is ahead of UTC at an instant, in milliseconds. */
function offsetAt(epochMs: number): number {
  const { year, month, day, hour, minute, second } = parts(epochMs);
  const wall = Date.UTC(+year, +month - 1, +day, +hour, +minute, +second);
  return wall - Math.floor(epochMs / 1_000) * 1_000;
}

/** The wall-clock time an instant reads as, as `YYYY-MM-DDTHH:MM`. */
function wallOf(epochMs: number): string {
  const { hour, minute } = parts(epochMs);
  return `${dateOf(epochMs)}T${hour}:${minute}`;
}

/**
 * The instant a wall-clock time (`YYYY-MM-DDTHH:MM`) occurs at. A time repeated when the
 * clocks go back is its first occurrence; a time skipped when they go forward is read in
 * the offset before the change, which lands just after the gap.
 */
export function instantOf(wall: string): number {
  const asUtc = Date.parse(`${wall}:00Z`);
  // A zone changes its offset months apart, so the offsets half a day either side are the
  // two this wall time can be read in.
  const before = asUtc - offsetAt(asUtc - DAY_MS / 2);
  const after = asUtc - offsetAt(asUtc + DAY_MS / 2);
  const occurrences = [before, after].filter((instant) => wallOf(instant) === wall);
  return occurrences.length > 0 ? Math.min(...occurrences) : before;
}
/**
 * Every hour from one instant to another inclusive, as bucket keys. The hours are stepped
 * as 3600 seconds rather than as clock readings, so a day the clocks change on has the 23 or
 * 25 hours it really had; a repeated hour is one bucket, as it is in the core.
 */
export function hoursBetween(fromMs: number, untilMs: number): string[] {
  const hours: string[] = [];
  for (let instant = fromMs; instant <= untilMs; instant += HOUR_MS) {
    const key = hourOf(instant);
    if (hours[hours.length - 1] !== key) hours.push(key);
  }
  return hours;
}
