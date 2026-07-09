export type CommonConfigState = 'default' | 'enabled' | 'disabled';

export function commonConfigStateFromMeta(meta: unknown): CommonConfigState {
  const root = asObject(meta);
  if (root?.common_config_enabled === true) return 'enabled';
  if (root?.common_config_enabled === false) return 'disabled';
  return 'default';
}

export function commonConfigStateToPayload(state: CommonConfigState): boolean | null {
  if (state === 'enabled') return true;
  if (state === 'disabled') return false;
  return null;
}

export function applyCommonConfigStateToMeta(
  meta: unknown,
  state: CommonConfigState,
): Record<string, unknown> {
  const root = { ...(asObject(meta) ?? {}) };
  if (state === 'default') {
    delete root.common_config_enabled;
  } else {
    root.common_config_enabled = state === 'enabled';
  }
  return root;
}

export function parseJsonObjectText(
  text: string,
  label = 'JSON',
): { value: Record<string, unknown>; error: null } | { value: null; error: string } {
  try {
    const parsed = text.trim() === '' ? {} : JSON.parse(text);
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return { value: parsed as Record<string, unknown>, error: null };
    }
    return { value: null, error: `${label} 必须是 JSON 对象` };
  } catch (e) {
    return {
      value: null,
      error: `${label} 不是合法 JSON: ${e instanceof Error ? e.message : String(e)}`,
    };
  }
}

function asObject(val: unknown): Record<string, unknown> | null {
  if (val && typeof val === 'object' && !Array.isArray(val)) {
    return val as Record<string, unknown>;
  }
  return null;
}
