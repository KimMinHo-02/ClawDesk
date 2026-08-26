/**
 * Pure state logic for the setup/install feature (Phase 2).
 *
 * Kept free of React and DOM so the core state mapping is unit-testable.
 * Error mapping uses the `install` i18n namespace keyed by the stable
 * `AppError` code (architecture §5: the frontend maps stable codes to
 * user messages, never raw infrastructure detail).
 */

import { getStrings } from "../../i18n/ko";
import {
  type EnvironmentReport,
  type NodeDetection,
  type OpenClawStatus,
  isTauriAppError,
} from "../../lib/tauri";

const installStrings = getStrings("install");

/**
 * Mirror of the Rust `node_version_supported` policy (Phase 2 contract):
 * Node 22 >= 22.22.3, 24 >= 24.15, 25 >= 25.9, 26+; everything else
 * (including Node 23) is rejected.
 */
export function isNodeSupported(version: string): boolean {
  const core = version.trim().split(/[-+]/)[0];
  const parts = core.split(".");
  if (parts.length < 2) {
    return false;
  }
  const major = Number(parts[0]);
  const minor = Number(parts[1]);
  const patch = parts.length >= 3 ? Number(parts[2]) : 0;
  if (![major, minor, patch].every((n) => Number.isInteger(n) && n >= 0)) {
    return false;
  }
  switch (major) {
    case 22:
      return minor > 22 || (minor === 22 && patch >= 3);
    case 24:
      return minor >= 15;
    case 25:
      return minor >= 9;
    default:
      return major >= 26;
  }
}

/** Node state for display. */
export type NodeState =
  | { kind: "not-found" }
  | { kind: "supported"; version: string }
  | { kind: "unsupported"; version: string };

export function nodeStateOf(node: NodeDetection): NodeState {
  if (node.status === "not-found") {
    return { kind: "not-found" };
  }
  return isNodeSupported(node.version)
    ? { kind: "supported", version: node.version }
    : { kind: "unsupported", version: node.version };
}

/**
 * Whether the Phase 8.1 one-shot Node.js update is offered: the Node is
 * PRESENT but unsupported. A missing Node stays guidance-only (Phase 2
 * contract — no auto-install).
 */
export function isNodeUpdateNeeded(node: NodeDetection): boolean {
  return nodeStateOf(node).kind === "unsupported";
}

/** OpenClaw state for display. */
export type OpenClawUiState =
  | { kind: "not-installed" }
  | { kind: "installed"; version: string | null };

export function openclawStateOf(openclaw: OpenClawStatus): OpenClawUiState {
  if (openclaw.status === "not-found") {
    return { kind: "not-installed" };
  }
  return { kind: "installed", version: openclaw.version };
}

/**
 * Whether the install button is actionable: OpenClaw must be missing and
 * Node must be present with a supported version. npm state is only known
 * during install (surfaced via stable error codes on failure).
 */
export function canInstall(report: EnvironmentReport): boolean {
  if (openclawStateOf(report.openclaw).kind === "installed") {
    return false;
  }
  return nodeStateOf(report.node).kind === "supported";
}

/** The stable `AppError` code of an IPC rejection, when structured. */
export function errorCodeOf(err: unknown): string | undefined {
  return isTauriAppError(err) ? err.code : undefined;
}

/**
 * Maps any IPC failure to a Korean user message using the stable
 * `AppError` code. Unknown codes and non-`AppError` rejections fall back
 * to the generic message.
 */
export function errorToMessage(err: unknown): string {
  const code = errorCodeOf(err);
  const errors = installStrings.errors as Record<string, string>;
  return (code !== undefined && errors[code]) || errors.fallback;
}
