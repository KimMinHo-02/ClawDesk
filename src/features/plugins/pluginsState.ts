/**
 * Pure logic for the Phase 4 plugins feature (toggle state, runtime inspect
 * state, error mapping). No Tauri calls here — keep it unit-testable.
 *
 * Toggle contract (no optimistic updates): on any finished toggle (success
 * or failure) `reloadCounter` is bumped so the component re-queries
 * `list-plugins` and shows the actual state (the CLI may have already
 * changed it).
 *
 * Runtime inspect contract: strictly on-demand — nothing in the list-load
 * path dispatches `request`, so the inspect (which loads plugin modules)
 * never runs automatically.
 */

import { type PluginRuntime, type TauriAppError } from "../../lib/tauri";

// --- toggle state --------------------------------------------------------------

/** Toggle state for the plugin list. */
export interface PluginsToggleState {
  /** The plugin id currently being toggled (null = idle). */
  pending: string | null;
  /** Korean message of the last failed toggle (null = none). */
  error: string | null;
  /** Bumped on every finished toggle → triggers a list re-query. */
  reloadCounter: number;
}

export const initialPluginsToggleState: PluginsToggleState = {
  pending: null,
  error: null,
  reloadCounter: 0,
};

export type PluginsToggleAction =
  | { type: "start"; key: string }
  | { type: "finish"; error: string | null };

/**
 * Toggle reducer:
 * - `start` while a toggle is pending is ignored (duplicate-submit guard);
 * - `finish` always bumps `reloadCounter` (re-query on success AND failure).
 */
export function pluginsToggleReducer(
  state: PluginsToggleState,
  action: PluginsToggleAction,
): PluginsToggleState {
  switch (action.type) {
    case "start":
      if (state.pending !== null) {
        return state;
      }
      return { pending: action.key, error: null, reloadCounter: state.reloadCounter };
    case "finish":
      return {
        pending: null,
        error: action.error,
        reloadCounter: state.reloadCounter + 1,
      };
  }
}

// --- runtime inspect state ----------------------------------------------------------

/** Runtime-inspect state for the selected plugin. */
export interface PluginsRuntimeState {
  /** The plugin id whose runtime was requested (null = never requested). */
  requestedId: string | null;
  /** The plugin id currently being inspected (null = idle). */
  loadingId: string | null;
  /** The last successful runtime payload (null = none / cleared on error). */
  data: PluginRuntime | null;
  /** Korean message of the last failed inspect (null = none). */
  error: string | null;
}

export const initialPluginsRuntimeState: PluginsRuntimeState = {
  requestedId: null,
  loadingId: null,
  data: null,
  error: null,
};

export type PluginsRuntimeAction =
  | { type: "request"; id: string }
  | { type: "finish"; id: string; error: string | null; data: PluginRuntime | null };

/**
 * Runtime reducer:
 * - `request` starts an on-demand inspect (ignored while one is loading);
 * - `finish` with an error clears any previous data (fail-closed: the UI
 *   must not keep showing a possibly stale "loaded" state);
 * - `finish` with data stores it for display.
 */
export function pluginsRuntimeReducer(
  state: PluginsRuntimeState,
  action: PluginsRuntimeAction,
): PluginsRuntimeState {
  switch (action.type) {
    case "request":
      if (state.loadingId !== null) {
        return state;
      }
      return {
        requestedId: action.id,
        loadingId: action.id,
        data: state.data,
        error: null,
      };
    case "finish":
      if (action.error !== null) {
        return {
          requestedId: action.id,
          loadingId: null,
          data: null,
          error: action.error,
        };
      }
      return {
        requestedId: action.id,
        loadingId: null,
        data: action.data,
        error: null,
      };
  }
}

// --- error mapping -------------------------------------------------------------------

/** Error codes the plugins feature can receive (stable, from the Rust layer). */
export const PLUGINS_ERROR_CODES = [
  "plugin-id-invalid",
  "openclaw-plugin-toggle-failed",
  "openclaw-plugins-read-failed",
  "openclaw-not-found",
  "process-timeout",
  "process-failed",
] as const;

export type PluginsErrorCode = (typeof PLUGINS_ERROR_CODES)[number];

/**
 * Maps an IPC `AppError` to an i18n key. Unknown codes fall back so the UI
 * can always show a message (the fallback message is generic Korean).
 */
export function mapPluginsError(error: TauriAppError): PluginsErrorCode | "fallback" {
  return (PLUGINS_ERROR_CODES as readonly string[]).includes(error.code)
    ? (error.code as PluginsErrorCode)
    : "fallback";
}
