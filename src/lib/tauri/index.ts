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
  listSkills: "list-skills",
  setSkillEnabled: "set-skill-enabled",
  listPlugins: "list-plugins",
  setPluginEnabled: "set-plugin-enabled",
  getPluginRuntime: "get-plugin-runtime",
  getToolPolicy: "get-tool-policy",
  setToolProfile: "set-tool-profile",
  setToolAllow: "set-tool-allow",
  setToolDeny: "set-tool-deny",
  setExecMode: "set-exec-mode",
  listSecurityProfiles: "list-security-profiles",
  saveSecurityProfile: "save-security-profile",
  deleteSecurityProfile: "delete-security-profile",
  applySecurityProfile: "apply-security-profile",
  runSecurityAudit: "run-security-audit",
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

// --- Phase 4 wire types ------------------------------------------------------
//
// Row fields the CLI may omit are optional (`null` → `undefined`/omitted);
// the UI degrades gracefully (fail-soft, contract §1/§2).

/** One row of `openclaw skills list --json`. */
export interface SkillRow {
  name: string;
  /** Configured state (`skills.entries.<name>.enabled`); null when absent. */
  enabled?: boolean | null;
  /** Load-time eligibility (`requires` gating); null when absent. */
  eligible?: boolean | null;
  description?: string | null;
  /** Load source (`workspace`, `bundled`, ...), when reported. */
  source?: string | null;
}

/** One row of `openclaw plugins list --json` (cold read). */
export interface PluginRow {
  id: string;
  enabled?: boolean | null;
  name?: string | null;
  format?: string | null;
  origin?: string | null;
  version?: string | null;
  dependencyStatus?: string | null;
}

/** Live runtime surface of one plugin (`plugins inspect --runtime --json`). */
export interface PluginRuntime {
  id: string;
  tools: string[];
  hooks: string[];
  services: string[];
  cliCommands: string[];
  gatewayMethods: string[];
  routes: string[];
  diagnostics?: string[] | null;
}

// --- Phase 5 wire types ------------------------------------------------------
//
// Read-side fail-soft: unset fields arrive as `null`/empty arrays and unknown
// enum values are kept raw (the write-side enums gate user input only).

/** The four OpenClaw tool profile values (`tools.profile`). */
export const TOOL_PROFILES = ["minimal", "coding", "messaging", "full"] as const;
export type ToolProfile = (typeof TOOL_PROFILES)[number];

/** The five OpenClaw exec mode values (`tools.exec.mode`). */
export const EXEC_MODES = ["deny", "allowlist", "ask", "auto", "full"] as const;
export type ExecMode = (typeof EXEC_MODES)[number];

/** The current tool policy (`get-tool-policy` response, redacted). */
export interface ToolPolicy {
  /** `null` when unset (unset behaves as `full`); raw string when unknown. */
  profile: string | null;
  /** `tools.allow` entries (tool id / `group:*` / wildcard pattern). */
  allow: string[];
  /** `tools.deny` entries — deny wins over allow. */
  deny: string[];
  /** `null` when unset (host default: no approval gate); raw when unknown. */
  execMode: string | null;
  /** Read-only display (`tools.elevated.enabled`). */
  elevatedEnabled: boolean | null;
  /** Read-only display (`tools.fs.workspaceOnly`). */
  fsWorkspaceOnly: boolean | null;
}

/** A named tool-policy preset (builtin or user). */
export interface SecurityProfile {
  id: string;
  /** Display-only name (1–50 chars, no control characters). */
  name: string;
  /** `tools.profile` enum: `minimal` | `coding` | `messaging` | `full`. */
  baseProfile: string;
  allow: string[];
  deny: string[];
  /** `tools.exec.mode` enum: `deny` | `allowlist` | `ask` | `auto` | `full`. */
  execMode: string;
}

/** `list-security-profiles` response. */
export interface SecurityProfileList {
  builtins: SecurityProfile[];
  users: SecurityProfile[];
  /** Id of the profile matching the current policy (builtins first), or
   * `null` when the policy is custom or could not be read. */
  currentApplied: string | null;
  /** True when the current policy read failed (the list is still shown). */
  policyReadFailed: boolean;
}

/** One finding row of `openclaw security audit --json`.
 *
 * `checkId` is required (rows without it are dropped by the Rust layer);
 * the rest are fail-soft. Unknown severity values keep the raw string.
 */
export interface SecurityFinding {
  checkId: string;
  severity?: string | null;
  title?: string | null;
  detail?: string | null;
}

/** `run-security-audit` response.
 *
 * `summary` has no asserted schema (informational display only).
 * `suppressedCount` is display-only (suppressed details are never surfaced).
 */
export interface SecurityAuditResult {
  summary: unknown;
  findings: SecurityFinding[];
  suppressedCount: number;
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

// --- Phase 4 command wrappers ------------------------------------------------

/** `list-skills`: all skills (`openclaw skills list --json`, read-only). */
export async function listSkills(): Promise<SkillRow[]> {
  return invoke<SkillRow[]>(COMMANDS.listSkills);
}

/** `set-skill-enabled`: toggles `skills.entries.<name>.enabled`.
 *
 * Applies from the next new session; the UI re-queries after the response.
 */
export async function setSkillEnabled(
  skillName: string,
  enabled: boolean,
): Promise<void> {
  return invoke<void>(COMMANDS.setSkillEnabled, { skillName, enabled });
}

/** `list-plugins`: the cold plugin inventory (read-only). */
export async function listPlugins(): Promise<PluginRow[]> {
  return invoke<PluginRow[]>(COMMANDS.listPlugins);
}

/** `set-plugin-enabled`: `openclaw plugins enable/disable <id>`.
 *
 * On failure the UI re-queries `list-plugins` (no optimistic updates).
 */
export async function setPluginEnabled(
  pluginId: string,
  enabled: boolean,
): Promise<void> {
  return invoke<void>(COMMANDS.setPluginEnabled, { pluginId, enabled });
}

/** `get-plugin-runtime`: live runtime surface of one plugin (on-demand).
 *
 * This loads plugin modules, so it must not run while loading the list.
 */
export async function getPluginRuntime(pluginId: string): Promise<PluginRuntime> {
  return invoke<PluginRuntime>(COMMANDS.getPluginRuntime, { pluginId });
}

// --- Phase 5 command wrappers ------------------------------------------------

/** `get-tool-policy`: the current redacted tool policy (read-only). */
export async function getToolPolicy(): Promise<ToolPolicy> {
  return invoke<ToolPolicy>(COMMANDS.getToolPolicy);
}

/** `set-tool-profile`: sets `tools.profile` (enum-validated, two-step write). */
export async function setToolProfile(profile: ToolProfile): Promise<void> {
  return invoke<void>(COMMANDS.setToolProfile, { profile });
}

/** `set-tool-allow`: replaces the whole `tools.allow` array (`--replace`). */
export async function setToolAllow(entries: string[]): Promise<void> {
  return invoke<void>(COMMANDS.setToolAllow, { entries });
}

/** `set-tool-deny`: replaces the whole `tools.deny` array (`--replace`).
 *
 * Deny wins over allow.
 */
export async function setToolDeny(entries: string[]): Promise<void> {
  return invoke<void>(COMMANDS.setToolDeny, { entries });
}

/** `set-exec-mode`: sets `tools.exec.mode` (enum-validated, two-step write). */
export async function setExecMode(mode: ExecMode): Promise<void> {
  return invoke<void>(COMMANDS.setExecMode, { mode });
}

/** `list-security-profiles`: builtins + user profiles + applied state. */
export async function listSecurityProfiles(): Promise<SecurityProfileList> {
  return invoke<SecurityProfileList>(COMMANDS.listSecurityProfiles);
}

/** `save-security-profile`: upserts a user profile (builtins are immutable).
 *
 * No config write happens on save.
 */
export async function saveSecurityProfile(profile: SecurityProfile): Promise<void> {
  return invoke<void>(COMMANDS.saveSecurityProfile, { profile });
}

/** `delete-security-profile`: removes a user profile.
 *
 * Builtin ids and unknown ids are `security-profile-not-found`.
 */
export async function deleteSecurityProfile(profileId: string): Promise<void> {
  return invoke<void>(COMMANDS.deleteSecurityProfile, { profileId });
}

/** `apply-security-profile`: writes the profile's four fields to the config
 * (profile → allow → deny → exec mode; first failure stops).
 *
 * The UI re-queries the actual state afterwards (no optimistic updates).
 */
export async function applySecurityProfile(profileId: string): Promise<void> {
  return invoke<void>(COMMANDS.applySecurityProfile, { profileId });
}

/** `run-security-audit`: cold, read-only security audit.
 *
 * Never `--deep`/`--fix`, no credentials (Rust layer enforces the argv).
 */
export async function runSecurityAudit(): Promise<SecurityAuditResult> {
  return invoke<SecurityAuditResult>(COMMANDS.runSecurityAudit);
}
