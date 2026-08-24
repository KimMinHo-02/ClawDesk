import { beforeEach, describe, expect, it, vi } from "vitest";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

import {
  COMMANDS,
  detectEnvironment,
  getPluginRuntime,
  installOpenClaw,
  isTauriAppError,
  listPlugins,
  listSkills,
  normalizeAppError,
  setPluginEnabled,
  setSkillEnabled,
  type EnvironmentReport,
} from "./index";

const sampleReport: EnvironmentReport = {
  windows_version: { major_version: 11, build: 26100, ubr: 0, product_name: null },
  architecture: "x64",
  node: { status: "found", version: "22.22.3" },
  openclaw: { status: "not-found" },
};

describe("COMMANDS", () => {
  it("uses the kebab-case frontend command names (single source)", () => {
    expect(COMMANDS.detectEnvironment).toBe("detect-environment");
    expect(COMMANDS.installOpenClaw).toBe("install-openclaw");
    expect(COMMANDS.listSkills).toBe("list-skills");
    expect(COMMANDS.setSkillEnabled).toBe("set-skill-enabled");
    expect(COMMANDS.listPlugins).toBe("list-plugins");
    expect(COMMANDS.setPluginEnabled).toBe("set-plugin-enabled");
    expect(COMMANDS.getPluginRuntime).toBe("get-plugin-runtime");
  });
});

describe("detectEnvironment", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("invokes detect-environment and resolves the typed report", async () => {
    mockInvoke.mockResolvedValueOnce(sampleReport);
    await expect(detectEnvironment()).resolves.toBe(sampleReport);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("detect-environment");
  });
});

describe("installOpenClaw", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("invokes install-openclaw and resolves the typed result", async () => {
    const result = { status: "installed" as const, version: "2026.7.1-2" };
    mockInvoke.mockResolvedValueOnce(result);
    await expect(installOpenClaw()).resolves.toBe(result);
    expect(mockInvoke).toHaveBeenCalledWith("install-openclaw");
  });

  it("resolves the already-installed wire shape", async () => {
    const result = { status: "already-installed" as const, version: "2026.7.0" };
    mockInvoke.mockResolvedValueOnce(result);
    await expect(installOpenClaw()).resolves.toBe(result);
  });
});

describe("Phase 4 wrappers", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("listSkills invokes list-skills without arguments", async () => {
    const rows = [{ name: "weather", enabled: true, eligible: true }];
    mockInvoke.mockResolvedValueOnce(rows);
    await expect(listSkills()).resolves.toBe(rows);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith("list-skills");
  });

  it("setSkillEnabled invokes set-skill-enabled with camelCase arguments", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await expect(setSkillEnabled("weather", false)).resolves.toBeUndefined();
    expect(mockInvoke).toHaveBeenCalledWith("set-skill-enabled", {
      skillName: "weather",
      enabled: false,
    });
  });

  it("listPlugins invokes list-plugins without arguments", async () => {
    const rows = [{ id: "@openclaw/discord", enabled: true }];
    mockInvoke.mockResolvedValueOnce(rows);
    await expect(listPlugins()).resolves.toBe(rows);
    expect(mockInvoke).toHaveBeenCalledWith("list-plugins");
  });

  it("setPluginEnabled invokes set-plugin-enabled with camelCase arguments", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await expect(setPluginEnabled("@openclaw/discord", true)).resolves.toBeUndefined();
    expect(mockInvoke).toHaveBeenCalledWith("set-plugin-enabled", {
      pluginId: "@openclaw/discord",
      enabled: true,
    });
  });

  it("getPluginRuntime invokes get-plugin-runtime and resolves the wire shape", async () => {
    const runtime = {
      id: "@openclaw/discord",
      tools: ["discord_send"],
      hooks: [],
      services: ["discord"],
      cliCommands: [],
      gatewayMethods: ["discord.connect"],
      routes: [],
    };
    mockInvoke.mockResolvedValueOnce(runtime);
    await expect(getPluginRuntime("@openclaw/discord")).resolves.toBe(runtime);
    expect(mockInvoke).toHaveBeenCalledWith("get-plugin-runtime", {
      pluginId: "@openclaw/discord",
    });
  });
});

describe("isTauriAppError", () => {
  it("accepts a structured AppError payload", () => {
    expect(isTauriAppError({ code: "node-not-found", message: "detail" })).toBe(true);
  });

  it("rejects payloads without a string code/message", () => {
    expect(isTauriAppError({ code: 1, message: "detail" })).toBe(false);
    expect(isTauriAppError({ code: "node-not-found" })).toBe(false);
    expect(isTauriAppError({ message: "detail" })).toBe(false);
    expect(isTauriAppError("node-not-found")).toBe(false);
    expect(isTauriAppError(null)).toBe(false);
    expect(isTauriAppError(undefined)).toBe(false);
    expect(isTauriAppError(42)).toBe(false);
  });
});

describe("normalizeAppError", () => {
  it("passes a structured AppError through unchanged", () => {
    const err = { code: "process-timeout", message: "npm install timed out" };
    expect(normalizeAppError(err)).toBe(err);
  });

  it("maps an Error instance to ipc-failed", () => {
    expect(normalizeAppError(new Error("boom"))).toEqual({
      code: "ipc-failed",
      message: "boom",
    });
  });

  it("maps a string rejection to ipc-failed", () => {
    expect(normalizeAppError("raw ipc failure")).toEqual({
      code: "ipc-failed",
      message: "raw ipc failure",
    });
  });

  it("maps unknown payloads to ipc-failed with a generic message", () => {
    expect(normalizeAppError({ unexpected: true })).toEqual({
      code: "ipc-failed",
      message: "unknown error",
    });
  });
});
