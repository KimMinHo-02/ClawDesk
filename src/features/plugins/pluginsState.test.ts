import { describe, expect, it } from "vitest";
import type { PluginRuntime } from "../../lib/tauri";
import {
  PLUGINS_ERROR_CODES,
  initialPluginsRuntimeState,
  initialPluginsToggleState,
  mapPluginsError,
  pluginsRuntimeReducer,
  pluginsToggleReducer,
  type PluginsToggleState,
} from "./pluginsState";

const sampleRuntime: PluginRuntime = {
  id: "@openclaw/discord",
  tools: ["discord_send"],
  hooks: [],
  services: ["discord"],
  cliCommands: [],
  gatewayMethods: ["discord.connect"],
  routes: [],
  diagnostics: null,
};

describe("pluginsToggleReducer", () => {
  it("starts a toggle from idle", () => {
    const next = pluginsToggleReducer(initialPluginsToggleState, {
      type: "start",
      key: "@openclaw/discord",
    });
    expect(next.pending).toBe("@openclaw/discord");
    expect(next.reloadCounter).toBe(0);
  });

  it("ignores a duplicate start while a toggle is pending", () => {
    const pending: PluginsToggleState = {
      pending: "@openclaw/discord",
      error: null,
      reloadCounter: 0,
    };
    const next = pluginsToggleReducer(pending, { type: "start", key: "other" });
    expect(next).toBe(pending); // duplicate guard: state unchanged
  });

  it("bumps the re-query counter on finish (success and failure)", () => {
    const pending: PluginsToggleState = {
      pending: "a",
      error: null,
      reloadCounter: 1,
    };
    const success = pluginsToggleReducer(pending, { type: "finish", error: null });
    const failure = pluginsToggleReducer(pending, { type: "finish", error: "실패" });
    expect(success.reloadCounter).toBe(2);
    // failure also triggers a re-query (no optimistic state)
    expect(failure.reloadCounter).toBe(2);
  });
});

describe("pluginsRuntimeReducer", () => {
  it("starts idle: list loading never triggers an inspect", () => {
    // The initial state (what the list-load path sees) has no pending
    // inspect, and no list-load action exists — only explicit `request`.
    expect(initialPluginsRuntimeState.loadingId).toBeNull();
    expect(initialPluginsRuntimeState.requestedId).toBeNull();
    expect(initialPluginsRuntimeState.data).toBeNull();
  });

  it("requests an inspect on demand", () => {
    const next = pluginsRuntimeReducer(initialPluginsRuntimeState, {
      type: "request",
      id: "@openclaw/discord",
    });
    expect(next.requestedId).toBe("@openclaw/discord");
    expect(next.loadingId).toBe("@openclaw/discord");
    expect(next.error).toBeNull();
  });

  it("ignores a second request while one is loading", () => {
    const loading = pluginsRuntimeReducer(initialPluginsRuntimeState, {
      type: "request",
      id: "a",
    });
    const next = pluginsRuntimeReducer(loading, { type: "request", id: "b" });
    expect(next).toBe(loading); // duplicate guard: state unchanged
  });

  it("stores the runtime payload on success", () => {
    const loading = pluginsRuntimeReducer(initialPluginsRuntimeState, {
      type: "request",
      id: "@openclaw/discord",
    });
    const next = pluginsRuntimeReducer(loading, {
      type: "finish",
      id: "@openclaw/discord",
      error: null,
      data: sampleRuntime,
    });
    expect(next.loadingId).toBeNull();
    expect(next.data).toBe(sampleRuntime);
    expect(next.error).toBeNull();
  });

  it("clears previous data on failure (fail-closed: no stale 'loaded')", () => {
    const loading = pluginsRuntimeReducer(initialPluginsRuntimeState, {
      type: "request",
      id: "a",
    });
    const withData = pluginsRuntimeReducer(loading, {
      type: "finish",
      id: "a",
      error: null,
      data: sampleRuntime,
    });
    const secondLoading = pluginsRuntimeReducer(withData, { type: "request", id: "b" });
    const failed = pluginsRuntimeReducer(secondLoading, {
      type: "finish",
      id: "b",
      error: "실패",
      data: null,
    });
    expect(failed.data).toBeNull(); // stale runtime data must be cleared
    expect(failed.error).toBe("실패");
    expect(failed.loadingId).toBeNull();
  });
});

describe("mapPluginsError", () => {
  it("passes every known stable code through", () => {
    for (const code of PLUGINS_ERROR_CODES) {
      expect(mapPluginsError({ code, message: "x" })).toBe(code);
    }
  });

  it("falls back for unknown codes", () => {
    expect(mapPluginsError({ code: "something-else", message: "x" })).toBe("fallback");
  });
});
