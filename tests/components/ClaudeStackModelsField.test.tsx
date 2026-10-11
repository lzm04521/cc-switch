import { screen } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { useForm } from "react-hook-form";
import { describe, expect, it, vi } from "vitest";
import {
  ClaudeStackModelsField,
  createClaudeStackModelRow,
} from "@/components/providers/forms/ClaudeStackModelsField";
import { Form } from "@/components/ui/form";
import { renderWithQueryClient } from "../utils/testQueryClient";

// fork：聚合只读预览列与后端 stack::encode / stack::display_name 必须一致
// （id 无 claude 段、显示名英文括号无空格、简称回退在 ProviderForm 侧拼好传入）。

const FormShell = ({ children }: PropsWithChildren) => {
  const form = useForm();
  return <Form {...form}>{children}</Form>;
};

const renderField = (
  props: Partial<
    React.ComponentProps<typeof ClaudeStackModelsField>
  > = {},
) =>
  renderWithQueryClient(
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
