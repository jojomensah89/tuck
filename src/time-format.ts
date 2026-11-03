const localeHourCycle = (new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
}).resolvedOptions() as Intl.ResolvedDateTimeFormatOptions & {
  hourCycle?: 'h11' | 'h12' | 'h23' | 'h24';
}).hourCycle;

const clockFormatter = new Intl.DateTimeFormat(undefined, {
  hour: '2-digit',
  minute: '2-digit',
  ...(localeHourCycle ? { hourCycle: localeHourCycle } : {}),
});

/** Formats clock time with the user's Windows locale and 12/24-hour cycle. */
export function formatClockTime(value: Date | number = new Date()): string {
  return clockFormatter.format(value);
}
