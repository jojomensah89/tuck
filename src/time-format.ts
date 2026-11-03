const localeHourCycle = (new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
}).resolvedOptions() as Intl.ResolvedDateTimeFormatOptions & {
  hourCycle?: 'h11' | 'h12' | 'h23' | 'h24';
}).hourCycle;

const clockFormatter = new Intl.DateTimeFormat(undefined, {
  // 24-hour clocks keep the leading zero (07:33); 12-hour clocks drop it (7:33 AM)
  hour: localeHourCycle === 'h23' || localeHourCycle === 'h24' ? '2-digit' : 'numeric',
  minute: '2-digit',
  ...(localeHourCycle ? { hourCycle: localeHourCycle } : {}),
});

/** Formats clock time with the user's Windows locale and 12/24-hour cycle. */
export function formatClockTime(value: Date | number = new Date()): string {
  return clockFormatter.format(value);
}
