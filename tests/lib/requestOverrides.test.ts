import { describe, expect, it } from "vitest";
import {
  buildLocalProxyRequestOverrides,
  isProtectedLocalProxyHeaderName,
  isValidBodyRewritePath,
  isValidHttpHeaderName,
  isValidHttpHeaderValue,
  parseBodyOverrideJson,
  parseBodyRewritesJson,
  parseHeaderOverrideJson,
  parseRequestOverrideJson,
} from "@/lib/requestOverrides";

describe("requestOverrides", () => {
  it("treats empty JSON fields as unset", () => {
    expect(buildLocalProxyRequestOverrides("", "   ")).toEqual({});
  });

  it("parses header and body override objects", () => {
    expect(
      buildLocalProxyRequestOverrides(
        '{ "X-Test": "ok" }',
        '{ "temperature": 0.2 }',
      ),
    ).toEqual({
      overrides: {
        headers: { "x-test": "ok" },
        body: { temperature: 0.2 },
      },
    });
  });

  it("rejects non-object body overrides", () => {
    expect(parseRequestOverrideJson("[]").error).toBeTruthy();
  });

  it("rejects non-string header values", () => {
    expect(parseHeaderOverrideJson('{ "X-Test": 1 }').error).toBeTruthy();
  });

  it("rejects invalid header names", () => {
    expect(isValidHttpHeaderName("X-Test")).toBe(true);
    expect(isValidHttpHeaderName("X Foo")).toBe(false);
    expect(isValidHttpHeaderName("Authorization:")).toBe(false);
    expect(parseHeaderOverrideJson('{ "X Foo": "bar" }').error).toBeTruthy();
  });

  it("rejects duplicate header names after case normalization", () => {
    expect(
      parseHeaderOverrideJson('{ "X-Foo": "a", "x-foo": "b" }').error,
    ).toBeTruthy();
  });

  it("rejects protected proxy-managed header names", () => {
    expect(isProtectedLocalProxyHeaderName("Content-Type")).toBe(true);
    expect(isProtectedLocalProxyHeaderName("authorization")).toBe(true);
    expect(isProtectedLocalProxyHeaderName("X-Test")).toBe(false);
    expect(
      parseHeaderOverrideJson('{ "content-type": "text/plain" }').error,
    ).toBeTruthy();
    expect(
      parseHeaderOverrideJson('{ "Authorization": "Bearer x" }').error,
    ).toBeTruthy();
  });

  it("matches backend header value control-character rule", () => {
    expect(isValidHttpHeaderValue("hello\tworld")).toBe(true);
    expect(isValidHttpHeaderValue("hello\nworld")).toBe(false);
  });

  it("rejects stream in body overrides", () => {
    expect(parseBodyOverrideJson('{ "stream": true }').error).toBeTruthy();
    expect(
      buildLocalProxyRequestOverrides("", '{ "stream": false }').error,
    ).toBeTruthy();
  });

  it("parses body rewrite rules", () => {
    expect(
      parseBodyRewritesJson(
        '{ "messages[*].role": { "system": "user" }, "thinking.type": { "adaptive": "enabled" } }',
      ),
    ).toEqual({
      rewrites: {
        "messages[*].role": { system: "user" },
        "thinking.type": { adaptive: "enabled" },
      },
    });
    expect(parseBodyRewritesJson("  ")).toEqual({});
  });

  it("validates body rewrite paths like the backend", () => {
    expect(isValidBodyRewritePath("messages[*].content[*].type")).toBe(true);
    expect(isValidBodyRewritePath("thinking.type")).toBe(true);
    expect(isValidBodyRewritePath("a[0].b")).toBe(false);
    expect(isValidBodyRewritePath("a..b")).toBe(false);
    expect(isValidBodyRewritePath("messages[")).toBe(false);
    expect(isValidBodyRewritePath("  ")).toBe(false);
    expect(
      parseBodyRewritesJson('{ "a[0].b": { "x": "y" } }').error,
    ).toBeTruthy();
  });

  it("rejects malformed body rewrite rules", () => {
    expect(parseBodyRewritesJson("[]").error).toBeTruthy();
    expect(parseBodyRewritesJson('{ "a.b": "user" }').error).toBeTruthy();
    expect(parseBodyRewritesJson('{ "a.b": ["x"] }').error).toBeTruthy();
    expect(parseBodyRewritesJson('{ "a.b": { "x": 1 } }').error).toBeTruthy();
    expect(
      parseBodyRewritesJson('{ "stream": { "true": "false" } }').error,
    ).toBeTruthy();
  });

  it("builds bodyRewrites into the overrides payload", () => {
    expect(
      buildLocalProxyRequestOverrides(
        "",
        "",
        '{ "messages[*].role": { "system": "user" } }',
      ),
    ).toEqual({
      overrides: { bodyRewrites: { "messages[*].role": { system: "user" } } },
    });
    expect(buildLocalProxyRequestOverrides("", "", "")).toEqual({});
    expect(buildLocalProxyRequestOverrides("", "", "{}")).toEqual({});
    expect(buildLocalProxyRequestOverrides("", "", "[]").error).toBeTruthy();
  });
});
