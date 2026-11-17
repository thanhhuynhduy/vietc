import {describe, expect, it} from 'vitest';
import {parseVni} from './vniParser';

describe('parseVni', () => {
  it.each([
    ['Vie6t5 Nam', 'Việt Nam'],
    ['tie61ng vie6t5', 'tiếng việt'],
    ['d9uo7ng2 sa1', 'đường sá'],
    ['ro6ng2 con', 'rồng con'],
  ])('converts %s to %s', (input, expected) => {
    expect(parseVni(input).text).toBe(expected);
  });

  it('preserves unsupported modifier digits', () => {
    expect(parseVni('b6').text).toBe('b6');
  });

  it('removes a pending tone when zero is entered', () => {
    expect(parseVni('a10').text).toBe('a');
  });

  it('preserves repeated spaces', () => {
    expect(parseVni('Viet65  Nam').text).toBe('Việt  Nam');
  });

  it('returns the idle state and message for empty input', () => {
    expect(parseVni('')).toEqual({
      text: '',
      logs: ['Chờ phím gõ từ terminal...'],
      state: 'S0',
    });
  });
});