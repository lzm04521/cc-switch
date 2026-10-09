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

// fork Task 8: 模型列表接口区块已随 routeModelsEndpoint 设置删除；
// 这里只覆盖聚合前缀输入本身
describe("RoutePrefixSettings 聚合前缀区块", () => {
  it("渲染标题与说明，不含模型列表开关", () => {
    render(<RoutePrefixSettings {...baseProps} />);
    expect(
      screen.getByText("聚合模型 id 前缀", { exact: false }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("switch")).toBeNull();
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
