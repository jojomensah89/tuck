import { describe, expect, test } from 'bun:test';
import { formatBytes, scrollEdges, scrollPage, ThumbnailMemo, type Thumb } from '../extensions-types';

describe('file sizes', () => {
  test('bytes and kilobytes are whole numbers, larger sizes keep one decimal', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(20)).toBe('20 B');
    expect(formatBytes(1023)).toBe('1023 B');
    expect(formatBytes(1024)).toBe('1 KB');
    expect(formatBytes(158_822)).toBe('155 KB');
    expect(formatBytes(16_384)).toBe('16 KB');
    expect(formatBytes(4.04 * 1024 ** 2)).toBe('4.0 MB');
    expect(formatBytes(17.4 * 1024 ** 2)).toBe('17.4 MB');
    expect(formatBytes(258.6 * 1024 ** 2)).toBe('258.6 MB');
    expect(formatBytes(1.5 * 1024 ** 3)).toBe('1.5 GB');
  });
  test('rounding up to the next unit moves to it', () => {
    expect(formatBytes(1024 * 1024 - 1)).toBe('1.0 MB');
    expect(formatBytes(1024 ** 3 - 1)).toBe('1.0 GB');
  });
  test('nonsense reads as nothing', () => {
    expect(formatBytes(Number.NaN)).toBe('0 B');
    expect(formatBytes(-5)).toBe('0 B');
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe('0 B');
  });
});

describe('sideways scrolling', () => {
  test('arrows appear only where there is more to scroll', () => {
    expect(scrollEdges(0, 420, 420)).toEqual({ before: false, after: false });
    expect(scrollEdges(0, 800, 420)).toEqual({ before: false, after: true });
    expect(scrollEdges(200, 800, 420)).toEqual({ before: true, after: true });
    expect(scrollEdges(379.5, 800, 420)).toEqual({ before: true, after: false });
  });
  test('an arrow moves a page of whole tiles', () => {
    expect(scrollPage(420, 104)).toBe(312);
    expect(scrollPage(360, 104)).toBe(208);
    expect(scrollPage(100, 104)).toBe(104);
  });
});

describe('thumbnail memo', () => {
  test('asks once per item and forgets items that leave', async () => {
    const asked: string[] = [];
    const memo = new ThumbnailMemo(async id => { asked.push(id); return { kind: 'none' } as Thumb; });
    await memo.get('a'); await memo.get('a'); await memo.get('b');
    expect(asked).toEqual(['a', 'b']);
    expect(memo.peek('a')).toEqual({ kind: 'none' });
    memo.keep(['b']);
    expect(memo.size).toBe(1);
    expect(memo.peek('a')).toBeUndefined();
    await memo.get('a');
    expect(asked).toEqual(['a', 'b', 'a']);
  });
  test('a failed request settles as no thumbnail and is not retried on re-render', async () => {
    let calls = 0;
    const memo = new ThumbnailMemo(async () => { calls++; throw new Error('worker gone'); });
    expect(await memo.get('x')).toEqual({ kind: 'none' });
    expect(await memo.get('x')).toEqual({ kind: 'none' });
    expect(calls).toBe(1);
  });
});
