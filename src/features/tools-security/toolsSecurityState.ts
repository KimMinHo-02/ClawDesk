/**
 * Pure logic for the Phase 5 tools/security feature (policy draft, entry
 * validation, profile actions, audit run, error mapping). No Tauri calls
 * here — keep it unit-testable.
 *
 * Contracts (no optimistic updates):
 * - Tool policy: every finished mutation (success OR failure) bumps
 *   `reloadCounter` → the component re-queries the actual policy.
 * - Entry editors: add/remove of allow/deny chips with a frontend
 *   pre-validation mirroring the Rust validator (S2 UX — the backend
 *   re-validates; it is the security boundary).
 * - Security profiles: one pending action at a time (save/apply/delete);
 *   every finished action bumps the profile re-query counter, and a
 *   finished apply additionally bumps the policy re-query counter.
 * - Audit: single-flight; failure is fail-closed (no stale result kept).
 */

import {
  EXEC_MODES,
  TOOL_PROFILES,
  type SecurityAuditResult,
  type SecurityProfile,
  type TauriAppError,
  type ToolPolicy,
} from "../../lib/tauri";

// --- tool policy draft (allow/deny chip editors) --------------------------------

/** The editable part of the tool policy (allow/deny entry chips). */
export interface PolicyDraftState {
  allow: string[];
  deny: string[];
  /** The entry being typed into the allow input. */
  allowInput: string;
  /** The entry being typed into the deny input. */
  denyInput: string;
}

export const initialPolicyDraft: PolicyDraftState = {
  allow: [],
  deny: [],
  allowInput: "",
  denyInput: "",
};

export type PolicyDraftAction =
  /** (Re)load the committed entries from a freshly read policy. */
  | { type: "load"; allow: string[]; deny: string[] }
  | { type: "set-allow-input"; value: string }
  | { type: "set-deny-input"; value: string }
  | { type: "add-allow" }
  | { type: "add-deny" }
  | { type: "remove-allow"; entry: string }
  | { type: "remove-deny"; entry: string };

/**
 * Draft reducer:
 * - `add-*` trims the input, requires a valid (non-duplicate) entry, and
 *   clears the input only on success (invalid input stays for correction);
 * - `load` replaces the committed entries but keeps in-progress inputs.
 */
export function policyDraftReducer(
  state: PolicyDraftState,
  action: PolicyDraftAction,
): PolicyDraftState {
  switch (action.type) {
    case "load":
      return { ...state, allow: action.allow, deny: action.deny };
    case "set-allow-input":
      return { ...state, allowInput: action.value };
    case "set-deny-input":
      return { ...state, denyInput: action.value };
    case "add-allow": {
      const entry = state.allowInput.trim();
      if (!isValidToolEntry(entry) || state.allow.includes(entry)) {
        return state;
      }
      return { ...state, allow: [...state.allow, entry], allowInput: "" };
    }
    case "add-deny": {
      const entry = state.denyInput.trim();
      if (!isValidToolEntry(entry) || state.deny.includes(entry)) {
        return state;
      }
      return { ...state, deny: [...state.deny, entry], denyInput: "" };
    }
    case "remove-allow":
      return { ...state, allow: state.allow.filter((e) => e !== action.entry) };
    case "remove-deny":
      return { ...state, deny: state.deny.filter((e) => e !== action.entry) };
  }
}

/** Whether the draft entries differ from the committed policy (save button). */
export function policyDraftDirty(draft: PolicyDraftState, policy: ToolPolicy): boolean {
  const key = (entries: string[]) => entries.join("\u0000");
  return key(draft.allow) !== key(policy.allow) || key(draft.deny) !== key(policy.deny);
}

// --- policy mutation (single pending write, re-query after) ---------------------

/** The four single-field policy mutations. */
export type PolicyMutationKind = "profile" | "exec-mode" | "allow" | "deny";

/** Mutation state: one pending write at a time (duplicate-submit guard). */
export interface PolicyMutationState {
  pending: PolicyMutationKind | null;
  /** Korean message of the last failed mutation (null = none). */
  error: string | null;
  /** Bumped on every finished mutation → triggers a policy re-query. */
  reloadCounter: number;
}

export const initialPolicyMutationState: PolicyMutationState = {
  pending: null,
  error: null,
  reloadCounter: 0,
};

export type PolicyMutationAction =
  | { type: "start"; kind: PolicyMutationKind }
  | { type: "finish"; error: string | null };

/**
 * Mutation reducer:
 * - `start` while a mutation is pending is ignored (duplicate-submit guard);
 * - `finish` always bumps `reloadCounter` (re-query on success AND failure).
 */
export function policyMutationReducer(
  state: PolicyMutationState,
  action: PolicyMutationAction,
): PolicyMutationState {
  switch (action.type) {
    case "start":
      if (state.pending !== null) {
        return state;
      }
      return { pending: action.kind, error: null, reloadCounter: state.reloadCounter };
    case "finish":
      return {
        pending: null,
        error: action.error,
        reloadCounter: state.reloadCounter + 1,
      };
  }
}

// --- tool entry validation (mirrors the Rust pre-check, S2 UX) -------------------

/**
 * Frontend pre-check for one allow/deny entry. Mirrors the Rust validator
 * (contract §1): non-empty, ≤128 chars, charset `[A-Za-z0-9_:.*-]`, no
 * whitespace/`/`/`..`; a `group:` prefix must be `group:[A-Za-z0-9-]{1,32}`.
 *
 * This is UX only — the Rust layer re-validates before any argv use.
 */
export function isValidToolEntry(entry: string): boolean {
  if (entry.length === 0 || entry.length > 128) {
    return false;
  }
  const groupRest = entry.startsWith("group:") ? entry.slice("group:".length) : null;
  if (groupRest !== null) {
    return (
      groupRest.length > 0 &&
      groupRest.length <= 32 &&
      /^[A-Za-z0-9-]+$/.test(groupRest)
    );
  }
  return /^[A-Za-z0-9_:.*-]+$/.test(entry) && !entry.includes("..");
}

// --- security profile form (create / edit) ----------------------------------------

/** Profile id validation (mirrors Rust `^[a-z][a-z0-9_-]{0,63}$`). */
export function isValidProfileId(id: string): boolean {
  return /^[a-z][a-z0-9_-]{0,63}$/.test(id);
}

/** Profile display-name validation (1–50 chars, no control characters). */
export function isValidProfileName(name: string): boolean {
  if (name.length === 0 || [...name].length > 50) {
    return false;
  }
  return ![...name].some((ch) => {
    const code = ch.codePointAt(0) ?? 0;
    return (code >= 0x00 && code <= 0x1f) || code === 0x7f;
  });
}

/** A profile being created or edited in the form. */
export interface ProfileFormState {
  /** null = the form is closed. */
  mode: "create" | "edit" | null;
  /** The source of the prefill in create mode. */
  source: "current" | "builtin-default" | "builtin-hardened";
  id: string;
  name: string;
  baseProfile: string;
  allow: string[];
  deny: string[];
  execMode: string;
}

export const initialProfileForm: ProfileFormState = {
  mode: null,
  source: "current",
  id: "",
  name: "",
  baseProfile: "coding",
  allow: [],
  deny: [],
  execMode: "full",
};

export type ProfileFormAction =
  | {
      type: "open-create";
      policy: ToolPolicy;
      builtins: SecurityProfile[];
    }
  | { type: "open-edit"; profile: SecurityProfile }
  | { type: "close" }
  | { type: "set-source"; source: ProfileFormState["source"]; policy: ToolPolicy; builtins: SecurityProfile[] }
  | { type: "set-id"; value: string }
  | { type: "set-name"; value: string }
  | { type: "set-base-profile"; value: string }
  | { type: "set-exec-mode"; value: string }
  | { type: "set-allow-text"; value: string }
  | { type: "set-deny-text"; value: string };

function prefillFromPolicy(policy: ToolPolicy): Pick<ProfileFormState, "baseProfile" | "allow" | "deny" | "execMode"> {
  return {
    baseProfile: policy.profile ?? "full",
    allow: [...policy.allow],
    deny: [...policy.deny],
    execMode: policy.execMode ?? "full",
  };
}

function builtinById(builtins: SecurityProfile[], id: string): SecurityProfile | undefined {
  return builtins.find((b) => b.id === id);
}

/**
 * Form reducer:
 * - `open-create` prefills from the current policy (source `current`);
 * - `open-edit` prefills from the user profile (id fixed);
 * - `set-source` re-prefills the four policy fields from the chosen source
 *   (the id/name fields are kept — they are not part of the source).
 */
export function profileFormReducer(
  state: ProfileFormState,
  action: ProfileFormAction,
): ProfileFormState {
  switch (action.type) {
    case "open-create":
      return {
        mode: "create",
        source: "current",
        id: "",
        name: "",
        ...prefillFromPolicy(action.policy),
      };
    case "open-edit":
      return {
        mode: "edit",
        source: state.source,
        id: action.profile.id,
        name: action.profile.name,
        baseProfile: action.profile.baseProfile,
        allow: [...action.profile.allow],
        deny: [...action.profile.deny],
        execMode: action.profile.execMode,
      };
    case "close":
      return { ...initialProfileForm };
    case "set-source": {
      if (state.mode !== "create") {
        return state;
      }
      const prefill =
        action.source === "current"
          ? prefillFromPolicy(action.policy)
          : (() => {
              const builtin = builtinById(
                action.builtins,
                action.source === "builtin-default" ? "default" : "hardened",
              );
              return builtin
                ? {
                    baseProfile: builtin.baseProfile,
                    allow: [...builtin.allow],
                    deny: [...builtin.deny],
                    execMode: builtin.execMode,
                  }
                : prefillFromPolicy(action.policy);
            })();
      return { ...state, source: action.source, ...prefill };
    }
    case "set-id":
      return { ...state, id: action.value };
    case "set-name":
      return { ...state, name: action.value };
    case "set-base-profile":
      return { ...state, baseProfile: action.value };
    case "set-exec-mode":
      return { ...state, execMode: action.value };
    case "set-allow-text":
      return { ...state, allow: splitEntries(action.value) };
    case "set-deny-text":
      return { ...state, deny: splitEntries(action.value) };
  }
}

/** Splits a comma-separated entry text (trims, drops empties, de-duplicates). */
export function splitEntries(text: string): string[] {
  const seen = new Set<string>();
  const entries: string[] = [];
  for (const part of text.split(",")) {
    const entry = part.trim();
    if (entry.length > 0 && !seen.has(entry)) {
      seen.add(entry);
      entries.push(entry);
    }
  }
  return entries;
}

/** Validates the whole form (frontend pre-check; the backend re-validates). */
export function profileFormErrors(form: ProfileFormState): string[] {
  const errors: string[] = [];
  if (!isValidProfileId(form.id)) {
    errors.push("id");
  }
  if (!isValidProfileName(form.name)) {
    errors.push("name");
  }
  if (!(TOOL_PROFILES as readonly string[]).includes(form.baseProfile)) {
    errors.push("baseProfile");
  }
  if (!(EXEC_MODES as readonly string[]).includes(form.execMode)) {
    errors.push("execMode");
  }
  for (const entry of form.allow) {
    if (!isValidToolEntry(entry)) {
      errors.push("allow");
      break;
    }
  }
  for (const entry of form.deny) {
    if (!isValidToolEntry(entry)) {
      errors.push("deny");
      break;
    }
  }
  return errors;
}

// --- security profile actions (save / apply / delete) ------------------------------

export type ProfileActionKind = "save" | "apply" | "delete";

/** One pending profile action at a time (duplicate guard). */
export interface ProfileActionState {
  pending: { kind: ProfileActionKind; id: string } | null;
  /** Korean message of the last failed action (null = none). */
  error: string | null;
  /** Bumped after every finished action → re-query the profile list. */
  reloadCounter: number;
  /** Bumped after a finished apply → re-query the tool policy too. */
  policyReloadCounter: number;
}

export const initialProfileActionState: ProfileActionState = {
  pending: null,
  error: null,
  reloadCounter: 0,
  policyReloadCounter: 0,
};

export type ProfileActionAction =
  | { type: "start"; kind: ProfileActionKind; id: string }
  | { type: "finish"; kind: ProfileActionKind; error: string | null };

/**
 * Action reducer:
 * - `start` while any action is pending is ignored (duplicate guard);
 * - `finish` always bumps the profile re-query counter; a finished apply
 *   additionally bumps the policy re-query counter (the config changed).
 */
export function profileActionReducer(
  state: ProfileActionState,
  action: ProfileActionAction,
): ProfileActionState {
  switch (action.type) {
    case "start":
      if (state.pending !== null) {
        return state;
      }
      return {
        ...state,
        pending: { kind: action.kind, id: action.id },
        error: null,
      };
    case "finish":
      return {
        pending: null,
        error: action.error,
        reloadCounter: state.reloadCounter + 1,
        policyReloadCounter:
          action.kind === "apply"
            ? state.policyReloadCounter + 1
            : state.policyReloadCounter,
      };
  }
}

// --- security audit (single-flight, fail-closed) ------------------------------------

export interface AuditState {
  running: boolean;
  /** Last successful audit result (null = none / cleared on failure). */
  result: SecurityAuditResult | null;
  /** Korean message of the last failed audit (null = none). */
  error: string | null;
}

export const initialAuditState: AuditState = {
  running: false,
  result: null,
  error: null,
};

export type AuditAction =
  | { type: "start" }
  | { type: "finish"; result: SecurityAuditResult | null; error: string | null };

/**
 * Audit reducer:
 * - `start` while running is ignored (duplicate-run guard);
 * - `finish` with an error is fail-closed: any previous result is cleared
 *   (the UI never keeps showing a possibly stale "clean" state).
 */
export function auditReducer(state: AuditState, action: AuditAction): AuditState {
  switch (action.type) {
    case "start":
      if (state.running) {
        return state;
      }
      return { running: true, result: null, error: null };
    case "finish":
      if (action.error !== null) {
        return { running: false, result: null, error: action.error };
      }
      return { running: false, result: action.result, error: null };
  }
}

// --- audit finding display -----------------------------------------------------------

/** Known checkId first-segment → Korean category (contract §5). */
export const AUDIT_CATEGORY_KEYS = [
  "fs",
  "gateway",
  "tools",
  "plugins",
  "skills",
  "channels",
  "sandbox",
  "browser",
  "hooks",
  "security",
] as const;
export type AuditCategoryKey = (typeof AUDIT_CATEGORY_KEYS)[number];

/** Maps a finding checkId to its known category (fail-soft → "unknown"). */
export function auditCategory(checkId: string): AuditCategoryKey | "unknown" {
  const head = checkId.split(".")[0] ?? "";
  return (AUDIT_CATEGORY_KEYS as readonly string[]).includes(head)
    ? (head as AuditCategoryKey)
    : "unknown";
}

/** Maps a finding severity to its badge key (unknown values → "unknown"). */
export function severityKey(
  severity: string | null | undefined,
): "critical" | "warn" | "info" | "unknown" {
  if (severity === "critical" || severity === "warn" || severity === "info") {
    return severity;
  }
  return "unknown";
}

// --- error mapping ---------------------------------------------------------------------

/** Error codes the tools/security feature can receive (stable, from Rust). */
export const TOOLS_SECURITY_ERROR_CODES = [
  "tool-profile-invalid",
  "tool-entry-invalid",
  "exec-mode-invalid",
  "security-profile-id-invalid",
  "security-profile-name-invalid",
  "security-profile-not-found",
  "security-profile-conflict",
  "security-profile-store-failed",
  "openclaw-security-audit-failed",
  "openclaw-config-read-failed",
  "openclaw-config-write-failed",
  "openclaw-config-invalid",
  "openclaw-not-found",
  "process-timeout",
  "process-failed",
] as const;

export type ToolsSecurityErrorCode = (typeof TOOLS_SECURITY_ERROR_CODES)[number];

/**
 * Maps an IPC `AppError` to an i18n key. Unknown codes fall back so the UI
 * can always show a message (the fallback message is generic Korean).
 */
export function mapToolsSecurityError(
  error: TauriAppError,
): ToolsSecurityErrorCode | "fallback" {
  return (TOOLS_SECURITY_ERROR_CODES as readonly string[]).includes(error.code)
    ? (error.code as ToolsSecurityErrorCode)
    : "fallback";
}
