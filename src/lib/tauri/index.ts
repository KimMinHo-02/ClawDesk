/**
 * Tauri IPC wrapper — the single source of truth for frontend command
 * names and request/response types (architecture §5).
 *
 * Command names are kebab-case here; Tauri maps them to the Rust
 * snake_case commands (`detect_environment`, `install_openclaw`).
 * All OS/OpenClaw work goes through this IPC boundary — the frontend
 * never spawns processes or shells directly (S1/S10).
 */

import { invoke } from "@tauri-apps/api/core";

/** Frontend command names (kebab-case). Defined in exactly one place. */
export const COMMANDS = {
  detectEnvironment: "detect-environment",
  installOpenClaw: "install-openclaw",
  listProviders: "list-providers",
  getProvider: "get-provider",
  saveProvider: "save-provider",
  deleteProvider: "delete-provider",
  listModels: "list-models",
  getDefaultModel: "get-default-model",
  setDefaultModel: "set-default-model",
  getReasoningDefault: "get-reasoning-default",
  setReasoningDefault: "set-reasoning-default",
  setProviderApiKey: "set-provider-api-key",
  deleteProviderApiKey: "delete-provider-api-key",
  listApiKeys: "list-api-keys",
} as const;

// --- Wire types (mirror the serde shapes in `src-tauri`) --------------------

/** `WindowsVersion` (src-tauri: domain::models::windows). */
export interface WindowsVersionInfo {
  major_version: number;
  build: number;
  ubr: number;
  product_name: string | null;
}

/** Only x64 exists on the wire; non-x64 is a structured Rust error. */
export type Architecture = "x64";

/** `NodeDetection` — internally tagged `status`, kebab-case. */
export type NodeDetection =
  | { status: "not-found" }
  | { status: "found"; version: string };

/** `UpdateState`. */
export type UpdateState = "updated" | "update-available" | "unknown";

/** `GatewayStatus`. */
export interface GatewayStatus {
  state: string;
  version: string | null;
  port: number | null;
}

/** `OpenClawStatus` — internally tagged `status`, kebab-case. */
export type OpenClawStatus =
  | { status: "not-found" }
  | {
      status: "detected";
      executable: string;
      version: string | null;
      gateway: GatewayStatus | null;
      update: UpdateState;
    };

/** `EnvironmentReport` — `detect-environment` response. */
export interface EnvironmentReport {
  windows_version: WindowsVersionInfo;
  architecture: Architecture;
  node: NodeDetection;
  openclaw: OpenClawStatus;
}

/** `InstallResult` — `install-openclaw` response. */
export type InstallResult =
  | { status: "installed"; version: string }
  | { status: "already-installed"; version: string };

// --- Phase 3 wire types ------------------------------------------------------
//
// The Phase 3 wire format mirrors the OpenClaw config format: camelCase
// field names (`baseUrl`, `contextWindow`, `supportedReasoningEfforts`), so
// provider/model payloads round-trip unchanged between UI and config.
// (Phase 2 types above keep their original snake_case shapes.)

/** The standard OpenClaw thinking-level ladder. */
export type ThinkingLevel =
  | "off"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "adaptive"
  | "max"
  | "ultra";

/** How the provider's `apiKey` field is populated (never the value, S7). */
export type ProviderApiKeyState = "absent" | "managed" | "other";

/** Reasoning-capability metadata (model `compat` block). */
export interface ModelCompat {
  supportsReasoningEffort?: boolean;
  supportedReasoningEfforts?: ThinkingLevel[];
}

/** A model entry in a provider (config-format shape). */
export interface ModelEntry {
  id: string;
  name?: string;
  reasoning: boolean;
  input: string[];
  contextWindow?: number;
  maxTokens?: number;
  compat?: ModelCompat;
}

/** A provider entry as read from `models.providers.<id>` (redacted). */
export interface ProviderDetail {
  id: string;
  baseUrl?: string;
  api?: string;
  apiKeyState?: ProviderApiKeyState;
  models: ModelEntry[];
}

/** One row of `openclaw models list --json`. */
export interface ModelRow {
  provider: string;
  model: string;
  /** `provider/model` reference. */
  full: string;
  name?: string;
  reasoning: boolean;
  contextTokens?: number;
  supportedReasoningEfforts?: ThinkingLevel[];
}

/** Provider list summary (`list-providers` response row). */
export interface ProviderSummary {
  id: string;
  baseUrl?: string;
  api?: string;
  modelCount: number;
  /** True only when a ClawDesk-managed key exists in the secret store. */
  apiKeyRegistered: boolean;
}

/** API key registration state (`list-api-keys` response row). */
export interface ApiKeyStatus {
  providerId: string;
  registered: boolean;
}

/** One model as submitted by the UI (`save-provider` payload part). */
export interface ModelInput {
  id: string;
  name?: string;
  reasoning?: boolean;
  input?: string[];
  contextWindow?: number;
  maxTokens?: number;
  supportsReasoningEffort?: boolean;
  supportedReasoningEfforts?: string[];
}

/** A provider as submitted by the UI (`save-provider` payload).
 *
 * Never contains an API key (S7): key management is a separate command.
 */
export interface ProviderInput {
  id: string;
  baseUrl?: string;
  api: string;
  models?: ModelInput[];
}

/**
 * Unified Rust `AppError` serialized across IPC: stable machine-readable
 * `code` plus a masked `message` (S3/S8). The frontend maps `code` to a
 * user-facing message; infrastructure detail never reaches the UI raw.
 */
export interface TauriAppError {
  code: string;
  message: string;
}

/** Type guard: is this IPC reject payload a structured `AppError`? */
export function isTauriAppError(err: unknown): err is TauriAppError {
  if (typeof err !== "object" || err === null) {
    return false;
  }
  const candidate = err as Record<string, unknown>;
  return (
    typeof candidate.code === "string" && typeof candidate.message === "string"
  );
}

/**
 * Normalizes any invoke failure into a structured `AppError`.
 * Non-`AppError` rejections (raw IPC failures) become `ipc-failed` so the
 * UI can always fall back to a generic user message.
 */
export function normalizeAppError(err: unknown): TauriAppError {
  if (isTauriAppError(err)) {
    return err;
  }
  const message =
    err instanceof Error ? err.message : typeof err === "string" ? err : "unknown error";
  return { code: "ipc-failed", message };
}

// --- Invoke wrappers ---------------------------------------------------------

/** `detect-environment`: current environment and OpenClaw state. */
export async function detectEnvironment(): Promise<EnvironmentReport> {
  return invoke<EnvironmentReport>(COMMANDS.detectEnvironment);
}

/** `install-openclaw`: installs `openclaw@latest` (or returns the existing install). */
export async function installOpenClaw(): Promise<InstallResult> {
  return invoke<InstallResult>(COMMANDS.installOpenClaw);
}

// --- Phase 3 command wrappers ------------------------------------------------

/** `list-providers`: provider summaries with computed key-registration state. */
export async function listProviders(): Promise<ProviderSummary[]> {
  return invoke<ProviderSummary[]>(COMMANDS.listProviders);
}

/** `get-provider`: full redacted provider detail (never includes key values). */
export async function getProvider(providerId: string): Promise<ProviderDetail> {
  return invoke<ProviderDetail>(COMMANDS.getProvider, { providerId });
}

/** `save-provider`: upsert (new = merge, existing = field-level update). */
export async function saveProvider(provider: ProviderInput): Promise<void> {
  return invoke<void>(COMMANDS.saveProvider, { provider });
}

/** `delete-provider`: removes the provider (and its managed key, if any). */
export async function deleteProvider(providerId: string): Promise<void> {
  return invoke<void>(COMMANDS.deleteProvider, { providerId });
}

/** `list-models`: all models across providers (read-only). */
export async function listModels(): Promise<ModelRow[]> {
  return invoke<ModelRow[]>(COMMANDS.listModels);
}

/** `get-default-model`: current default `provider/model` ref, if set. */
export async function getDefaultModel(): Promise<string | null> {
  return invoke<string | null>(COMMANDS.getDefaultModel);
}

/** `set-default-model`: `openclaw models set <provider/model>`. */
export async function setDefaultModel(modelRef: string): Promise<void> {
  return invoke<void>(COMMANDS.setDefaultModel, { modelRef });
}

/** `get-reasoning-default`: global thinking default, or `null` when unset. */
export async function getReasoningDefault(): Promise<ThinkingLevel | null> {
  return invoke<ThinkingLevel | null>(COMMANDS.getReasoningDefault);
}

/** `set-reasoning-default`: global thinking default (enum-validated). */
export async function setReasoningDefault(level: ThinkingLevel): Promise<void> {
  return invoke<void>(COMMANDS.setReasoningDefault, { level });
}

/** `set-provider-api-key`: registers the key in DPAPI only (S7).
 *
 * The key value travels to the secret store and is never shown again.
 */
export async function setProviderApiKey(
  providerId: string,
  apiKey: string,
): Promise<void> {
  return invoke<void>(COMMANDS.setProviderApiKey, { providerId, apiKey });
}

/** `delete-provider-api-key`: removes the managed key (config ref + store). */
export async function deleteProviderApiKey(providerId: string): Promise<void> {
  return invoke<void>(COMMANDS.deleteProviderApiKey, { providerId });
}

/** `list-api-keys`: registration state of managed keys (non-secret). */
export async function listApiKeys(): Promise<ApiKeyStatus[]> {
  return invoke<ApiKeyStatus[]>(COMMANDS.listApiKeys);
}
