import { describe, expect, it } from "vitest";
import {
  TIER_NONE_VALUE,
  buildTierOptions,
  decodeAggregateRef,
  encodeAggregateRef,
  resolveTierRef,
  tierRefToSelectValue,
  formatFetchedAt,
  formatRfc3339,
} from "./aggregationHelpers";
import type { AggregateView, CustomAggregateView } from "@/lib/api/aggregation";

const autoView = (key: string, memberCount = 1): AggregateView => ({
  key,
  members: Array.from({ length: memberCount }, (_, i) => ({
    providerId: `p${i}`,
    providerName: `Provider ${i}`,
    modelId: key,
    source: "fetched" as const,
  })),
});

const customView = (
  id: string,
  name: string,
  overrides: Partial<CustomAggregateView> = {},
): CustomAggregateView => ({
  id,
  name,
  members: [],
  orderedMembers: [],
  isEmpty: true,
  missingMembers: [],
  ...overrides,
});

describe("encode/decode aggregate ref", () => {
  it("round-trips an auto ref", () => {
    const ref = { type: "auto" as const, value: "glm-4.6" };
    expect(decodeAggregateRef(encodeAggregateRef(ref))).toEqual(ref);
  });

  it("round-trips a custom ref", () => {
    const ref = { type: "custom" as const, value: "uuid-123" };
    expect(decodeAggregateRef(encodeAggregateRef(ref))).toEqual(ref);
  });

  it("handles keys containing slashes and dots", () => {
    const ref = { type: "auto" as const, value: "zhipu/glm-4.6" };
    expect(decodeAggregateRef(encodeAggregateRef(ref))).toEqual(ref);
  });

  it("returns null for the none sentinel", () => {
    expect(decodeAggregateRef(TIER_NONE_VALUE)).toBeNull();
  });

  it("returns null for malformed input", () => {
    expect(decodeAggregateRef("bogus")).toBeNull();
    expect(decodeAggregateRef("unknown:x")).toBeNull();
  });
});

describe("buildTierOptions", () => {
  it("marks empty aggregates", () => {
    const { auto, custom } = buildTierOptions(
      [autoView("a", 2), autoView("b", 0)],
      [
        customView("c1", "Custom A", { isEmpty: false }),
        customView("c2", "Custom B", { isEmpty: true }),
      ],
    );
    expect(auto).toEqual([
      { value: "auto:a", label: "a", isEmpty: false },
      { value: "auto:b", label: "b", isEmpty: true },
    ]);
    expect(custom).toEqual([
      { value: "custom:c1", label: "Custom A", isEmpty: false },
      { value: "custom:c2", label: "Custom B", isEmpty: true },
    ]);
  });
});

describe("tierRefToSelectValue", () => {
  it("returns the none sentinel for undefined", () => {
    expect(tierRefToSelectValue(undefined)).toBe(TIER_NONE_VALUE);
  });

  it("encodes a defined ref", () => {
    expect(tierRefToSelectValue({ type: "custom", value: "x" })).toBe(
      "custom:x",
    );
  });
});

describe("resolveTierRef", () => {
  const aggregates = [autoView("a", 2), autoView("empty", 0)];
  const customs = [customView("c1", "C1", { isEmpty: false })];

  it("returns null for undefined ref", () => {
    expect(resolveTierRef(undefined, aggregates, customs)).toBeNull();
  });

  it("resolves an existing auto ref", () => {
    expect(
      resolveTierRef({ type: "auto", value: "a" }, aggregates, customs),
    ).toEqual({ resolved: true, label: "a", isEmpty: false });
  });

  it("flags an empty auto aggregate", () => {
    expect(
      resolveTierRef({ type: "auto", value: "empty" }, aggregates, customs),
    ).toEqual({ resolved: true, label: "empty", isEmpty: true });
  });

  it("flags a stale (missing) auto ref", () => {
    expect(
      resolveTierRef({ type: "auto", value: "gone" }, aggregates, customs),
    ).toEqual({ resolved: false, label: "gone", isEmpty: true });
  });

  it("resolves an existing custom ref by name", () => {
    expect(
      resolveTierRef({ type: "custom", value: "c1" }, aggregates, customs),
    ).toEqual({ resolved: true, label: "C1", isEmpty: false });
  });

  it("flags a stale custom ref", () => {
    expect(
      resolveTierRef({ type: "custom", value: "gone" }, aggregates, customs),
    ).toEqual({ resolved: false, label: "gone", isEmpty: true });
  });
});

describe("formatters", () => {
  it("returns empty string for zero/invalid epoch", () => {
    expect(formatFetchedAt(0)).toBe("");
    expect(formatFetchedAt(undefined)).toBe("");
    expect(formatFetchedAt(-1)).toBe("");
  });

  it("formats a valid epoch to a non-empty string", () => {
    expect(formatFetchedAt(1_700_000_000_000)).not.toBe("");
  });

  it("returns empty string for missing/invalid rfc3339", () => {
    expect(formatRfc3339(undefined)).toBe("");
    expect(formatRfc3339("")).toBe("");
    expect(formatRfc3339("not-a-date")).toBe("");
  });

  it("formats a valid rfc3339 string", () => {
    expect(formatRfc3339("2026-07-13T04:00:00Z")).not.toBe("");
  });
});
