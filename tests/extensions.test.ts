import {describe,it,expect} from 'bun:test';
import {formatBytes,formatDuration,validateHttpUrl} from '../src/extensions-types.js';
describe('Extension presentation uses honest units and safe URLs',()=>{
 it('formats indeterminate progress bytes and real elapsed time',()=>{expect(formatBytes(2048)).toBe('2.0 KB');expect(formatBytes(NaN)).toBe('0 B');expect(formatDuration(61500)).toBe('01:01');});
 it('rejects privileged schemes, credentials and control characters',()=>{for(const url of ['javascript:alert(1)','file:///C:/a','https://user:pass@example.com','https://example.com/\n'])expect(validateHttpUrl(url)).toBeNull();expect(validateHttpUrl('https://example.com/meet')).toBe('https://example.com/meet');});
});
