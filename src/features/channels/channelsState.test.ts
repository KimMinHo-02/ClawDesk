import { describe, expect, it } from "vitest";
import type { ChannelConfig } from "../../lib/tauri";
import {
  CHANNEL_IDS,
  dmAccessDraftDirty,
  dmAccessDraftFromConfig,
  dmAccessDraftReducer,
  groupPolicyDirty,
  initialChannelsOpState,
  initialPairingState,
  isDmAccessConsistent,
  isKnownDmPolicy,
  isKnownGroupPolicy,
  isValidAllowFromEntry,
  isValidPairingCode,
  mapChannelsError,
  pairingReducer,
  runtimeStateLabel,
  channelsOpReducer,
  tokenView,
} from "./channelsState";

const emptyConfig: ChannelConfig = {
  enabled: null,
  tokenState: "absent",
  dmPolicy: null,
  allowFrom: [],
  groupPolicy: null,
};

describe("CHANNEL_IDS", () => {
  it("manages discord and telegram only", () => {
    expect(CHANNEL_IDS).toEqual(["discord", "telegram"]);
  });
});

describe("isValidAllowFromEntry", () => {
  it("accepts * and numeric user ids of 1-32 digits", () => {
    expect(isValidAllowFromEntry("*")).toBe(true);
    expect(isValidAllowFromEntry("1")).toBe(true);
    expect(isValidAllowFromEntry("1234567890")).toBe(true);
    expect(isValidAllowFromEntry("9".repeat(32))).toBe(true);
  });

  it("rejects non-numeric, overlong, and empty entries", () => {
    expect(isValidAllowFromEntry("")).toBe(false);
    expect(isValidAllowFromEntry("user123")).toBe(false);
    expect(isValidAllowFromEntry("12 34")).toBe(false);
    expect(isValidAllowFromEntry("12a4")).toBe(false);
    expect(isValidAllowFromEntry("9".repeat(33))).toBe(false);
    expect(isValidAllowFromEntry("discord:123")).toBe(false);
  });
});

describe("isValidPairingCode", () => {
  it("accepts 4-64 chars of [A-Za-z0-9_-]", () => {
    expect(isValidPairingCode("abcd")).toBe(true);
    expect(isValidPairingCode("abcd1234")).toBe(true);
    expect(isValidPairingCode("a_b-c")).toBe(true);
    expect(isValidPairingCode("x".repeat(64))).toBe(true);
  });

  it("rejects short, overlong, and bad-character codes", () => {
    expect(isValidPairingCode("abc")).toBe(false);
    expect(isValidPairingCode("x".repeat(65))).toBe(false);
    expect(isValidPairingCode("")).toBe(false);
    expect(isValidPairingCode("ab cd")).toBe(false);
    expect(isValidPairingCode("ab/cd")).toBe(false);
  });
});

describe("isDmAccessConsistent", () => {
  it("requires at least one entry for allowlist", () => {
    expect(isDmAccessConsistent("allowlist", [])).toBe(false);
    expect(isDmAccessConsistent("allowlist", ["1234567890"])).toBe(true);
  });

  it("requires * for open", () => {
    expect(isDmAccessConsistent("open", [])).toBe(false);
    expect(isDmAccessConsistent("open", ["1234567890"])).toBe(false);
    expect(isDmAccessConsistent("open", ["*"])).toBe(true);
  });

  it("accepts pairing/disabled regardless of entries", () => {
    expect(isDmAccessConsistent("pairing", [])).toBe(true);
    expect(isDmAccessConsistent("pairing", ["*"])).toBe(true);
    expect(isDmAccessConsistent("disabled", ["1234567890"])).toBe(true);
  });
});

describe("channelsOpReducer", () => {
  it("guards duplicate starts and keeps the counter until finish", () => {
    const started = channelsOpReducer(initialChannelsOpState, {
      type: "start",
      kind: "token",
      channel: "discord",
    });
    expect(started.pending).toEqual({ kind: "token", channel: "discord" });
    // A second start while pending is ignored.
    expect(
      channelsOpReducer(started, { type: "start", kind: "connect", channel: "telegram" }),
    ).toBe(started);
  });

  it("clears pending, sets the error, and bumps the counter on finish", () => {
    const started = channelsOpReducer(initialChannelsOpState, {
      type: "start",
      kind: "dm-access",
      channel: "telegram",
    });
    const failed = channelsOpReducer(started, {
      type: "finish",
      error: "저장에 실패했습니다.",
    });
    expect(failed.pending).toBeNull();
    expect(failed.error).toBe("저장에 실패했습니다.");
    expect(failed.reloadCounter).toBe(1);
    const succeeded = channelsOpReducer(failed, {
      type: "start",
      kind: "group-policy",
      channel: "discord",
    });
    expect(succeeded.error).toBeNull();
    const done = channelsOpReducer(succeeded, { type: "finish", error: null });
    expect(done.reloadCounter).toBe(2);
    expect(done.error).toBeNull();
  });
});

describe("dmAccessDraft", () => {
  it("builds a draft with the pairing default for null policies", () => {
    const draft = dmAccessDraftFromConfig(emptyConfig);
    expect(draft.dmPolicy).toBe("pairing");
    expect(draft.allowFrom).toEqual([]);
    expect(draft.input).toBe("");
  });

  it("keeps known raw policy values from the config", () => {
    const draft = dmAccessDraftFromConfig({
      ...emptyConfig,
      dmPolicy: "allowlist",
      allowFrom: ["1234567890"],
    });
    expect(draft.dmPolicy).toBe("allowlist");
    expect(draft.allowFrom).toEqual(["1234567890"]);
  });

  it("adds valid unique entries and keeps invalid input for correction", () => {
    const draft = dmAccessDraftFromConfig(emptyConfig);
    const withInput = dmAccessDraftReducer(draft, { type: "set-input", value: "  1234567890  " });
    const added = dmAccessDraftReducer(withInput, { type: "add-entry" });
    expect(added.allowFrom).toEqual(["1234567890"]);
    expect(added.input).toBe("");
    // Duplicate is rejected.
    const dup = dmAccessDraftReducer(added, { type: "set-input", value: "1234567890" });
    expect(dmAccessDraftReducer(dup, { type: "add-entry" })).toBe(dup);
    // Invalid input stays in the field.
    const invalid = dmAccessDraftReducer(added, { type: "set-input", value: "not-a-number" });
    const rejected = dmAccessDraftReducer(invalid, { type: "add-entry" });
    expect(rejected).toBe(invalid);
  });

  it("removes entries and re-loads without losing the in-progress input", () => {
    let draft = dmAccessDraftReducer(dmAccessDraftFromConfig(emptyConfig), {
      type: "set-input",
      value: "999",
    });
    draft = dmAccessDraftReducer(draft, { type: "add-entry" });
    draft = dmAccessDraftReducer(draft, { type: "set-input", value: "888" });
    const reloaded = dmAccessDraftReducer(draft, {
      type: "load",
      config: { ...emptyConfig, dmPolicy: "open", allowFrom: ["*"] },
    });
    expect(reloaded.dmPolicy).toBe("open");
    expect(reloaded.allowFrom).toEqual(["*"]);
    expect(reloaded.input).toBe("");
    draft = dmAccessDraftReducer(draft, { type: "remove-entry", entry: "999" });
    expect(draft.allowFrom).toEqual([]);
  });

  it("computes dirty against the config (pairing/allowlist defaults)", () => {
    const draft = dmAccessDraftFromConfig(emptyConfig);
    expect(dmAccessDraftDirty(draft, emptyConfig)).toBe(false);
    const changed = dmAccessDraftReducer(draft, { type: "set-policy", value: "disabled" });
    expect(dmAccessDraftDirty(changed, emptyConfig)).toBe(true);
    const added = dmAccessDraftReducer(changed, { type: "set-input", value: "1" });
    const addedEntry = dmAccessDraftReducer(added, { type: "add-entry" });
    expect(dmAccessDraftDirty(addedEntry, emptyConfig)).toBe(true);
    // A config with the same values is not dirty.
    const same: ChannelConfig = { ...emptyConfig, dmPolicy: "disabled", allowFrom: ["1"] };
    expect(dmAccessDraftDirty(addedEntry, same)).toBe(false);
  });
});

describe("groupPolicyDirty", () => {
  it("uses the allowlist default for null config values", () => {
    expect(groupPolicyDirty("allowlist", emptyConfig)).toBe(false);
    expect(groupPolicyDirty("open", emptyConfig)).toBe(true);
    const withValue: ChannelConfig = { ...emptyConfig, groupPolicy: "disabled" };
    expect(groupPolicyDirty("disabled", withValue)).toBe(false);
    expect(groupPolicyDirty("open", withValue)).toBe(true);
  });
});

describe("pairingReducer", () => {
  it("guards duplicate loads", () => {
    const started = pairingReducer(initialPairingState, { type: "start" });
    expect(started.loading).toBe(true);
    expect(pairingReducer(started, { type: "start" })).toBe(started);
  });

  it("fails closed on error (clears previous rows)", () => {
    const started = pairingReducer(initialPairingState, { type: "start" });
    const loaded = pairingReducer(started, {
      type: "finish",
      requests: [{ code: "abcd1234", sender: "someone" }],
      error: null,
    });
    expect(loaded.loading).toBe(false);
    expect(loaded.requests).toEqual([{ code: "abcd1234", sender: "someone" }]);
    const startedAgain = pairingReducer(loaded, { type: "start" });
    expect(startedAgain.loading).toBe(true);
    const errored = pairingReducer(startedAgain, {
      type: "finish",
      requests: null,
      error: "조회에 실패했습니다.",
    });
    expect(errored.requests).toBeNull();
    expect(errored.error).toBe("조회에 실패했습니다.");
  });

  it("keeps empty result distinct from an error", () => {
    const started = pairingReducer(initialPairingState, { type: "start" });
    const empty = pairingReducer(started, { type: "finish", requests: [], error: null });
    expect(empty.requests).toEqual([]);
    expect(empty.error).toBeNull();
  });
});

describe("runtimeStateLabel", () => {
  it("maps known values and fails soft to unknown", () => {
    expect(runtimeStateLabel("connected")).toBe("connected");
    expect(runtimeStateLabel("some-future-state")).toBe("unknown");
    expect(runtimeStateLabel(null)).toBe("unknown");
  });
});

describe("tokenView", () => {
  it("returns the config token state", () => {
    expect(tokenView(emptyConfig)).toBe("absent");
    expect(tokenView({ ...emptyConfig, tokenState: "managed" })).toBe("managed");
    expect(tokenView({ ...emptyConfig, tokenState: "external" })).toBe("external");
  });
});

describe("mapChannelsError", () => {
  it("maps known stable codes to themselves", () => {
    expect(mapChannelsError({ code: "channel-token-not-found", message: "m" })).toBe(
      "channel-token-not-found",
    );
    expect(mapChannelsError({ code: "openclaw-pairing-failed", message: "m" })).toBe(
      "openclaw-pairing-failed",
    );
    expect(mapChannelsError({ code: "process-timeout", message: "m" })).toBe("process-timeout");
  });

  it("falls back for unknown codes", () => {
    expect(mapChannelsError({ code: "ipc-failed", message: "m" })).toBe("fallback");
    expect(mapChannelsError({ code: "something-new", message: "m" })).toBe("fallback");
  });
});

describe("enum guards", () => {
  it("accepts only the known policy values", () => {
    expect(isKnownDmPolicy("pairing")).toBe(true);
    expect(isKnownDmPolicy("allowlist")).toBe(true);
    expect(isKnownDmPolicy("open")).toBe(true);
    expect(isKnownDmPolicy("disabled")).toBe(true);
    expect(isKnownDmPolicy("future-value")).toBe(false);
    expect(isKnownDmPolicy(null)).toBe(false);

    expect(isKnownGroupPolicy("open")).toBe(true);
    expect(isKnownGroupPolicy("allowlist")).toBe(true);
    expect(isKnownGroupPolicy("disabled")).toBe(true);
    expect(isKnownGroupPolicy("pairing")).toBe(false);
    expect(isKnownGroupPolicy(null)).toBe(false);
  });
});
