import { describe, expect, test } from 'bun:test';
import { clipCategory, isClipboardLink } from '../clipboard-categories';
describe('Clipboard categories', () => {
  test('only a single http or https URL is a link', () => {
    for (const text of ['https://example.com/a?q=1', 'http://localhost:8080/']) expect(isClipboardLink(text)).toBe(true);
    for (const text of ['words', 'https://', 'file:///tmp/a', 'ftp://example.com', ' https://example.com', 'https://example.com hello', 'https://a.test\nhttps://b.test']) expect(isClipboardLink(text)).toBe(false);
  });
  test('explicit media and file types win over text', () => {
    expect(clipCategory({text:'hello'})).toBe('text');
    expect(clipCategory({text:'https://example.com'})).toBe('link');
    for (const kind of ['image', 'video', 'file'] as const) expect(clipCategory({text:'https://example.com',kind})).toBe(kind);
  });
});
