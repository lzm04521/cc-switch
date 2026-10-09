import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { RoutePrefixSettings } from "@/components/settings/RoutePrefixSettings";

const baseProps = {
  routePrefix: "ccs-",
  onAutoSave: vi.fn(async () => true),
};

afterEach(() => {
  vi.clearAllMocks();
});

// fork Task 8: routeModelsEndpoint 设置曾随旧体系删除，聚合三调整恢复为
// 「聚合模型列表内容」三选一（groups/models/both）
describe("RoutePrefixSettings 聚合前缀区块", () => {
  it("渲染标题与说明，粘性开关存在（fork 五项优化 D4）", () => {
    render(<RoutePrefixSettings {...baseProps} />);
    expect(
      screen.getByText("聚合模型 id 前缀", { exact: false }),
    ).toBeInTheDocument();
    expect(screen.getByRole("switch")).toBeInTheDocument();
  });

  it("改前缀保存：onAutoSave 收到新值并出现失效提示", async () => {
    render(<RoutePrefixSettings {...baseProps} />);
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "G." } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(baseProps.onAutoSave).toHaveBeenCalledWith({
        routePrefix: "G.",
      }),
    );
    expect(
      screen.getByText("前缀已变更", { exact: false }),
    ).toBeInTheDocument();
  });

  it("未修改时保存按钮禁用", () => {
    render(<RoutePrefixSettings {...baseProps} />);
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
  });
});

describe("RoutePrefixSettings 聚合模型列表三选一", () => {
  it("缺省渲染 both 选项", () => {
    render(<RoutePrefixSettings {...baseProps} />);
    expect(
      screen.getByText("分组 + 模型", { exact: false }),
    ).toBeInTheDocument();
  });

  it("选中 groups：onAutoSave 收到 routeModelsEndpoint 更新", async () => {
    render(<RoutePrefixSettings {...baseProps} />);
    fireEvent.click(screen.getByRole("combobox"));
    fireEvent.click(screen.getByText("仅分组（短形式，选中走默认模型）"));
    await waitFor(() =>
      expect(baseProps.onAutoSave).toHaveBeenCalledWith({
        routeModelsEndpoint: { mode: "groups" },
      }),
    );
  });

  it("保存失败不切换选中值", async () => {
    const onAutoSave = vi.fn(async () => false);
    render(<RoutePrefixSettings {...baseProps} onAutoSave={onAutoSave} />);
    fireEvent.click(screen.getByRole("combobox"));
    fireEvent.click(screen.getByText("仅模型（完整条目）"));
    await waitFor(() => expect(onAutoSave).toHaveBeenCalledTimes(1));
    // 失败回滚：仍显示 both
    expect(
      await screen.findByText("分组 + 模型", { exact: false }),
    ).toBeInTheDocument();
  });
});

describe("RoutePrefixSettings 粘性会话开关（fork 五项优化 D4）", () => {
  it("缺省渲染为开，说明含 subagent 示例", () => {
    render(<RoutePrefixSettings {...baseProps} />);
    const toggle = screen.getByRole("switch");
    expect(toggle).toHaveAttribute("data-state", "checked");
    expect(screen.getByText("subagent", { exact: false })).toBeInTheDocument();
  });

  it("routeStickySession=false 渲染为关", () => {
    render(<RoutePrefixSettings {...baseProps} routeStickySession={false} />);
    expect(screen.getByRole("switch")).toHaveAttribute(
      "data-state",
      "unchecked",
    );
  });

  it("切换调用 onAutoSave 保存", async () => {
    const onAutoSave = vi.fn(async () => true);
    render(<RoutePrefixSettings {...baseProps} onAutoSave={onAutoSave} />);
    fireEvent.click(screen.getByRole("switch"));
    await waitFor(() =>
      expect(onAutoSave).toHaveBeenCalledWith({ routeStickySession: false }),
    );
  });

  it("保存失败回滚为开", async () => {
    const failing = vi.fn(async () => false);
    render(
      <RoutePrefixSettings {...baseProps} onAutoSave={failing} />,
    );
    fireEvent.click(screen.getByRole("switch"));
    await waitFor(() => expect(failing).toHaveBeenCalledTimes(1));
    // 保存失败：回滚为开
    await waitFor(() =>
      expect(screen.getByRole("switch")).toHaveAttribute(
        "data-state",
        "checked",
      ),
    );
  });
});
