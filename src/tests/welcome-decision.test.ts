import {test, expect} from 'bun:test';
import {hasSavedPreferences} from '../welcome-decision';
test('new install has no saved preferences',()=>expect(hasSavedPreferences({length:0,key:()=>null})).toBe(false));
test('upgrade keeps any saved preference',()=>expect(hasSavedPreferences({length:1,key:()=> 'tuck.live.v2'})).toBe(true));
test('unreadable storage suppresses welcome',()=>expect(hasSavedPreferences({get length():number{throw Error('unreadable')},key:()=>null})).toBe(true));
