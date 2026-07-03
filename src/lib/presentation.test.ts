import { describe, expect, it } from 'vitest';
import { CATEGORY_COLORS, CATEGORY_LABELS, TOOL_LABELS } from './presentation';

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
});
