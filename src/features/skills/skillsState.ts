/**
 * Pure logic for the Phase 4 skills feature (toggle state, error mapping).
 * No Tauri calls here — keep this unit-testable without the IPC layer.
 *
 * Toggle contract (no optimistic updates): the UI never mutates the list
 * locally; every finished toggle (success or failure) bumps `reloadCounter`
 * so the component re-queries `list-skills` and shows the actual state.
 */

import { type TauriAppError } from "../../lib/tauri";

// --- toggle state --------------------------------------------------------------

/** Toggle state for the skill list. */
export interface SkillsToggleState {
  /** The skill name currently being toggled (null = idle). */
  pending: string | null;
  /** Korean message of the last failed toggle (null = none). */
  error: string | null;
  /** Bumped on every finished toggle → triggers a list re-query. */
  reloadCounter: number;
}

export const initialSkillsToggleState: SkillsToggleState = {
  pending: null,
  error: null,
  reloadCounter: 0,
};

export type SkillsToggleAction =
  | { type: "start"; key: string }
  | { type: "finish"; error: string | null };

/**
 * Toggle reducer:
 * - `start` while a toggle is pending is ignored (duplicate-submit guard);
 * - `finish` always bumps `reloadCounter` (re-query on success AND failure —
 *   no optimistic updates, the CLI applies changes from the next session).
 */
export function skillsToggleReducer(
  state: SkillsToggleState,
  action: SkillsToggleAction,
): SkillsToggleState {
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

// --- error mapping -----------------------------------------------------------------

/** Error codes the skills feature can receive (stable, from the Rust layer). */
export const SKILLS_ERROR_CODES = [
  "skill-name-invalid",
  "skill-not-found",
  "openclaw-skills-read-failed",
  "openclaw-config-write-failed",
  "openclaw-config-invalid",
  "openclaw-not-found",
  "process-timeout",
  "process-failed",
] as const;

export type SkillsErrorCode = (typeof SKILLS_ERROR_CODES)[number];

/**
 * Maps an IPC `AppError` to an i18n key. Unknown codes fall back so the UI
 * can always show a message (the fallback message is generic Korean).
 */
export function mapSkillsError(error: TauriAppError): SkillsErrorCode | "fallback" {
  return (SKILLS_ERROR_CODES as readonly string[]).includes(error.code)
    ? (error.code as SkillsErrorCode)
    : "fallback";
}
