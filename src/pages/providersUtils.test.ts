import { describe, expect, it } from 'vitest';
import {
  APP_TYPES,
  canMoveDown,
  canMoveUp,
  groupByAppType,
  moveItem,
} from './providersUtils';
import type { Provider } from '../lib/api';

function makeProvider(
  id: string,
  appType: 'claude-code' | 'codex',
  sortIndex: number | null,
  overrides: Partial<Provider> = {},
): Provider {
  return {
    id,
    app_type: appType,
    name: `p-${id}`,
    mode: 'proxy',
    settings_config: {},
    is_current: false,
    category: null,
    sort_index: sortIndex,
    notes: null,
    meta: {},
    created_at: '2026-07-05T00:00:00Z',
    updated_at: '2026-07-05T00:00:00Z',
    ...overrides,
  };
}

describe('groupByAppType', () => {
  it('groups providers into claude-code and codex buckets', () => {
    const result = groupByAppType([
      makeProvider('a', 'claude-code', 1),
      makeProvider('b', 'codex', 0),
      makeProvider('c', 'claude-code', 0),
    ]);
    expect(result['claude-code'].map((p) => p.id)).toEqual(['c', 'a']);
    expect(result.codex.map((p) => p.id)).toEqual(['b']);
  });

  it('sorts each bucket by sort_index ascending with NULLS last', () => {
    const result = groupByAppType([
      makeProvider('a', 'claude-code', null),
      makeProvider('b', 'claude-code', 2),
      makeProvider('c', 'claude-code', null),
      makeProvider('d', 'claude-code', 0),
      makeProvider('e', 'claude-code', 1),
    ]);
    expect(result['claude-code'].map((p) => p.id)).toEqual([
      'd', // 0
      'e', // 1
      'b', // 2
      'a', // null
      'c', // null
    ]);
  });

  it('ignores providers with unsupported app_type', () => {
    const result = groupByAppType([
      makeProvider('a', 'claude-code', 0),
      makeProvider('b', 'opencode' as 'claude-code', 0),
    ]);
    expect(result['claude-code'].map((p) => p.id)).toEqual(['a']);
    expect(result.codex).toEqual([]);
  });

  it('returns empty buckets when no providers', () => {
    const result = groupByAppType([]);
    expect(result['claude-code']).toEqual([]);
    expect(result.codex).toEqual([]);
  });

  it('exposes APP_TYPES in stable order', () => {
    expect(APP_TYPES).toEqual(['claude-code', 'codex']);
  });
});

describe('canMoveUp / canMoveDown', () => {
  it('canMoveUp is false at index 0 and true otherwise', () => {
    expect(canMoveUp(0)).toBe(false);
    expect(canMoveUp(1)).toBe(true);
    expect(canMoveUp(3)).toBe(true);
  });

  it('canMoveDown respects length boundary', () => {
    expect(canMoveDown(0, 3)).toBe(true);
    expect(canMoveDown(2, 3)).toBe(false);
    expect(canMoveDown(3, 3)).toBe(false);
    expect(canMoveDown(0, 1)).toBe(false);
  });
});

describe('moveItem', () => {
  const items = [
    makeProvider('a', 'claude-code', 0),
    makeProvider('b', 'claude-code', 1),
    makeProvider('c', 'claude-code', 2),
  ];

  it('moves an item up and renumbers sort_index from 0', () => {
    // 把索引 2 的 c 上移到索引 1。
    const result = moveItem(items, 2, 1);
    expect(result).toEqual([
      { id: 'a', sort_index: 0 },
      { id: 'c', sort_index: 1 },
      { id: 'b', sort_index: 2 },
    ]);
  });

  it('moves an item down and renumbers sort_index from 0', () => {
    // 把索引 0 的 a 下移到索引 1。
    const result = moveItem(items, 0, 1);
    expect(result).toEqual([
      { id: 'b', sort_index: 0 },
      { id: 'a', sort_index: 1 },
      { id: 'c', sort_index: 2 },
    ]);
  });

  it('returns identity order when from === to', () => {
    const result = moveItem(items, 1, 1);
    expect(result).toEqual([
      { id: 'a', sort_index: 0 },
      { id: 'b', sort_index: 1 },
      { id: 'c', sort_index: 2 },
    ]);
  });

  it('clamps out-of-range from/to into valid bounds', () => {
    // from 越界 → 夹紧到末项再上移。
    const result = moveItem(items, 99, 0);
    expect(result).toEqual([
      { id: 'c', sort_index: 0 },
      { id: 'a', sort_index: 1 },
      { id: 'b', sort_index: 2 },
    ]);
  });

  it('handles empty list', () => {
    expect(moveItem([], 0, 0)).toEqual([]);
  });

  it('handles single item', () => {
    const single = [makeProvider('a', 'claude-code', 0)];
    expect(moveItem(single, 0, 0)).toEqual([{ id: 'a', sort_index: 0 }]);
  });
});
