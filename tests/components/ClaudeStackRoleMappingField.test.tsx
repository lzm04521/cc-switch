import { fireEvent, screen } from "@testing-library/react";
import { renderWithQueryClient as render } from "../utils/testQueryClient";
import { describe, expect, it, vi } from "vitest";
import {
  ClaudeStackRoleMappingField,
  type ClaudeStackRoleTiers,
} from "@/components/providers/forms/ClaudeStackRoleMappingField";
import { createClaudeStackModelRow } from "@/components/providers/forms/ClaudeStackModelsField";

vi.mock("@/components/ui/select", () => ({
  Select: ({ value, onValueChange, children }: any) => (
    <select
      data-testid="stack-role-select"
      value={value}
      onChange={(event: any) => onValueChange?.(event.target.value)}
    >
      {children}
    </select>
  ),
  SelectTrigger: ({ children }: any) => <>{children}</>,
  SelectValue: () => null,
  SelectContent: ({ children }: any) => <>{children}</>,
  SelectItem: ({ value, children }: any) => (
    <option value={value}>{children}</option>
  ),
}));

const rows = [
  createClaudeStackModelRow({ model: "glm-5.2", displayName: "GLM" }),
  createClaudeStackModelRow({ model: "glm-flash[1M]" }),
  createClaudeStackModelRow({ model: "   " }),
];

const values: ClaudeStackRoleTiers = {
  sonnet: "glm-5.2[1M]", // 带 1M：去标记后命中 glm-5.2
  opus: "not-in-list", // 未命中 → 跟随默认
  fable: "", // 未配 → 跟随默认
  haiku: " glm-flash ", // 首尾空格：trim 后命中第二行
};

const renderField = (
  props: Partial<React.ComponentProps<typeof ClaudeStackRoleMappingField>> = {},
) =>
  render(
    <ClaudeStackRoleMappingField
      rows={rows}
      values={values}
      onTierChange={vi.fn()}
      {...props}
    />,
  );

const selects = () =>
  screen.getAllByTestId("stack-role-select") as HTMLSelectElement[];

describe("ClaudeStackRoleMappingField 聚合默认分组模型映射", () => {
  it("渲染 Sonnet/Opus/Fable/Haiku 四行下拉", () => {
    renderField();
    for (const role of ["Sonnet", "Opus", "Fable", "Haiku"]) {
      expect(screen.getByText(role)).toBeInTheDocument();
    }
    expect(selects()).toHaveLength(4);
  });

  it("选项 = 跟随默认 + 列表模型行：空行剔除，label 用显示名", () => {
    renderField();
    const first = selects()[0];
    const options = Array.from(first.options).map((o) => o.value);
    expect(options).toEqual(["__follow_default__", "glm-5.2", "glm-flash"]);
    expect(first.options[1].textContent).toBe("GLM");
  });

  it("值去 1M/trim 后命中才选中；未配或未命中都是跟随默认", () => {
    renderField();
    const all = selects();
    expect(all[0].value).toBe("glm-5.2"); // sonnet
    expect(all[1].value).toBe("__follow_default__"); // opus 未命中
    expect(all[2].value).toBe("__follow_default__"); // fable 未配
    expect(all[3].value).toBe("glm-flash"); // haiku
  });

  it("选模型回调模型名；选跟随默认回调空串", () => {
    const onTierChange = vi.fn();
    renderField({ onTierChange });
    const all = selects();
    fireEvent.change(all[0], { target: { value: "glm-flash" } });
    expect(onTierChange).toHaveBeenCalledWith("sonnet", "glm-flash");
    fireEvent.change(all[2], { target: { value: "__follow_default__" } });
    expect(onTierChange).toHaveBeenCalledWith("fable", "");
  });
});
