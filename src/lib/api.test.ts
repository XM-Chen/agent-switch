import { describe, expect, it } from 'vitest';
import {
  buildLogsPath,
  providersApi,
  type CreateProviderBody,
  type Provider,
} from './api';

describe('buildLogsPath / log_type filtering', () => {
  it('includes log_type=production when log_type is production', () => {
    const path = buildLogsPath({ log_type: 'production' });
    expect(path).toContain('log_type=production');
  });

  it('includes log_type=test when log_type is test', () => {
    const path = buildLogsPath({ log_type: 'test' });
    expect(path).toContain('log_type=test');
  });

  it('omits log_type when not provided', () => {
    const path = buildLogsPath({ tool: 'claude-code' });
    expect(path).not.toContain('log_type=');
    expect(path).toContain('tool=claude-code');
  });

  it('combines log_type=production with tool parameter for narrower production filter', () => {
    const path = buildLogsPath({ log_type: 'production', tool: 'codex' });
    expect(path).toContain('log_type=production');
    expect(path).toContain('tool=codex');
  });

  it('uses log_type=test over tool parameter when both are set', () => {
    // LogsPage builds params with log_type precedence; buildLogsPath just emits both,
    // backend ignores tool when log_type=test (see request_logs.rs).
    const path = buildLogsPath({ log_type: 'test', tool: 'claude-code' });
    expect(path).toContain('log_type=test');
    expect(path).toContain('tool=claude-code');
  });

  it('encodes status, limit, offset, from, to parameters', () => {
    const path = buildLogsPath({
      status: 200,
      limit: 20,
      offset: 40,
      from: '2026-07-01',
      to: '2026-07-03',
    });
    expect(path).toContain('status=200');
    expect(path).toContain('limit=20');
    expect(path).toContain('offset=40');
    expect(path).toContain('from=2026-07-01');
    expect(path).toContain('to=2026-07-03');
  });

  it('returns bare /logs when no params are provided', () => {
    expect(buildLogsPath()).toBe('/logs');
    expect(buildLogsPath({})).toBe('/logs');
  });
});

describe('providersApi', () => {
  const sampleProvider: Provider = {
    id: 'p1',
    app_type: 'claude-code',
    name: 'P1',
    mode: 'proxy',
    settings_config: {},
    is_current: false,
    category: null,
    sort_index: 0,
    notes: null,
    meta: {},
    created_at: '2026-07-05T00:00:00Z',
    updated_at: '2026-07-05T00:00:00Z',
  };

  function makeJsonResponse(body: unknown, init?: { status?: number }): Response {
    return new Response(JSON.stringify(body), {
      status: init?.status ?? 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }

  /** 临时替换 globalThis.fetch，返回恢复函数。 */
  function mockFetch(
    handler: (url: string, init?: RequestInit) => Response | Promise<Response>,
  ): () => void {
    const original = globalThis.fetch;
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : String(input);
      return handler(url, init);
    }) as typeof fetch;
    return () => {
      globalThis.fetch = original;
    };
  }

  it('list() encodes app_type into query string', async () => {
    const restore = mockFetch((url) => {
      expect(url).toContain('/providers?app_type=claude-code');
      return makeJsonResponse([sampleProvider]);
    });
    try {
      const result = await providersApi.list('claude-code');
      expect(result).toEqual([sampleProvider]);
    } finally {
      restore();
    }
  });

  it('create() posts the body as JSON', async () => {
    const body: CreateProviderBody = {
      app_type: 'codex',
      name: 'new',
      mode: 'direct',
      settings_config: { foo: 'bar' },
    };
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/providers$/);
      expect(init?.method).toBe('POST');
      const parsed = JSON.parse(String(init?.body));
      expect(parsed).toEqual(body);
      return makeJsonResponse({ ...sampleProvider, id: 'new', ...body });
    });
    try {
      const result = await providersApi.create(body);
      expect(result.id).toBe('new');
      expect(result.mode).toBe('direct');
    } finally {
      restore();
    }
  });

  it('switch() returns { warnings } shape', async () => {
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/providers\/p1\/switch$/);
      expect(init?.method).toBe('POST');
      return makeJsonResponse({ warnings: ['备份跳过'] });
    });
    try {
      const result = await providersApi.switch('p1');
      expect(result.warnings).toEqual(['备份跳过']);
    } finally {
      restore();
    }
  });

  it('reorder() posts { items } payload', async () => {
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/providers\/reorder$/);
      expect(init?.method).toBe('POST');
      const parsed = JSON.parse(String(init?.body));
      expect(parsed.items).toEqual([
        { id: 'a', sort_index: 0 },
        { id: 'b', sort_index: 1 },
      ]);
      return new Response(null, { status: 204 });
    });
    try {
      await expect(
        providersApi.reorder([
          { id: 'a', sort_index: 0 },
          { id: 'b', sort_index: 1 },
        ]),
      ).resolves.toBeUndefined();
    } finally {
      restore();
    }
  });

  it('remove() issues DELETE', async () => {
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/providers\/p1$/);
      expect(init?.method).toBe('DELETE');
      return new Response(null, { status: 204 });
    });
    try {
      await expect(providersApi.remove('p1')).resolves.toBeUndefined();
    } finally {
      restore();
    }
  });
});
