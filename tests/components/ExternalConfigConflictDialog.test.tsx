import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ExternalConfigConflictDialog } from "@/components/proxy/ExternalConfigConflictDialog";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) =>
      typeof options?.defaultValue === "string" ? options.defaultValue : key,
  }),
}));

const acceptMutateAsyncMock = vi.fn();
const rejectMutateAsyncMock = vi.fn();

vi.mock("@/lib/query/proxy", () => ({
  useAcceptExternalConfigChange: () => ({
    mutateAsync: acceptMutateAsyncMock,
    isPending: false,
  }),
  useRejectExternalConfigChange: () => ({
    mutateAsync: rejectMutateAsyncMock,
    isPending: false,
  }),
}));

describe("ExternalConfigConflictDialog", () => {
  beforeEach(() => {
    acceptMutateAsyncMock.mockReset();
    rejectMutateAsyncMock.mockReset();
  });

  it("does not render when there is no conflict", () => {
    render(
      <ExternalConfigConflictDialog conflict={undefined} onResolved={vi.fn()} />,
    );
    expect(
      screen.queryByText("接受外部更改"),
    ).not.toBeInTheDocument();
  });

  it("accepts the change and resolves on success", async () => {
    acceptMutateAsyncMock.mockResolvedValueOnce(undefined);
    const onResolved = vi.fn();
    render(
      <ExternalConfigConflictDialog
        conflict={{ appType: "claude", generation: 3 }}
        onResolved={onResolved}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "接受外部更改" }));

    await waitFor(() =>
      expect(acceptMutateAsyncMock).toHaveBeenCalledWith({
        appType: "claude",
        generation: 3,
      }),
    );
    await waitFor(() => expect(onResolved).toHaveBeenCalledWith("claude"));
  });

  it("keeps the dialog open and shows the reason when accept fails", async () => {
    acceptMutateAsyncMock.mockRejectedValueOnce(
      new Error("无法可靠解析实际路由目标"),
    );
    const onResolved = vi.fn();
    render(
      <ExternalConfigConflictDialog
        conflict={{ appType: "codex", generation: 5 }}
        onResolved={onResolved}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "接受外部更改" }));

    await waitFor(() =>
      expect(screen.getByText("无法可靠解析实际路由目标")).toBeInTheDocument(),
    );
    expect(onResolved).not.toHaveBeenCalled();
  });

  it("rejects the change and resolves on success", async () => {
    rejectMutateAsyncMock.mockResolvedValueOnce(undefined);
    const onResolved = vi.fn();
    render(
      <ExternalConfigConflictDialog
        conflict={{ appType: "gemini", generation: 2 }}
        onResolved={onResolved}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "使用 Agent-Switch 配置覆盖" }),
    );

    await waitFor(() =>
      expect(rejectMutateAsyncMock).toHaveBeenCalledWith({
        appType: "gemini",
        generation: 2,
      }),
    );
    await waitFor(() => expect(onResolved).toHaveBeenCalledWith("gemini"));
  });
});
