import { describe, expect, it } from "vitest";
import { getStrings } from "../../i18n/ko";
import type { EnvironmentReport, NodeDetection, OpenClawStatus } from "../../lib/tauri";
import {
  canInstall,
  errorToMessage,
  isNodeSupported,
  nodeStateOf,
  openclawStateOf,
} from "./setupState";

const installStrings = getStrings("install");

function report(
  node: NodeDetection,
  openclaw: OpenClawStatus,
): EnvironmentReport {
  return {
    windows_version: { major_version: 11, build: 26100, ubr: 0, product_name: null },
    architecture: "x64",
    node,
    openclaw,
  };
}

describe("isNodeSupported (mirror of the Rust Phase 2 policy)", () => {
  it.each([
    "22.22.3",
    "22.23.0",
    "22.99.0",
    "24.15",
    "24.15.0",
    "24.16.1",
    "25.9.0",
    "25.10.0",
    "26.0",
    "26.0.0",
    "26.1.2",
    "27.0.0",
    "22.22.3-beta.1",
  ])("supports %s", (version) => {
    expect(isNodeSupported(version)).toBe(true);
  });

  it.each([
    "22.22.2",
    "22.0.0",
    "22.22",
    "23.0.0",
    "23.11.0",
    "24.14.9",
    "25.8.9",
    "21.7.3",
    "20.11.1",
    "18.19.0",
    "22",
    "garbage",
    "",
    "v22.22.3",
  ])("rejects %s", (version) => {
    expect(isNodeSupported(version)).toBe(false);
  });
});

describe("nodeStateOf", () => {
  it("maps not-found", () => {
    expect(nodeStateOf({ status: "not-found" })).toEqual({ kind: "not-found" });
  });

  it("maps a supported version", () => {
    expect(nodeStateOf({ status: "found", version: "22.22.3" })).toEqual({
      kind: "supported",
      version: "22.22.3",
    });
  });

  it("maps an unsupported version (Node 23)", () => {
    expect(nodeStateOf({ status: "found", version: "23.0.0" })).toEqual({
      kind: "unsupported",
      version: "23.0.0",
    });
  });
});

describe("openclawStateOf", () => {
  it("maps not-found to not-installed", () => {
    expect(openclawStateOf({ status: "not-found" })).toEqual({ kind: "not-installed" });
  });

  it("maps detected with version", () => {
    expect(
      openclawStateOf({
        status: "detected",
        executable: "C:\\npm\\openclaw",
        version: "2026.7.1-2",
        gateway: null,
        update: "updated",
      }),
    ).toEqual({ kind: "installed", version: "2026.7.1-2" });
  });

  it("maps detected without a collected version", () => {
    expect(
      openclawStateOf({
        status: "detected",
        executable: "C:\\npm\\openclaw",
        version: null,
        gateway: null,
        update: "unknown",
      }),
    ).toEqual({ kind: "installed", version: null });
  });
});

describe("canInstall", () => {
  const supportedNode: NodeDetection = { status: "found", version: "22.22.3" };
  const notFoundNode: NodeDetection = { status: "not-found" };
  const unsupportedNode: NodeDetection = { status: "found", version: "23.0.0" };
  const notInstalled: OpenClawStatus = { status: "not-found" };
  const installed: OpenClawStatus = {
    status: "detected",
    executable: "C:\\npm\\openclaw",
    version: "2026.7.1-2",
    gateway: null,
    update: "updated",
  };

  it("allows install only when OpenClaw is missing and Node is supported", () => {
    expect(canInstall(report(supportedNode, notInstalled))).toBe(true);
    expect(canInstall(report(notFoundNode, notInstalled))).toBe(false);
    expect(canInstall(report(unsupportedNode, notInstalled))).toBe(false);
    expect(canInstall(report(supportedNode, installed))).toBe(false);
    expect(canInstall(report(notFoundNode, installed))).toBe(false);
  });
});

describe("errorToMessage (stable code → Korean user message)", () => {
  it.each(
    (Object.keys(installStrings.errors) as Array<keyof typeof installStrings.errors>).filter(
      (key) => key !== "fallback",
    ),
  )("maps %s to its dedicated message", (code) => {
    expect(errorToMessage({ code, message: "infra detail must not leak" })).toBe(
      installStrings.errors[code],
    );
  });

  it("falls back for an unknown code", () => {
    expect(errorToMessage({ code: "some-future-code", message: "x" })).toBe(
      installStrings.errors.fallback,
    );
  });

  it("falls back for non-AppError rejections", () => {
    expect(errorToMessage("raw string")).toBe(installStrings.errors.fallback);
    expect(errorToMessage(new Error("boom"))).toBe(installStrings.errors.fallback);
    expect(errorToMessage({ code: "node-not-found" })).toBe(
      installStrings.errors.fallback,
    );
    expect(errorToMessage(undefined)).toBe(installStrings.errors.fallback);
  });

  it("never includes infrastructure detail from the raw message", () => {
    const message = errorToMessage({ code: "openclaw-install-failed", message: "sk-****" });
    expect(message).not.toContain("sk-");
    expect(message).toBe(installStrings.errors["openclaw-install-failed"]);
  });
});
