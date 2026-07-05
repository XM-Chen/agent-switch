import { describe, expect, it } from 'vitest';
import {
  APP_TYPE_LABELS,
  CATEGORY_COLORS,
  CATEGORY_LABELS,
  MODE_COLORS,
  MODE_LABELS,
  TOOL_LABELS,
} from './presentation';

describe('presentation label maps', () => {
  it('contains known tool labels from tool takeover API', () => {
    expect(TOOL_LABELS['claude-code']).toBe('Claude Code');
    expect(TOOL_LABELS.codex).toBe('Codex');
    expect(TOOL_LABELS.opencode).toBe('OpenCode');
  });

  it('contains labels and colors for known live categories', () => {
    for (const category of [
      'agent_switch',
      'official',
      'third_party',
      'unconfigured',
      'unrecognized',
    ]) {
      expect(CATEGORY_LABELS[category]).toBeTruthy();
      expect(CATEGORY_COLORS[category]).toBeTruthy();
    }
  });

  it('contains app_type labels for switcher page', () => {
    expect(APP_TYPE_LABELS['claude-code']).toBe('Claude Code');
    expect(APP_TYPE_LABELS.codex).toBe('Codex');
  });

  it('contains mode labels and colors for proxy and direct', () => {
    expect(MODE_LABELS.proxy).toBe('代理');
    expect(MODE_LABELS.direct).toBe('直连');
    expect(MODE_COLORS.proxy).toBeTruthy();
    expect(MODE_COLORS.direct).toBeTruthy();
  });

  it('uses distinct colors for mode vs category badges', () => {
    // mode 颜色不应与现有 category 颜色完全相同，避免视觉混淆。
    expect(MODE_COLORS.proxy).not.toBe(CATEGORY_COLORS.official);
    expect(MODE_COLORS.direct).not.toBe(CATEGORY_COLORS.official);
  });
});
