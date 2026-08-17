import { describe, expect, it } from "vitest";
import { isClickGesture } from "./utils";

describe("isClickGesture", () => {
  it("位移小于阈值判定为点击", () => {
    expect(isClickGesture({ x: 100, y: 100 }, { x: 103, y: 101 })).toBe(true);
  });

  it("位移超过阈值判定为拖拽", () => {
    expect(isClickGesture({ x: 100, y: 100 }, { x: 120, y: 100 })).toBe(false);
  });

  it("零位移判定为点击", () => {
    expect(isClickGesture({ x: 10, y: 20 }, { x: 10, y: 20 })).toBe(true);
  });
});
