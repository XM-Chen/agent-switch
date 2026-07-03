import { describe, expect, it } from 'vitest';
import { aggregateEndpointHealth, bucketHealth, countFallbackHops } from './dashboardUtils';

describe('bucketHealth', () => {
  const now = Date.now();

  it('returns "cooling" when cooldown_until is in the future', () => {
    const cooldownUntil = new Date(now + 10000).toISOString();
    expect(bucketHealth(cooldownUntil, null, null, now)).toBe('cooling');
  });

  it('returns "recent_failure" when last_failure_at is within 1 hour', () => {
    const lastFailureAt = new Date(now - 30 * 60 * 1000).toISOString();
    expect(bucketHealth(null, lastFailureAt, null, now)).toBe('recent_failure');
  });

  it('returns "normal" when last_success_at exists and no recent failure', () => {
    const lastSuccessAt = new Date(now - 5 * 60 * 1000).toISOString();
    expect(bucketHealth(null, null, lastSuccessAt, now)).toBe('normal');
  });

  it('returns "idle" when no indicators are present', () => {
    expect(bucketHealth(null, null, null, now)).toBe('idle');
  });

  it('prioritizes cooling over recent_failure', () => {
    const cooldownUntil = new Date(now + 10000).toISOString();
    const lastFailureAt = new Date(now - 5 * 60 * 1000).toISOString();
    expect(bucketHealth(cooldownUntil, lastFailureAt, null, now)).toBe('cooling');
  });
});

describe('aggregateEndpointHealth', () => {
  const now = Date.now();

  it('aggregates endpoints health correctly', () => {
    const endpoints = [
      { cooldown_until: new Date(now + 1000).toISOString(), last_failure_at: null, last_success_at: null },
      { cooldown_until: null, last_failure_at: new Date(now - 30 * 60 * 1000).toISOString(), last_success_at: null },
      { cooldown_until: null, last_failure_at: null, last_success_at: new Date(now - 5 * 60 * 1000).toISOString() },
      { cooldown_until: null, last_failure_at: null, last_success_at: null },
    ];

    const result = aggregateEndpointHealth(endpoints, [], now);
    expect(result).toEqual({ normal: 1, cooling: 1, recentFailure: 1, idle: 1 });
  });

  it('falls back to routes candidates when endpoints is empty', () => {
    const routes = [
      {
        candidates: [
          { cooldown_until: null, last_failure_at: null, last_success_at: new Date(now - 5 * 60 * 1000).toISOString() },
        ],
      },
    ];

    const result = aggregateEndpointHealth([], routes, now);
    expect(result).toEqual({ normal: 1, cooling: 0, recentFailure: 0, idle: 0 });
  });

  it('prioritizes endpoints over routes candidates when both present', () => {
    const endpoints = [
      { cooldown_until: null, last_failure_at: null, last_success_at: new Date(now - 5 * 60 * 1000).toISOString() },
    ];
    const routes = [
      {
        candidates: [
          { cooldown_until: null, last_failure_at: null, last_success_at: null },
        ],
      },
    ];

    const result = aggregateEndpointHealth(endpoints, routes, now);
    expect(result).toEqual({ normal: 1, cooling: 0, recentFailure: 0, idle: 0 });
  });
});

describe('countFallbackHops', () => {
  it('returns 0 for null chain', () => {
    expect(countFallbackHops(null)).toBe(0);
  });

  it('returns 0 for empty array', () => {
    expect(countFallbackHops('[]')).toBe(0);
  });

  it('returns correct hop count for valid array', () => {
    const chain = JSON.stringify([
      { endpoint_id: 'ep1', model: 'gpt-4', status: 500, error: 'timeout' },
      { endpoint_id: 'ep2', model: 'gpt-4', status: 200, error: null },
    ]);
    expect(countFallbackHops(chain)).toBe(2);
  });

  it('returns 0 for invalid JSON', () => {
    expect(countFallbackHops('not-json')).toBe(0);
  });

  it('returns 0 for non-array JSON', () => {
    expect(countFallbackHops('{"key":"value"}')).toBe(0);
  });
});
