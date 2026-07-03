import { describe, expect, it } from 'vitest';
import { buildLogsPath } from './api';

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
