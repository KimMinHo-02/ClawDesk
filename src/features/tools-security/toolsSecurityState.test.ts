import { describe, expect, it } from "vitest";
import type { SecurityProfile, ToolPolicy } from "../../lib/tauri";
import {
  auditCategory,
  auditReducer,
  initialAuditState,
  initialPolicyDraft,
  initialPolicyMutationState,
  initialProfileActionState,
  initialProfileForm,
  isValidProfileId,
  isValidProfileName,
  isValidToolEntry,
  mapToolsSecurityError,
  policyDraftDirty,
  policyDraftReducer,
  policyMutationReducer,
  profileActionReducer,
  profileFormErrors,
  profileFormReducer,
  severityKey,
  splitEntries,
  TOOLS_SECURITY_ERROR_CODES,
  type PolicyMutationState,
} from "./toolsSecurityState";

const emptyPolicy: ToolPolicy = {
  profile: null,
  allow: [],
  deny: [],
  execMode: null,
  elevatedEnabled: null,
  fsWorkspaceOnly: null,
};

const sampleBuiltins: SecurityProfile[] = [
  {
    id: "default",
    name: "기본",
    baseProfile: "coding",
    allow: [],
    deny: [],
    execMode: "full",
  },
  {
    id: "hardened",
    name: "보안 강화",
    baseProfile: "messaging",
    allow: [],
    deny: ["group:automation", "group:runtime", "group:fs", "sessions_spawn", "sessions_send"],
    execMode: "deny",
  },
];

describe("isValidToolEntry", () => {
  it("accepts tool ids, group refs, and wildcard patterns", () => {
    expect(isValidToolEntry("web_search")).toBe(true);
    expect(isValidToolEntry("session_status")).toBe(true);
    expect(isValidToolEntry("group:fs")).toBe(true);
    expect(isValidToolEntry("group:automation")).toBe(true);
    expect(isValidToolEntry("image*")).toBe(true);
    expect(isValidToolEntry("outlook__*")).toBe(true);
    expect(isValidToolEntry("a-b_c.d")).toBe(true);
  });

  it("rejects empty, overlong, traversal, whitespace, and bad groups", () => {
    expect(isValidToolEntry("")).toBe(false);
    expect(isValidToolEntry("x".repeat(129))).toBe(false);
    expect(isValidToolEntry("../evil")).toBe(false);
    expect(isValidToolEntry("a/b")).toBe(false);
    expect(isValidToolEntry("a b")).toBe(false);
    expect(isValidToolEntry("a\tb")).toBe(false);
    expect(isValidToolEntry("group:")).toBe(false);
    expect(isValidToolEntry(`group:${"x".repeat(33)}`)).toBe(false);
    expect(isValidToolEntry("group:bad/slash")).toBe(false);
    expect(isValidToolEntry("a*b?")).toBe(false);
  });
});

describe("policyDraftReducer", () => {
  it("adds a valid entry and clears the input", () => {
    const withInput = policyDraftReducer(initialPolicyDraft, {
      type: "set-allow-input",
      value: "web_search",
    });
    const next = policyDraftReducer(withInput, { type: "add-allow" });
    expect(next.allow).toEqual(["web_search"]);
    expect(next.allowInput).toBe("");
  });

  it("trims the input before adding", () => {
    const withInput = policyDraftReducer(initialPolicyDraft, {
      type: "set-deny-input",
      value: "  group:fs  ",
    });
    const next = policyDraftReducer(withInput, { type: "add-deny" });
    expect(next.deny).toEqual(["group:fs"]);
  });

  it("keeps the input on an invalid entry (no silent drop)", () => {
    const withInput = policyDraftReducer(initialPolicyDraft, {
      type: "set-allow-input",
      value: "../evil",
    });
    const next = policyDraftReducer(withInput, { type: "add-allow" });
    expect(next.allow).toEqual([]);
    expect(next.allowInput).toBe("../evil");
  });

  it("keeps the input on a duplicate entry", () => {
    const withEntry = policyDraftReducer(initialPolicyDraft, {
      type: "load",
      allow: ["web_search"],
      deny: [],
    });
    const withInput = policyDraftReducer(withEntry, {
      type: "set-allow-input",
      value: "web_search",
    });
    const next = policyDraftReducer(withInput, { type: "add-allow" });
    expect(next.allow).toEqual(["web_search"]);
    expect(next.allowInput).toBe("web_search");
  });

  it("removes entries and keeps in-progress inputs across a load", () => {
    let state = policyDraftReducer(initialPolicyDraft, {
      type: "load",
      allow: ["web_search", "image*"],
      deny: ["group:fs"],
    });
    state = policyDraftReducer(state, { type: "set-allow-input", value: "new_tool" });
    state = policyDraftReducer(state, { type: "remove-allow", entry: "image*" });
    expect(state.allow).toEqual(["web_search"]);
    const reloaded = policyDraftReducer(state, {
      type: "load",
      allow: ["web_search"],
      deny: [],
    });
    expect(reloaded.allow).toEqual(["web_search"]);
    expect(reloaded.deny).toEqual([]);
    expect(reloaded.allowInput).toBe("new_tool"); // in-progress input survives
  });
});

describe("policyDraftDirty", () => {
  it("is false when the draft matches the policy", () => {
    const draft = policyDraftReducer(initialPolicyDraft, {
      type: "load",
      allow: ["web_search"],
      deny: [],
    });
    const policy: ToolPolicy = { ...emptyPolicy, allow: ["web_search"] };
    expect(policyDraftDirty(draft, policy)).toBe(false);
  });

  it("is true on any add or remove", () => {
    const draft = policyDraftReducer(initialPolicyDraft, {
      type: "load",
      allow: ["web_search"],
      deny: [],
    });
    const policy: ToolPolicy = { ...emptyPolicy, allow: ["web_search"] };
    expect(policyDraftDirty(policyDraftReducer(draft, { type: "remove-allow", entry: "web_search" }), policy)).toBe(true);
    const withInput = policyDraftReducer(draft, { type: "set-allow-input", value: "more" });
    expect(policyDraftDirty(policyDraftReducer(withInput, { type: "add-allow" }), policy)).toBe(true);
  });
});

describe("policyMutationReducer", () => {
  it("starts a mutation from idle", () => {
    const next = policyMutationReducer(initialPolicyMutationState, {
      type: "start",
      kind: "profile",
    });
    expect(next.pending).toBe("profile");
    expect(next.reloadCounter).toBe(0);
  });

  it("ignores a duplicate start while a mutation is pending", () => {
    const pending: PolicyMutationState = {
      pending: "profile",
      error: null,
      reloadCounter: 0,
    };
    const next = policyMutationReducer(pending, { type: "start", kind: "exec-mode" });
    expect(next).toBe(pending); // duplicate guard: state unchanged
  });

  it("bumps the re-query counter on finish (success and failure)", () => {
    const pending: PolicyMutationState = {
      pending: "allow",
      error: null,
      reloadCounter: 3,
    };
    const success = policyMutationReducer(pending, { type: "finish", error: null });
    const failure = policyMutationReducer(pending, { type: "finish", error: "실패" });
    expect(success.reloadCounter).toBe(4);
    expect(failure.reloadCounter).toBe(4);
    expect(failure.error).toBe("실패");
  });
});

describe("isValidProfileId", () => {
  it("accepts normal slugs", () => {
    expect(isValidProfileId("a")).toBe(true);
    expect(isValidProfileId("my-profile_1")).toBe(true);
    expect(isValidProfileId(`a${"b".repeat(63)}`)).toBe(true);
  });

  it("rejects bad shapes", () => {
    expect(isValidProfileId("")).toBe(false);
    expect(isValidProfileId("Bad-ID")).toBe(false);
    expect(isValidProfileId("1abc")).toBe(false);
    expect(isValidProfileId("a/b")).toBe(false);
    expect(isValidProfileId(`a${"b".repeat(64)}`)).toBe(false);
  });
});

describe("isValidProfileName", () => {
  it("accepts display names (Korean included)", () => {
    expect(isValidProfileName("내 프로필")).toBe(true);
    expect(isValidProfileName("a".repeat(50))).toBe(true);
  });

  it("rejects empty, overlong, and control characters", () => {
    expect(isValidProfileName("")).toBe(false);
    expect(isValidProfileName("a".repeat(51))).toBe(false);
    expect(isValidProfileName("a\u0000b")).toBe(false);
    expect(isValidProfileName("a\u007fb")).toBe(false);
    expect(isValidProfileName("a b")).toBe(true); // whitespace is allowed (display-only)
  });
});

describe("profileFormReducer", () => {
  it("open-create prefills from the current policy (unset ≡ full)", () => {
    const next = profileFormReducer(initialProfileForm, {
      type: "open-create",
      policy: { ...emptyPolicy, allow: ["web_search"], execMode: "ask" },
      builtins: sampleBuiltins,
    });
    expect(next.mode).toBe("create");
    expect(next.source).toBe("current");
    expect(next.baseProfile).toBe("full"); // unset → full
    expect(next.execMode).toBe("ask");
    expect(next.allow).toEqual(["web_search"]);
    expect(next.id).toBe("");
  });

  it("open-edit prefills from the user profile", () => {
    const profile: SecurityProfile = {
      id: "my-profile",
      name: "내 프로필",
      baseProfile: "messaging",
      allow: [],
      deny: ["group:automation"],
      execMode: "deny",
    };
    const next = profileFormReducer(initialProfileForm, { type: "open-edit", profile });
    expect(next.mode).toBe("edit");
    expect(next.id).toBe("my-profile");
    expect(next.name).toBe("내 프로필");
    expect(next.baseProfile).toBe("messaging");
    expect(next.deny).toEqual(["group:automation"]);
    expect(next.execMode).toBe("deny");
  });

  it("set-source re-prefills from a builtin (id/name kept)", () => {
    let form = profileFormReducer(initialProfileForm, {
      type: "open-create",
      policy: emptyPolicy,
      builtins: sampleBuiltins,
    });
    form = profileFormReducer(form, { type: "set-id", value: "copy-hardened" });
    form = profileFormReducer(form, { type: "set-name", value: "경화 복사" });
    form = profileFormReducer(form, {
      type: "set-source",
      source: "builtin-hardened",
      policy: emptyPolicy,
      builtins: sampleBuiltins,
    });
    expect(form.id).toBe("copy-hardened"); // kept
    expect(form.name).toBe("경화 복사"); // kept
    expect(form.baseProfile).toBe("messaging");
    expect(form.execMode).toBe("deny");
    expect(form.deny).toEqual([
      "group:automation",
      "group:runtime",
      "group:fs",
      "sessions_spawn",
      "sessions_send",
    ]);
  });

  it("set-source is ignored in edit mode", () => {
    const profile: SecurityProfile = {
      id: "my-profile",
      name: "x",
      baseProfile: "coding",
      allow: [],
      deny: [],
      execMode: "full",
    };
    const form = profileFormReducer(initialProfileForm, { type: "open-edit", profile });
    const next = profileFormReducer(form, {
      type: "set-source",
      source: "builtin-hardened",
      policy: emptyPolicy,
      builtins: sampleBuiltins,
    });
    expect(next).toBe(form);
  });

  it("splits comma-separated entry text (trim, drop empties, de-duplicate)", () => {
    expect(splitEntries("a, b ,a,, c")).toEqual(["a", "b", "c"]);
    expect(splitEntries("  ")).toEqual([]);
  });

  it("close resets the form", () => {
    const form = profileFormReducer(initialProfileForm, {
      type: "open-create",
      policy: emptyPolicy,
      builtins: sampleBuiltins,
    });
    expect(profileFormReducer(form, { type: "close" })).toEqual(initialProfileForm);
  });
});

describe("profileFormErrors", () => {
  const valid: ReturnType<typeof profileFormReducer> = profileFormReducer(
    initialProfileForm,
    { type: "open-create", policy: emptyPolicy, builtins: sampleBuiltins },
  );

  it("passes a valid form", () => {
    const form = {
      ...valid,
      id: "my-profile",
      name: "내 프로필",
      allow: ["web_search"],
      deny: ["group:fs"],
    };
    expect(profileFormErrors(form)).toEqual([]);
  });

  it("flags every invalid field", () => {
    const form = {
      ...valid,
      id: "Bad-ID",
      name: "",
      baseProfile: "nope",
      execMode: "nope",
      allow: ["../evil"],
      deny: ["a b"],
    };
    expect(profileFormErrors(form)).toEqual([
      "id",
      "name",
      "baseProfile",
      "execMode",
      "allow",
      "deny",
    ]);
  });
});

describe("profileActionReducer", () => {
  it("starts one action from idle", () => {
    const next = profileActionReducer(initialProfileActionState, {
      type: "start",
      kind: "apply",
      id: "hardened",
    });
    expect(next.pending).toEqual({ kind: "apply", id: "hardened" });
  });

  it("ignores a second start while any action is pending", () => {
    const pending = profileActionReducer(initialProfileActionState, {
      type: "start",
      kind: "save",
      id: "a",
    });
    const next = profileActionReducer(pending, { type: "start", kind: "delete", id: "b" });
    expect(next).toBe(pending);
  });

  it("bumps the profile counter on every finish; apply also bumps the policy counter", () => {
    const apply = profileActionReducer(initialProfileActionState, {
      type: "start",
      kind: "apply",
      id: "a",
    });
    const applied = profileActionReducer(apply, { type: "finish", kind: "apply", error: null });
    expect(applied.reloadCounter).toBe(1);
    expect(applied.policyReloadCounter).toBe(1);

    const save = profileActionReducer(applied, { type: "start", kind: "save", id: "b" });
    const saved = profileActionReducer(save, { type: "finish", kind: "save", error: null });
    expect(saved.reloadCounter).toBe(2);
    expect(saved.policyReloadCounter).toBe(1); // save does not touch the policy

    const del = profileActionReducer(saved, { type: "start", kind: "delete", id: "c" });
    const deleted = profileActionReducer(del, {
      type: "finish",
      kind: "delete",
      error: "실패",
    });
    expect(deleted.reloadCounter).toBe(3); // failure still re-queries
    expect(deleted.policyReloadCounter).toBe(1);
    expect(deleted.error).toBe("실패");
  });
});

describe("auditReducer", () => {
  it("starts from idle and ignores a duplicate start while running", () => {
    const running = auditReducer(initialAuditState, { type: "start" });
    expect(running.running).toBe(true);
    const next = auditReducer(running, { type: "start" });
    expect(next).toBe(running); // duplicate-run guard
  });

  it("stores the result on success", () => {
    const running = auditReducer(initialAuditState, { type: "start" });
    const result = { summary: { total: 0 }, findings: [], suppressedCount: 0 };
    const next = auditReducer(running, { type: "finish", result, error: null });
    expect(next.running).toBe(false);
    expect(next.result).toBe(result);
    expect(next.error).toBeNull();
  });

  it("clears the previous result on failure (fail-closed)", () => {
    const running = auditReducer(initialAuditState, { type: "start" });
    const ok = auditReducer(running, {
      type: "finish",
      result: { summary: {}, findings: [], suppressedCount: 0 },
      error: null,
    });
    const second = auditReducer(ok, { type: "start" });
    const failed = auditReducer(second, { type: "finish", result: null, error: "감사 실패" });
    expect(failed.result).toBeNull(); // no stale "clean" state
    expect(failed.error).toBe("감사 실패");
    expect(failed.running).toBe(false);
  });
});

describe("auditCategory / severityKey", () => {
  it("maps known checkId prefixes to categories", () => {
    expect(auditCategory("fs.config.perms_world_readable")).toBe("fs");
    expect(auditCategory("gateway.exposure.open")).toBe("gateway");
    expect(auditCategory("tools.exec.security_full_configured")).toBe("tools");
    expect(auditCategory("plugins.allowlist.missing")).toBe("plugins");
    expect(auditCategory("skills.load.failed")).toBe("skills");
    expect(auditCategory("channels.discord.token")).toBe("channels");
    expect(auditCategory("sandbox.docker.missing")).toBe("sandbox");
    expect(auditCategory("browser.profile.share")).toBe("browser");
    expect(auditCategory("hooks.script.path")).toBe("hooks");
    expect(auditCategory("security.installPolicy.lax")).toBe("security");
  });

  it("falls back to unknown for unmapped prefixes", () => {
    expect(auditCategory("agents.defaults.something")).toBe("unknown");
    expect(auditCategory("nope")).toBe("unknown");
  });

  it("maps severities (unknown values → unknown, raw kept by the caller)", () => {
    expect(severityKey("critical")).toBe("critical");
    expect(severityKey("warn")).toBe("warn");
    expect(severityKey("info")).toBe("info");
    expect(severityKey("unknown-level")).toBe("unknown");
    expect(severityKey(null)).toBe("unknown");
    expect(severityKey(undefined)).toBe("unknown");
  });
});

describe("mapToolsSecurityError", () => {
  it("passes every known stable code through", () => {
    for (const code of TOOLS_SECURITY_ERROR_CODES) {
      expect(mapToolsSecurityError({ code, message: "x" })).toBe(code);
    }
  });

  it("falls back for unknown codes", () => {
    expect(mapToolsSecurityError({ code: "something-else", message: "x" })).toBe("fallback");
  });
});
