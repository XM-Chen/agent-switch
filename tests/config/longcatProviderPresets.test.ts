import { describe, expect, it } from "vitest";
import { claudeDesktopProviderPresets } from "@/config/claudeDesktopProviderPresets";
import { providerPresets } from "@/config/claudeProviderPresets";
import { codexProviderPresets } from "@/config/codexProviderPresets";
import { hermesProviderPresets } from "@/config/hermesProviderPresets";
import { openclawProviderPresets } from "@/config/openclawProviderPresets";
import { opencodeProviderPresets } from "@/config/opencodeProviderPresets";

const longcat = <T extends { name: string }>(presets: T[]) =>
  presets.find((preset) => preset.name === "Longcat")!;

describe("LongCat 2.0 presets", () => {
  it("keeps the model, endpoint and capabilities aligned across applications", () => {
    const presets = [
      longcat(providerPresets),
      longcat(claudeDesktopProviderPresets),
      longcat(codexProviderPresets),
      longcat(hermesProviderPresets),
      longcat(openclawProviderPresets),
      longcat(opencodeProviderPresets),
    ];
    const serialized = JSON.stringify(presets);

    expect(serialized).not.toContain("LongCat-Flash-Chat");
    expect(serialized).not.toContain("LongCat-2.0-Preview");
    expect(serialized.match(/LongCat-2\.0/g)?.length).toBeGreaterThanOrEqual(6);

    const claw = longcat(openclawProviderPresets);
    expect(claw.settingsConfig.baseUrl).toBe(
      "https://api.longcat.chat/openai/v1",
    );
    expect(claw.settingsConfig.models?.[0]).toMatchObject({
      id: "LongCat-2.0",
      reasoning: false,
      input: ["text"],
      contextWindow: 1048576,
      maxTokens: 131072,
      compat: { maxTokensField: "max_tokens" },
    });
  });

  it("removes the redundant OpenAI-compatible template presets", () => {
    expect(
      openclawProviderPresets.some((p) => p.name === "OpenAI Compatible"),
    ).toBe(false);
    expect(
      opencodeProviderPresets.some((p) => p.name === "OpenAI Compatible"),
    ).toBe(false);
  });
});
