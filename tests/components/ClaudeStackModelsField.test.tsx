import { fireEvent, screen, within } from "@testing-library/react";
import { renderWithQueryClient as render } from "../utils/testQueryClient";
import { useForm } from "react-hook-form";
import type { PropsWithChildren } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  ClaudeStackModelsField,
  createClaudeStackModelRow,
} from "@/components/providers/forms/ClaudeStackModelsField";
import { Form } from "@/components/ui/form";

const FormShell = ({ children }: PropsWithChildren) => {
  const form = useForm();
  return <Form {...form}>{children}</Form>;
};

// fork：聚合只读预览列与后端 stack::encode / stack::display_name 必须一致
// （id 无 claude 段、显示名英文括号无空格、简称回退在 ProviderForm 侧拼好传入）。
const renderField = (
  props: Partial<
    React.ComponentProps<typeof ClaudeStackModelsField>
  > = {},
) =>
  render(
    <FormShell>
      <ClaudeStackModelsField
        rows={[
          createClaudeStackModelRow({ model: "glm-5.2", displayName: "GLM" }),
        ]}
        onRowsChange={vi.fn()}
        fetchedModels={[]}
        onFetchModels={vi.fn()}
        isFetchingModels={false}
        previewKey="zhipu"
        providerName="智谱"
        {...props}
      />
    </FormShell>,
  );

describe("ClaudeStackModelsField 聚合只读预览列", () => {
  it("聚合 id 与 Codex 同构：前缀+key+模型，不带 claude 段", () => {
    renderField();
    expect(screen.getByTitle("ccs-zhipu.glm-5.2")).toBeInTheDocument();
  });

  it("显示名用英文括号且无空格", () => {
    renderField();
    expect(screen.getByTitle("GLM(智谱)")).toBeInTheDocument();
  });

  it("1M 标记保留在 id 末尾", () => {
    renderField({
      rows: [createClaudeStackModelRow({ model: "glm-5.2", oneM: true })],
    });
    expect(screen.getByTitle("ccs-zhipu.glm-5.2[1M]")).toBeInTheDocument();
  });

  it("行显示名缺省回退模型名", () => {
    renderField({
      rows: [createClaudeStackModelRow({ model: "glm-5.2" })],
      providerName: "",
    });
    expect(screen.getByTitle("glm-5.2()")).toBeInTheDocument();
  });
});

describe("ClaudeStackModelsField model picker", () => {  it("keeps checked rows untouched, removes unchecked ones and appends new picks", () => {
    const first = createClaudeStackModelRow({
      model: "glm-5.2[1m]",
      displayName: "GLM",
    });
    const second = createClaudeStackModelRow({ model: "kimi-k3" });
    const draft = createClaudeStackModelRow();
    const onRowsChange = vi.fn();
    render(
      <FormShell>
        <ClaudeStackModelsField
          rows={[first, second, draft]}
          onRowsChange={onRowsChange}
          fetchedModels={[
            { id: "glm-5.2", ownedBy: null },
            { id: "kimi-k3", ownedBy: null },
            { id: "minimax-m3", ownedBy: null },
          ]}
          onFetchModels={vi.fn()}
          isFetchingModels={false}
        />
      </FormShell>,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "providerForm.modelPicker.title" }),
    );
    const dialog = screen.getByRole("dialog");
    // 带 [1m] 的行按去掉标记后的 ID 认成已勾选
    expect(
      within(dialog).getByRole("checkbox", { name: "glm-5.2" }),
    ).toBeChecked();
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "kimi-k3" }));
    fireEvent.click(
      within(dialog).getByRole("checkbox", { name: "minimax-m3" }),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: "providerForm.modelPicker.apply",
      }),
    );

    expect(onRowsChange).toHaveBeenCalledTimes(1);
    const [rows] = onRowsChange.mock.calls[0];
    expect(rows.slice(0, 2)).toEqual([first, draft]);
    expect(rows).toHaveLength(3);
    expect(rows[2]).toMatchObject({ model: "minimax-m3" });
  });
});

// 勾选带 contextLength 的 FetchedModel 时，新行应保留 contextLength
describe("ClaudeStackModelsField contextLength 透传", () => {
  it("勾选有 contextLength 的模型，新行写入 contextLength", () => {
    const onRowsChange = vi.fn();
    render(
      <FormShell>
        <ClaudeStackModelsField
          rows={[]}
          onRowsChange={onRowsChange}
          fetchedModels={[
            { id: "glm-5.2", ownedBy: null, contextLength: 128_000 },
            { id: "kimi-k3", ownedBy: null },
          ]}
          onFetchModels={vi.fn()}
          isFetchingModels={false}
        />
      </FormShell>,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "providerForm.modelPicker.title" }),
    );
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "glm-5.2" }));
    fireEvent.click(within(dialog).getByRole("checkbox", { name: "kimi-k3" }));
    fireEvent.click(
      screen.getByRole("button", { name: "providerForm.modelPicker.apply" }),
    );

    expect(onRowsChange).toHaveBeenCalledTimes(1);
    const [rows] = onRowsChange.mock.calls[0];
    expect(rows).toHaveLength(2);
    // 有 contextLength 的模型应透传
    expect(rows[0]).toMatchObject({ model: "glm-5.2", contextLength: 128_000 });
    // 无 contextLength 的模型不带该字段
    expect(rows[1]).toMatchObject({ model: "kimi-k3" });
    expect(rows[1].contextLength).toBeUndefined();
  });
});

// fork：上游 4f08989a 删除的「设为默认模型」交互，fork 恢复
describe("ClaudeStackModelsField 设为默认模型", () => {
  it("第一行 ★ 标记默认且不可点，点其余行 ★ 把该行挪到首位", () => {
    const first = createClaudeStackModelRow({
      model: "glm-5.2",
      displayName: "GLM",
    });
    const second = createClaudeStackModelRow({ model: "kimi-k3" });
    const onRowsChange = vi.fn();
    render(
      <FormShell>
        <ClaudeStackModelsField
          rows={[first, second]}
          onRowsChange={onRowsChange}
          fetchedModels={[]}
          onFetchModels={vi.fn()}
          isFetchingModels={false}
        />
      </FormShell>,
    );

    const defaultStar = screen.getByRole("button", { name: "默认模型" });
    expect(defaultStar).toBeDisabled();
    expect(defaultStar).toHaveAttribute("aria-pressed", "true");

    fireEvent.click(
      screen.getByRole("button", { name: "设为默认模型" }),
    );
    expect(onRowsChange).toHaveBeenCalledTimes(1);
    const [rows] = onRowsChange.mock.calls[0];
    expect(rows).toHaveLength(2);
    expect(rows[0]).toBe(second);
    expect(rows[1]).toBe(first);
  });
});
