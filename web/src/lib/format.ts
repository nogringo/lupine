const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function bytes(n: number): string {
  let value = n;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

const DATE = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });
const TIME = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

export function date(millis: number | undefined): string {
  if (!millis) return "";
  const when = new Date(millis);
  const today = new Date();
  return when.toDateString() === today.toDateString() ? TIME.format(when) : DATE.format(when);
}

export function message(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** A shortened npub, for display. */
export function short(npub: string): string {
  return npub.length > 20 ? `${npub.slice(0, 12)}...${npub.slice(-6)}` : npub;
}
