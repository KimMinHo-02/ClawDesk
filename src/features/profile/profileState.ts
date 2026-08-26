/**
 * Pure logic for the Phase 8 profile feature (error mapping, log-limit
 * validation, log line formatting). No Tauri calls here — keep this
 * unit-testable without the IPC layer.
 *
 * The feature is read-only (PRODUCT_CONTRACT §4.7): four independent
 * sections (agents / update / gateway / diagnostics), each with its own
 * loading/error/data state and a refresh action. There are no optimistic
 * updates — a refresh always shows the actual state.
 */

import { type LogEvent, type TauriAppError } from "../../lib/tauri";

// --- log limit (one-shot tail) -------------------------------------------------

/** The selectable tail sizes (the backend accepts 1..=1000). */
export const LOG_LIMIT_OPTIONS = [50, 100, 200, 500] as const;

/** The default tail size (matches the OpenClaw CLI default). */
export const DEFAULT_LOG_LIMIT = 200;

export const LOG_LIMIT_MIN = 1;
export const LOG_LIMIT_MAX = 1000;

/** Frontend pre-validation (the Rust service re-validates, fail-closed). */
export function isValidLogLimit(limit: number): boolean {
  return Number.isInteger(limit) && limit >= LOG_LIMIT_MIN && limit <= LOG_LIMIT_MAX;
}

// --- log line display ------------------------------------------------------------

/**
 * Renders one type-tagged log event as a single plain-text line for the
 * viewer. Fail-soft: absent optional fields are simply omitted.
 * The viewer data is already masked by the Rust pipeline (S8).
 */
export function formatLogEvent(event: LogEvent): string {
  switch (event.kind) {
    case "log": {
      const parts: string[] = [];
      if (event.time) parts.push(event.time);
      if (event.level) parts.push(event.level.toUpperCase());
      if (event.subsystem) parts.push(event.subsystem);
      const prefix = parts.length > 0 ? `${parts.join(" ")} ` : "";
      return `${prefix}${event.message}`;
    }
    case "meta":
      return `[meta] ${event.file ?? "?"} (${event.sourceKind ?? "unknown"})`;
    case "notice":
      return event.message ? `[notice] ${event.message}` : "[notice]";
    case "raw":
      return event.line;
  }
}

// --- error mapping -----------------------------------------------------------------

/** Error codes the profile feature can receive (stable, from the Rust layer). */
export const PROFILE_ERROR_CODES = [
  "openclaw-agents-read-failed",
  "openclaw-logs-read-failed",
  "logs-limit-invalid",
  "openclaw-gateway-parse",
  "openclaw-not-found",
  "process-timeout",
  "process-failed",
] as const;

export type ProfileErrorCode = (typeof PROFILE_ERROR_CODES)[number];

/**
 * Maps an IPC `AppError` to an i18n key. Unknown codes fall back so the UI
 * can always show a message (the fallback message is generic Korean).
 */
export function mapProfileError(error: TauriAppError): ProfileErrorCode | "fallback" {
  return (PROFILE_ERROR_CODES as readonly string[]).includes(error.code)
    ? (error.code as ProfileErrorCode)
    : "fallback";
}
