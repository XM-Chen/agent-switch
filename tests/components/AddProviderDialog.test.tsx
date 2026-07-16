import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AddProviderDialog } from "@/components/providers/AddProviderDialog";
import type { ProviderFormValues } from "@/components/providers/forms/ProviderForm";
import type { UniversalProvider } from "@/types";

const universalMocks = vi.hoisted(() => ({
  upsert: vi.fn(),
  sync: vi.fn(),
  warning: vi.fn(),
}));
let saveUniversal: ((provider: UniversalProvider) => void) | undefined;

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    universalProvidersApi: {
      upsert: (...args: unknown[]) => universalMocks.upsert(...args),
      sync: (...args: unknown[]) => universalMocks.sync(...args),
    },
  };
});

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: universalMocks.warning },
}));

vi.mock("@/components/universal", () => ({
  UniversalProviderPanel: () => <div>universal-panel</div>,
}));

vi.mock("@/components/universal/UniversalProviderFormModal", () => ({
  UniversalProviderFormModal: ({
    onSave,
  }: {
    onSave: (provider: UniversalProvider) => void;
  }) => {
    saveUniversal = onSave;
    return null;
  },
}));

vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DialogContent: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DialogHeader: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DialogTitle: ({ children }: { children: React.ReactNode }) => (
    <h1>{children}</h1>
  ),
  DialogDescription: ({ children }: { children: React.ReactNode }) => (
    <p>{children}</p>
  ),
  DialogFooter: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
}));

let mockFormValues: ProviderFormValues;

vi.mock("@/components/providers/forms/ProviderForm", () => ({
  ProviderForm: ({
    onSubmit,
  }: {
    onSubmit: (values: ProviderFormValues) => void;
  }) => (
    <form
      id="provider-form"
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit(mockFormValues);
      }}
    />
  ),
}));

describe("AddProviderDialog", () => {
  beforeEach(() => {
    universalMocks.upsert.mockReset().mockResolvedValue(true);
    universalMocks.sync.mockReset().mockResolvedValue(true);
    universalMocks.warning.mockReset();
    saveUniversal = undefined;
    mockFormValues = {
      name: "Test Provider",
      websiteUrl: "https://provider.example.com",
      settingsConfig: JSON.stringify({ env: {}, config: {} }),
      meta: {
        custom_endpoints: {
          "https://api.new-endpoint.com": {
            url: "https://api.new-endpoint.com",
            addedAt: 1,
          },
        },
      },
    };
  });

  it("新增统一供应商后自动同步，失败仅警告且关闭", async () => {
    universalMocks.sync.mockRejectedValueOnce(new Error("sync failed"));
    const handleOpenChange = vi.fn();
    render(
      <AddProviderDialog
        open
        onOpenChange={handleOpenChange}
        appId="claude"
        onSubmit={vi.fn()}
      />,
    );

    saveUniversal?.({
      id: "universal-1",
      name: "Universal",
      providerType: "custom",
      baseUrl: "https://example.com",
      apiKey: "key",
      models: {},
      apps: { claude: true, codex: true, gemini: true },
    });

    await waitFor(() =>
      expect(universalMocks.sync).toHaveBeenCalledWith("universal-1"),
    );
    expect(universalMocks.upsert).toHaveBeenCalledOnce();
    expect(universalMocks.warning).toHaveBeenCalledOnce();
    expect(handleOpenChange).toHaveBeenCalledWith(false);
  });

  it("使用 ProviderForm 返回的自定义端点", async () => {
    const handleSubmit = vi.fn().mockResolvedValue(undefined);
    const handleOpenChange = vi.fn();

    render(
      <AddProviderDialog
        open
        onOpenChange={handleOpenChange}
        appId="claude"
        onSubmit={handleSubmit}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "common.add",
      }),
    );

    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));

    const submitted = handleSubmit.mock.calls[0][0];
    expect(submitted.meta?.custom_endpoints).toEqual(
      mockFormValues.meta?.custom_endpoints,
    );
    expect(handleOpenChange).toHaveBeenCalledWith(false);
  });

  it("在缺少自定义端点时回退到配置中的 baseUrl", async () => {
    const handleSubmit = vi.fn().mockResolvedValue(undefined);

    mockFormValues = {
      name: "Base URL Provider",
      websiteUrl: "",
      settingsConfig: JSON.stringify({
        env: { ANTHROPIC_BASE_URL: "https://claude.base" },
        config: {},
      }),
    };

    render(
      <AddProviderDialog
        open
        onOpenChange={vi.fn()}
        appId="claude"
        onSubmit={handleSubmit}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "common.add",
      }),
    );

    await waitFor(() => expect(handleSubmit).toHaveBeenCalledTimes(1));

    const submitted = handleSubmit.mock.calls[0][0];
    expect(submitted.meta?.custom_endpoints).toEqual({
      "https://claude.base": {
        url: "https://claude.base",
        addedAt: expect.any(Number),
        lastUsed: undefined,
      },
    });
  });
});
