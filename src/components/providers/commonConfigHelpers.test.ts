import { describe, expect, it } from 'vitest';
import {
  applyCommonConfigStateToMeta,
  commonConfigStateFromMeta,
  commonConfigStateToPayload,
  parseJsonObjectText,
} from './commonConfigHelpers';

describe('commonConfigHelpers', () => {
  it('parses only JSON objects', () => {
    expect(parseJsonObjectText('{"hooks":{"a":1}}').value).toEqual({ hooks: { a: 1 } });
    expect(parseJsonObjectText('', 'Common Config').value).toEqual({});
    expect(parseJsonObjectText('[1]', 'Common Config').error).toContain('必须是 JSON 对象');
    expect(parseJsonObjectText('null', 'Common Config').error).toContain('必须是 JSON 对象');
    expect(parseJsonObjectText('{', 'Common Config').error).toContain('不是合法 JSON');
  });

  it('maps meta.common_config_enabled to tri-state UI state', () => {
    expect(commonConfigStateFromMeta({ common_config_enabled: true })).toBe('enabled');
    expect(commonConfigStateFromMeta({ common_config_enabled: false })).toBe('disabled');
    expect(commonConfigStateFromMeta({ snapshot: { env: { KEEP: '1' } } })).toBe('default');
    expect(commonConfigStateFromMeta(null)).toBe('default');
  });

  it('serializes tri-state UI state to provider update payload', () => {
    expect(commonConfigStateToPayload('default')).toBeNull();
    expect(commonConfigStateToPayload('enabled')).toBe(true);
    expect(commonConfigStateToPayload('disabled')).toBe(false);
  });

  it('applies tri-state to meta without dropping snapshot env', () => {
    const meta = {
      common_config_enabled: false,
      snapshot: { env: { KEEP: '1' }, hooks: { a: 1 } },
      other: 'keep',
    };

    expect(applyCommonConfigStateToMeta(meta, 'enabled')).toEqual({
      common_config_enabled: true,
      snapshot: { env: { KEEP: '1' }, hooks: { a: 1 } },
      other: 'keep',
    });
    expect(applyCommonConfigStateToMeta(meta, 'default')).toEqual({
      snapshot: { env: { KEEP: '1' }, hooks: { a: 1 } },
      other: 'keep',
    });
  });
});
