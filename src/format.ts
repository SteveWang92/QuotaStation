export function formatNumber(value: number): string {
  return new Intl.NumberFormat("en-AU").format(value);
}

export function formatCurrency(value: number | null): string {
  if (value === null) return "Unavailable";
  return new Intl.NumberFormat("en-AU", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  }).format(value);
}

export function formatTimestamp(value: string | null): string {
  if (!value) return "Never";
  return new Intl.DateTimeFormat("en-AU", {
    dateStyle: "medium",
    timeStyle: "medium",
  }).format(new Date(value));
}

export function formatCountdown(epochSeconds: number | null): string {
  if (epochSeconds === null) return "Unknown";
  const remaining = Math.max(0, epochSeconds * 1000 - Date.now());
  const totalMinutes = Math.floor(remaining / 60_000);
  const days = Math.floor(totalMinutes / 1_440);
  const hours = Math.floor((totalMinutes % 1_440) / 60);
  const minutes = totalMinutes % 60;
  return days > 0 ? `${days}d ${hours}h ${minutes}m` : `${hours}h ${minutes}m`;
}

export function formatResetTimestamp(epochSeconds: number | null): string {
  if (epochSeconds === null) return "Reset time unknown";
  return new Intl.DateTimeFormat("en-AU", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(epochSeconds * 1000));
}
