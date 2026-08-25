/**
 * Pure logic for the Phase 6 channels feature (Discord / Telegram).
 * No Tauri calls here — keep it unit-testable.
 *
 * Contracts (no optimistic updates):
 * - One pending mutation at a time across the whole feature
 *   (duplicate-submit guard); every finished mutation (success OR failure)
 *   bumps `reloadCounter` → the component re-queries the actual state.
 * - Entry/policy editors: frontend pre-validation mirroring the Rust
 *   validators (S2 UX — the backend re-validates; it is the security
 *   boundary).
 * - The channel token value travels to Rust only (S7) — it is never kept
 *   in feature state after submit and is never displayed.
 */

import {
  DM_POLICIES,
  GROUP_POLICIES,
  type ChannelConfig,
  type TauriAppError,
} from "../../lib/tauri";

/** The channels ClawDesk manages (contract: Discord / Telegram only). */
export const CHANNEL_IDS = ["discord", "telegram"] as const;
export type ChannelId = (typeof CHANNEL_IDS)[number];

// --- entry / policy validation (mirrors the Rust pre-checks, S2 UX) ------------

/**
 * Frontend pre-check for one `allowFrom` entry. Mirrors the Rust validator
 * (contract §1): `*` or a numeric user id of 1–32 digits.
 *
 * This is UX only — the Rust layer re-validates before any argv use.
 */
export function isValidAllowFromEntry(entry: string): boolean {
  if (entry === "*") {
    return true;
  }
  return entry.length >= 1 && entry.length <= 32 && /^\d+$/.test(entry);
}

/**
 * Frontend pre-check for a pairing code. Mirrors the Rust validator
 * (contract §6): 4–64 chars of `[A-Za-z0-9_-]`.
 */
export function isValidPairingCode(code: string): boolean {
  return code.length >= 4 && code.length <= 64 && /^[A-Za-z0-9_-]+$/.test(code);
}

/**
 * DM-access cross-rule (mirrors Rust `validate_dm_access`): `allowlist`
 * requires at least one entry, `open` requires `*` to be present.
 */
export function isDmAccessConsistent(dmPolicy: string, allowFrom: string[]): boolean {
  if (dmPolicy === "allowlist" && allowFrom.length === 0) {
    return false;
  }
  if (dmPolicy === "open" && !allowFrom.includes("*")) {
    return false;
  }
  return true;
}

// --- global mutation state (single pending op, re-query after) ------------------

/** The channel mutations the feature can run (one at a time). */
export type ChannelsOpKind =
  | "connect"
  | "token"
  | "enabled"
  | "dm-access"
  | "group-policy"
  | "approve-pairing";

/** Mutation state: one pending operation at a time (duplicate-submit guard). */
export interface ChannelsOpState {
  pending: { kind: ChannelsOpKind; channel: string } | null;
  /** Korean message of the last failed mutation (null = none). */
  error: string | null;
  /** Bumped on every finished mutation → triggers a state re-query. */
  reloadCounter: number;
}

export const initialChannelsOpState: ChannelsOpState = {
  pending: null,
  error: null,
  reloadCounter: 0,
};

export type ChannelsOpAction =
  | { type: "start"; kind: ChannelsOpKind; channel: string }
  | { type: "finish"; error: string | null };

/**
 * Mutation reducer:
 * - `start` while an operation is pending is ignored (duplicate-submit guard);
 * - `finish` always bumps `reloadCounter` (re-query on success AND failure).
 */
export function channelsOpReducer(
  state: ChannelsOpState,
  action: ChannelsOpAction,
): ChannelsOpState {
  switch (action.type) {
    case "start":
      if (state.pending !== null) {
        return state;
      }
      return { pending: { kind: action.kind, channel: action.channel }, error: null, reloadCounter: state.reloadCounter };
    case "finish":
      return {
        pending: null,
        error: action.error,
        reloadCounter: state.reloadCounter + 1,
      };
  }
}

// --- DM access draft (policy select + allowFrom chip editor) ---------------------

/** The editable DM-access part of a channel config. */
export interface DmAccessDraft {
  dmPolicy: string;
  allowFrom: string[];
  /** The entry being typed into the allowFrom input. */
  input: string;
}

/** Builds a fresh draft from a redacted channel config (fail-soft values). */
export function dmAccessDraftFromConfig(config: ChannelConfig): DmAccessDraft {
  return {
    dmPolicy: config.dmPolicy ?? "pairing",
    allowFrom: [...config.allowFrom],
    input: "",
  };
}

export type DmAccessDraftAction =
  /** (Re)load the committed values from a freshly read config. */
  | { type: "load"; config: ChannelConfig }
  | { type: "set-policy"; value: string }
  | { type: "set-input"; value: string }
  | { type: "add-entry" }
  | { type: "remove-entry"; entry: string };

/**
 * Draft reducer:
 * - `add-entry` trims the input, requires a valid (non-duplicate) entry,
 *   and clears the input only on success (invalid input stays for correction);
 * - `load` replaces the committed values but keeps the in-progress input.
 */
export function dmAccessDraftReducer(
  state: DmAccessDraft,
  action: DmAccessDraftAction,
): DmAccessDraft {
  switch (action.type) {
    case "load":
      return { ...dmAccessDraftFromConfig(action.config) };
    case "set-policy":
      return { ...state, dmPolicy: action.value };
    case "set-input":
      return { ...state, input: action.value };
    case "add-entry": {
      const entry = state.input.trim();
      if (!isValidAllowFromEntry(entry) || state.allowFrom.includes(entry)) {
        return state;
      }
      return { ...state, allowFrom: [...state.allowFrom, entry], input: "" };
    }
    case "remove-entry":
      return { ...state, allowFrom: state.allowFrom.filter((e) => e !== action.entry) };
  }
}

/** Whether the DM-access draft differs from the committed config (save gate). */
export function dmAccessDraftDirty(draft: DmAccessDraft, config: ChannelConfig): boolean {
  const policy = config.dmPolicy ?? "pairing";
  const key = (entries: string[]) => entries.join("\u0000");
  return draft.dmPolicy !== policy || key(draft.allowFrom) !== key(config.allowFrom);
}

/** Whether the group policy differs from the committed config (save gate). */
export function groupPolicyDirty(value: string, config: ChannelConfig): boolean {
  return value !== (config.groupPolicy ?? "allowlist");
}

// --- pairing list (single-flight read) --------------------------------------------

export interface PairingState {
  loading: boolean;
  /** The loaded rows (`null` = not loaded yet / cleared on failure). */
  requests: { code: string; sender: string | null }[] | null;
  /** Korean message of the last failed load (null = none). */
  error: string | null;
}

export const initialPairingState: PairingState = {
  loading: false,
  requests: null,
  error: null,
};

export type PairingAction =
  | { type: "start" }
  | { type: "finish"; requests: { code: string; sender: string | null }[] | null; error: string | null };

/**
 * Pairing-list reducer:
 * - `start` while loading is ignored (duplicate-run guard);
 * - `finish` with an error is fail-closed: any previous rows are cleared
 *   (the UI never keeps showing possibly stale pairing codes).
 */
export function pairingReducer(state: PairingState, action: PairingAction): PairingState {
  switch (action.type) {
    case "start":
      if (state.loading) {
        return state;
      }
      return { loading: true, requests: null, error: null };
    case "finish":
      if (action.error !== null) {
        return { loading: false, requests: null, error: action.error };
      }
      return { loading: false, requests: action.requests, error: null };
  }
}

// --- runtime state display ---------------------------------------------------------

/** Known `channels status` state values → Korean label (fail-soft). */
export function runtimeStateLabel(raw: string | null): "connected" | "unknown" {
  return raw === "connected" ? "connected" : "unknown";
}

/** Token state → display key (drives which token actions are offered). */
export type TokenView = "absent" | "managed" | "external";

export function tokenView(config: ChannelConfig): TokenView {
  return config.tokenState;
}

// --- error mapping ---------------------------------------------------------------------

/** Error codes the channels feature can receive (stable, from Rust). */
export const CHANNELS_ERROR_CODES = [
  "channel-id-invalid",
  "channel-token-invalid",
  "channel-token-not-found",
  "dm-policy-invalid",
  "group-policy-invalid",
  "allow-from-entry-invalid",
  "dm-access-inconsistent",
  "pairing-code-invalid",
  "openclaw-channels-failed",
  "openclaw-pairing-failed",
  "openclaw-plugin-install-failed",
  "openclaw-config-read-failed",
  "openclaw-config-write-failed",
  "openclaw-config-invalid",
  "secret-store-unavailable",
  "secret-ref-registration-failed",
  "openclaw-not-found",
  "process-timeout",
  "process-failed",
] as const;

export type ChannelsErrorCode = (typeof CHANNELS_ERROR_CODES)[number];

/**
 * Maps an IPC `AppError` to an i18n key. Unknown codes fall back so the UI
 * can always show a message (the fallback message is generic Korean).
 */
export function mapChannelsError(error: TauriAppError): ChannelsErrorCode | "fallback" {
  return (CHANNELS_ERROR_CODES as readonly string[]).includes(error.code)
    ? (error.code as ChannelsErrorCode)
    : "fallback";
}

/** Convenience guards for the component (enum membership of raw values). */
export function isKnownDmPolicy(value: string | null): value is (typeof DM_POLICIES)[number] {
  return value !== null && (DM_POLICIES as readonly string[]).includes(value);
}

export function isKnownGroupPolicy(value: string | null): value is (typeof GROUP_POLICIES)[number] {
  return value !== null && (GROUP_POLICIES as readonly string[]).includes(value);
}
