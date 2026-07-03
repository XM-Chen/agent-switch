import { describe, expect, it } from 'vitest';
import { formatTime } from './format';

describe('formatTime', () => {
  it('formats ISO time as HH:mm:ss in local time', () => {
    const iso = '2026-07-03T08:09:10Z';
    const expected = new Date(iso).toLocaleTimeString('en-GB', { hour12: false });

    expect(formatTime(iso)).toBe(expected);
  });

  it('returns original text for invalid date input', () => {
    expect(formatTime('not-a-date')).toBe('not-a-date');
  });
});
