import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { ImportCcsDialog } from "@/components/providers/ImportCcsDialog";
import { server } from "../msw/server";
import type {
  CcsDetectResponse,
  CcsImportItem,
  CcsSyncResponse,
} from "@/lib/api/providers";

// Dialog 组件 mock 成纯 div，规避 portal/焦点陷阱。
vi.mock("@/components/ui/dialog", () => ({
  Dialog: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  DialogContent: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogHeader: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  DialogTitle: ({ children }: { children: ReactNode }) => <h1>{children}</h1>,
  DialogDescription: ({ children }: { children: ReactNode }) => (
    <p>{children}</p>
  ),
  DialogFooter: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
}));

const TAURI_ENDPOINT = "http://tauri.local";

function renderWithQueryClient(ui: ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>,
  );
}

const detectResponse = (
  providers: CcsDetectResponse["providers"],
): CcsDetectResponse => ({
  configPath: "/home/mock/.cc-switch/cc-switch.db",
  source: "sqlite",
  found: true,
  providers,
});

function mockDetect(response: CcsDetectResponse) {
  server.use(
    http.post(`${TAURI_ENDPOINT}/detect_ccs_channels`, () =>
      HttpResponse.json(response),
    ),
  );
}

function mockSync(
  response: CcsSyncResponse,
  capture?: (items: CcsImportItem[]) => void,
) {
  server.use(
    http.post(`${TAURI_ENDPOINT}/sync_ccs_channels`, async ({ request }) => {
      const body = (await request.json()) as { items: CcsImportItem[] };
      capture?.(body.items);
      return HttpResponse.json(response);
    }),
  );
}

describe("ImportCcsDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders three states and disables unimportable items", async () => {
    mockDetect(
      detectResponse([
        {
          originalId: "a",
          name: "New Channel",
          baseUrl: "https://a.example.com",
          hasApiKey: true,
          importable: true,
          status: "new",
          importedName: "New Channel",
        },
        {
          originalId: "b",
          name: "Updated Channel",
          baseUrl: "https://b.example.com",
          hasApiKey: true,
          importable: true,
          status: "update",
          importedName: "Updated Channel",
          targetProviderId: "local-b",
        },
        {
          originalId: "c",
          name: "Official Login",
          hasApiKey: false,
          importable: false,
          status: "new",
          importedName: "Official Login",
          warning: "无 base_url",
        },
      ]),
    );

    renderWithQueryClient(<ImportCcsDialog open onOpenChange={() => {}} />);

    await waitFor(() =>
      expect(screen.getByText("New Channel")).toBeInTheDocument(),
    );

    const checkboxes = screen.getAllByRole("checkbox");
    expect(checkboxes).toHaveLength(3);
    // new + update 可导入 → 默认勾选；不可导入项禁用且不勾选。
    expect(checkboxes[0]).toHaveAttribute("data-state", "checked");
    expect(checkboxes[1]).toHaveAttribute("data-state", "checked");
    expect(checkboxes[2]).toBeDisabled();
    expect(checkboxes[2]).toHaveAttribute("data-state", "unchecked");
    // 不可导入原因可见。
    expect(screen.getByText("无 base_url")).toBeInTheDocument();
  });

  it("defaults unchanged items to unselected", async () => {
    mockDetect(
      detectResponse([
        {
          originalId: "u",
          name: "Unchanged Channel",
          baseUrl: "https://u.example.com",
          hasApiKey: true,
          importable: true,
          status: "unchanged",
          importedName: "Unchanged Channel",
          targetProviderId: "local-u",
        },
      ]),
    );

    renderWithQueryClient(<ImportCcsDialog open onOpenChange={() => {}} />);

    await waitFor(() =>
      expect(screen.getByText("Unchanged Channel")).toBeInTheDocument(),
    );

    const checkbox = screen.getByRole("checkbox");
    expect(checkbox).toHaveAttribute("data-state", "unchecked");
  });

  it("syncs selected items and reports success", async () => {
    mockDetect(
      detectResponse([
        {
          originalId: "a",
          name: "New Channel",
          baseUrl: "https://a.example.com",
          hasApiKey: true,
          importable: true,
          status: "new",
          importedName: "New Channel",
        },
      ]),
    );
    const captured: CcsImportItem[] = [];
    mockSync(
      {
        created: [{ originalId: "a", providerId: "p1", name: "New Channel" }],
        updated: [],
        skipped: [],
        errors: [],
      },
      (items) => captured.push(...items),
    );

    const onOpenChange = vi.fn();
    renderWithQueryClient(<ImportCcsDialog open onOpenChange={onOpenChange} />);

    await waitFor(() =>
      expect(screen.getByText("New Channel")).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByRole("button", { name: /同步所选|Sync/ }));

    await waitFor(() => expect(captured).toHaveLength(1));
    expect(captured[0]).toEqual({
      originalId: "a",
      importedName: "New Channel",
    });
    // 全成功 → 关闭对话框。
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  });

  it("shows not-found message when ccs is absent", async () => {
    mockDetect({
      configPath: "/home/mock/.cc-switch/config.json",
      source: "none",
      found: false,
      providers: [],
    });

    renderWithQueryClient(<ImportCcsDialog open onOpenChange={() => {}} />);

    await waitFor(() =>
      expect(
        screen.getByText("未检测到本机 cc-switch 数据"),
      ).toBeInTheDocument(),
    );
  });
});
