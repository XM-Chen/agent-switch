import { describe, expect, it } from 'vitest';
import {
  buildLogsPath,
  buildSessionMessagesPath,
  buildSessionsPath,
  providersApi,
  skillsApi,
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

describe('buildSessionsPath', () => {
  it('defaults to claude-code app_type', () => {
    expect(buildSessionsPath()).toBe('/sessions?app_type=claude-code');
  });

  it('includes search, limit and offset parameters', () => {
    const path = buildSessionsPath({ search: 'hello world', limit: 20, offset: 40 });
    expect(path).toContain('app_type=claude-code');
    expect(path).toContain('search=hello+world');
    expect(path).toContain('limit=20');
    expect(path).toContain('offset=40');
  });

  it('builds messages path with encoded source_path', () => {
    const path = buildSessionMessagesPath('C:\\Users\\me\\.claude\\projects\\a.jsonl');
    expect(path).toContain('/sessions/messages?');
    expect(path).toContain('app_type=claude-code');
    expect(path).toContain('source_path=C%3A%5CUsers%5Cme%5C.claude%5Cprojects%5Ca.jsonl');
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

describe('skillsApi 阶段 C 端点', () => {
  function makeJsonResponse(body: unknown, init?: { status?: number }): Response {
    return new Response(JSON.stringify(body), {
      status: init?.status ?? 200,
      headers: { 'Content-Type': 'application/json' },
    });
  }

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

  it('installRepo() POST /skills/install-repo，body 透传 repo/subdir', async () => {
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/skills\/install-repo$/);
      expect(init?.method).toBe('POST');
      const parsed = JSON.parse(String(init?.body));
      expect(parsed.repo).toBe('owner/name');
      expect(parsed.subdir).toBe('skills/foo');
      return makeJsonResponse({
        skill: { id: 's1', name: 'foo', directory: 'foo', content_hash: 'h' },
        sync: [],
      });
    });
    try {
      const r = await skillsApi.installRepo({ repo: 'owner/name', subdir: 'skills/foo' });
      expect(r.skill.directory).toBe('foo');
    } finally {
      restore();
    }
  });

  it('listBackups() 无 directory 时不带 query', async () => {
    const restore = mockFetch((url) => {
      expect(url).toMatch(/\/skills\/backups$/);
      expect(url).not.toContain('?');
      return makeJsonResponse([]);
    });
    try {
      await skillsApi.listBackups();
    } finally {
      restore();
    }
  });

  it('listBackups(dir) 对 directory 做 URL 编码', async () => {
    const restore = mockFetch((url) => {
      expect(url).toContain('/skills/backups?directory=my%20skill');
      return makeJsonResponse([]);
    });
    try {
      await skillsApi.listBackups('my skill');
    } finally {
      restore();
    }
  });

  it('uninstall() 发 DELETE /skills/{id}', async () => {
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/skills\/s1$/);
      expect(init?.method).toBe('DELETE');
      return makeJsonResponse({
        id: 's1',
        directory: 'foo',
        backup: { directory: 'foo', timestamp: '1', path: 'p', has_snapshot: true },
        sync: [],
      });
    });
    try {
      const r = await skillsApi.uninstall('s1');
      expect(r.backup.has_snapshot).toBe(true);
    } finally {
      restore();
    }
  });

  it('restore() POST body 含 directory 与 timestamp', async () => {
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/skills\/restore$/);
      const parsed = JSON.parse(String(init?.body));
      expect(parsed).toEqual({ directory: 'foo', timestamp: '123' });
      return makeJsonResponse({ directory: 'foo', restored_from: 'p', sync: [] });
    });
    try {
      await skillsApi.restore('foo', '123');
    } finally {
      restore();
    }
  });

  it('update() 缺省 ids 时发送 { ids: null }', async () => {
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/skills\/update$/);
      const parsed = JSON.parse(String(init?.body));
      expect(parsed.ids).toBeNull();
      return makeJsonResponse({ items: [] });
    });
    try {
      await skillsApi.update();
    } finally {
      restore();
    }
  });

  it('search() POST body 含 query', async () => {
    const restore = mockFetch((url, init) => {
      expect(url).toMatch(/\/skills\/search$/);
      const parsed = JSON.parse(String(init?.body));
      expect(parsed.query).toBe('pdf');
      return makeJsonResponse({ query: 'pdf', results: [] });
    });
    try {
      const r = await skillsApi.search('pdf');
      expect(r.query).toBe('pdf');
    } finally {
      restore();
    }
  });
});
