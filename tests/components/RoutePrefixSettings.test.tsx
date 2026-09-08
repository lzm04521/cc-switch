import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { RoutePrefixSettings } from "@/components/settings/RoutePrefixSettings";

const baseProps = {
  routePrefix: "G.",
  onAutoSave: vi.fn(async () => true),
};

afterEach(() => {
  vi.clearAllMocks();
});

describe("RoutePrefixSettings 模型列表接口区块", () => {
  it("默认关闭：渲染开关，不渲染返回类型按钮", () => {
    render(<RoutePrefixSettings {...baseProps} />);
    expect(
      screen.getByText("/v1/models 模型列表", { exact: false }),
    ).toBeInTheDocument();
    expect(screen.queryByText("返回类型")).toBeNull();
  });

  it("点击开关：onAutoSave 收到 enabled=true 且保留当前 mode", async () => {
    render(
      <RoutePrefixSettings
        {...baseProps}
        routeModelsEndpoint={{ enabled: false, mode: "both" }}
      />,
    );
    // Switch 是唯一的 role=switch
    fireEvent.click(screen.getByRole("switch"));
    await waitFor(() =>
      expect(baseProps.onAutoSave).toHaveBeenCalledWith({
        routeModelsEndpoint: { enabled: true, mode: "both" },
      }),
    );
    // 开关保存成功只闪区块自身的反馈，不误闪前缀「保存」按钮
    expect(screen.queryByRole("button", { name: "已保存" })).toBeNull();
    expect(screen.getByRole("button", { name: "保存" })).toBeInTheDocument();
  });

  it("开启后点「模型」按钮：onAutoSave 收到 mode=models", async () => {
    render(
      <RoutePrefixSettings
        {...baseProps}
        routeModelsEndpoint={{ enabled: true, mode: "groups" }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "模型" }));
    await waitFor(() =>
      expect(baseProps.onAutoSave).toHaveBeenCalledWith({
        routeModelsEndpoint: { enabled: true, mode: "models" },
      }),
    );
  });
});
