import {describe,it,expect} from 'bun:test';
import {formatBytes} from '../src/extensions-types.js';
describe('File shelf byte labels',()=>{
 it('formats file sizes and handles unknown sizes',()=>{expect(formatBytes(2048)).toBe('2.0 KB');expect(formatBytes(NaN)).toBe('0 B');});
});
