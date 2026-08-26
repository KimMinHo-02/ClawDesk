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
  getChannels: "get-channels",
  getChannelConfig: "get-channel-config",
  setChannelToken: "set-channel-token",
  deleteChannelToken: "delete-channel-token",
  connectChannel: "connect-channel",
  setChannelEnabled: "set-channel-enabled",
  setDmAccess: "set-dm-access",
  setGroupPolicy: "set-group-policy",
  listPairingRequests: "list-pairing-requests",
  approvePairing: "approve-pairing",
  getAutomations: "get-automations",
  getAutomation: "get-automation",
  createAutomation: "create-automation",
  updateAutomation: "update-automation",
  setAutomationEnabled: "set-automation-enabled",
  deleteAutomation: "delete-automation",
  getGatewayStatus: "get-gateway-status",
  getUpdateStatus: "get-update-status",
  getAgents: "get-agents",
  getLogs: "get-logs",
  updateNode: "update-node",
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

// --- Phase 6 wire types ------------------------------------------------------
//
// Read-side fail-soft: `enabled`/`dmPolicy`/`groupPolicy`/`runtimeState`
// arrive as `null` when unset, and unknown raw values are kept (the
// write-side enums gate user input only). The channel token value itself is
// never a field (S7) — only its `managed`/`external`/`absent` state.

/** How the channel token field is populated (redacted, never the value). */
export type ChannelTokenState = "absent" | "managed" | "external";

/** The four OpenClaw DM policy values (`channels.<ch>.dmPolicy`). */
export const DM_POLICIES = ["pairing", "allowlist", "open", "disabled"] as const;
export type DmPolicy = (typeof DM_POLICIES)[number];

/** The three OpenClaw group policy values (`channels.<ch>.groupPolicy`). */
export const GROUP_POLICIES = ["open", "allowlist", "disabled"] as const;
export type GroupPolicy = (typeof GROUP_POLICIES)[number];

/** A redacted `channels.<channel>` snapshot (`get-channel-config`). */
export interface ChannelConfig {
  /** `null` when the section is absent. */
  enabled: boolean | null;
  /** Token field state (the value itself never crosses IPC, S7). */
  tokenState: ChannelTokenState;
  /** `null` when unset; unknown raw values are kept. */
  dmPolicy: string | null;
  /** Non-array/absent → empty; non-string elements skipped. */
  allowFrom: string[];
  /** `null` when unset; unknown raw values are kept. */
  groupPolicy: string | null;
}

/** A merged row of `get-channels` (list row + status row). */
export interface ChannelSummary {
  id: string;
  installed: boolean;
  configured: boolean;
  enabled: boolean;
  /** Raw runtime state (`null` when the gateway is unreachable). */
  runtimeState: string | null;
}

/** `get-channels` response. */
export interface ChannelsOverview {
  /** `false` → the UI presents config-based state only (no "connected" guess). */
  gatewayReachable: boolean;
  channels: ChannelSummary[];
}

/** One pending pairing request (`list-pairing-requests` row). */
export interface PairingRequest {
  code: string;
  /** Fail-soft; `null` when absent. */
  sender: string | null;
}

// --- Phase 7 wire types ------------------------------------------------------
//
// Read-side fail-soft: `name`/`enabled`/`status`/`schedule`/`payload` arrive
// as `null` when the CLI omits them, and unknown raw values are kept (the
// write-side enums gate user input only). The session field never crosses the
// wire — the Rust layer fixes the pairing by payload kind.

/** The schedule kinds ClawDesk manages (contract: `at`/`every`/`cron` only). */
export const SCHEDULE_KINDS = ["at", "every", "cron"] as const;
export type ScheduleKind = (typeof SCHEDULE_KINDS)[number];

/** The payload kinds ClawDesk manages (contract: `reminder`/`task` only). */
export const PAYLOAD_KINDS = ["reminder", "task"] as const;
export type PayloadKind = (typeof PAYLOAD_KINDS)[number];

/** The reminder wake values. */
export const WAKE_VALUES = ["now", "next-heartbeat"] as const;
export type WakeValue = (typeof WAKE_VALUES)[number];

/** Best-effort schedule view of a job (fail-soft; unknown shapes are `null`). */
export interface AutomationScheduleView {
  kind: string;
  value: string | null;
  tz: string | null;
}

/** Best-effort payload view of a job (fail-soft; unknown shapes are `null`). */
export interface AutomationPayloadView {
  kind: string;
  text: string | null;
}

/** One row of `openclaw automations list --all --json` (fail-soft). */
export interface AutomationJobRow {
  id: string;
  name: string | null;
  enabled: boolean | null;
  /** Raw status (`null` when absent); unknown values are kept. */
  status: string | null;
  nextRunAtMs: number | null;
  schedule: AutomationScheduleView | null;
  payload: AutomationPayloadView | null;
}

/** The `get-automation` detail (fail-soft). */
export interface AutomationJob {
  id: string;
  name: string | null;
  enabled: boolean | null;
  status: string | null;
  schedule: AutomationScheduleView | null;
  payload: AutomationPayloadView | null;
}

/** `get-automations` response. */
export interface AutomationJobList {
  jobs: AutomationJobRow[];
}

/** `create-automation` response. */
export interface AutomationCreated {
  jobId: string;
}

// --- Phase 8 wire types ------------------------------------------------------
//
// Read-only display shapes (PRODUCT_CONTRACT §4.7). `current`/`latest` and
// the optional agent/log fields are omitted on the wire when absent
// (fail-soft — the display degrades, it never errors per row).

/** `get-update-status` response: the Phase 1 state plus version strings. */
export interface UpdateStatusDetail {
  state: UpdateState;
  /** Omitted when the state could not be determined. */
  current?: string | null;
  /** Omitted when the state could not be determined. */
  latest?: string | null;
}

/** One row of `openclaw agents list --json` (read-only display). */
export interface AgentRow {
  id: string;
  /** `true` for the default agent (`main`). */
  default: boolean;
  name?: string | null;
  emoji?: string | null;
  /** The agent's workspace directory, when reported. */
  workspace?: string | null;
  /** Channel binding count, when reported. */
  bindings?: number | null;
}

/** One type-tagged event of `openclaw logs --limit <n> --json`.
 *
 * Tagged by `kind`; non-classifiable lines arrive as `raw`.
 */
export type LogEvent =
  | {
      kind: "log";
      time?: string | null;
      level?: string | null;
      subsystem?: string | null;
      message: string;
      hostname?: string | null;
      agentId?: string | null;
      sessionId?: string | null;
      channel?: string | null;
    }
  | {
      kind: "meta";
      file?: string | null;
      source?: string | null;
      sourceKind?: string | null;
      service?: string | null;
      cursor?: string | null;
      size?: number | null;
    }
  | {
      kind: "notice";
      message?: string | null;
      truncated?: boolean | null;
    }
  | {
      kind: "raw";
      line: string;
    };

/** `get-logs` response: the one-shot tail result.
 *
 * An empty tail (no log lines) is a successful zero-line result.
 */
export interface LogsResult {
  lines: LogEvent[];
  /** Log file from the first `meta` event, if any. */
  source?: string | null;
  /** True when a `notice` event reports truncation. */
  truncated: boolean;
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

// --- Phase 6 command wrappers ------------------------------------------------

/** `get-channels`: merged `channels list --all` + `channels status` rows for
 * discord/telegram (read-only). */
export async function getChannels(): Promise<ChannelsOverview> {
  return invoke<ChannelsOverview>(COMMANDS.getChannels);
}

/** `get-channel-config`: redacted `channels.<channel>` snapshot.
 *
 * The token value is never included (S7) — only its state.
 */
export async function getChannelConfig(channel: string): Promise<ChannelConfig> {
  return invoke<ChannelConfig>(COMMANDS.getChannelConfig, { channel });
}

/** `set-channel-token`: registers the token in DPAPI first (S7), then writes
 * the exec SecretRef to the config.
 *
 * The token value travels to Rust only — it is never shown again.
 */
export async function setChannelToken(channel: string, token: string): Promise<void> {
  return invoke<void>(COMMANDS.setChannelToken, { channel, token });
}

/** `delete-channel-token`: removes the managed ref + DPAPI entry. */
export async function deleteChannelToken(channel: string): Promise<void> {
  return invoke<void>(COMMANDS.deleteChannelToken, { channel });
}

/** `connect-channel`: token-ref precondition → (Discord) idempotent plugin
 * install → `enabled=true`. Fixed order, first failure stops.
 *
 * The UI re-queries the actual state afterwards (no optimistic updates).
 */
export async function connectChannel(channel: string): Promise<void> {
  return invoke<void>(COMMANDS.connectChannel, { channel });
}

/** `set-channel-enabled`: scalar `enabled` write (disable keeps token and
 * policies). */
export async function setChannelEnabled(channel: string, enabled: boolean): Promise<void> {
  return invoke<void>(COMMANDS.setChannelEnabled, { channel, enabled });
}

/** `set-dm-access`: writes `dmPolicy` → `allowFrom` (`--replace`) in a fixed
 * order (pre-validated on both sides). */
export async function setDmAccess(
  channel: string,
  dmPolicy: string,
  allowFrom: string[],
): Promise<void> {
  return invoke<void>(COMMANDS.setDmAccess, { channel, dmPolicy, allowFrom });
}

/** `set-group-policy`: enum-validated scalar `groupPolicy` write. */
export async function setGroupPolicy(channel: string, groupPolicy: string): Promise<void> {
  return invoke<void>(COMMANDS.setGroupPolicy, { channel, groupPolicy });
}

/** `list-pairing-requests`: pending pairing requests for the channel. */
export async function listPairingRequests(channel: string): Promise<PairingRequest[]> {
  return invoke<PairingRequest[]>(COMMANDS.listPairingRequests, { channel });
}

/** `approve-pairing`: channel + code validated before the CLI call. */
export async function approvePairing(channel: string, code: string): Promise<void> {
  return invoke<void>(COMMANDS.approvePairing, { channel, code });
}

// --- Phase 7 command wrappers ------------------------------------------------

/** `get-automations`: all job rows including disabled (read-only). */
export async function getAutomations(): Promise<AutomationJobList> {
  return invoke<AutomationJobList>(COMMANDS.getAutomations);
}

/** `get-automation`: one job detail (id pre-validated, fail-closed). */
export async function getAutomation(jobId: string): Promise<AutomationJob> {
  return invoke<AutomationJob>(COMMANDS.getAutomation, { jobId });
}

/** `create-automation`: reminder or task. The session pairing is fixed in
 * Rust (the wire carries no session field). All inputs are format-validated
 * in Rust before any CLI call (S2). Returns the new job id.
 */
export async function createAutomation(
  name: string,
  scheduleKind: string,
  scheduleValue: string,
  scheduleTz: string | null,
  payloadKind: string,
  text: string,
  wake: string | null,
): Promise<AutomationCreated> {
  return invoke<AutomationCreated>(COMMANDS.createAutomation, {
    name,
    scheduleKind,
    scheduleValue,
    scheduleTz,
    payloadKind,
    text,
    wake,
  });
}

/** `update-automation`: same field set as create; the payload kind cannot
 * change (kind change = delete + recreate, blocked by the UI). */
export async function updateAutomation(
  jobId: string,
  name: string,
  scheduleKind: string,
  scheduleValue: string,
  scheduleTz: string | null,
  payloadKind: string,
  text: string,
  wake: string | null,
): Promise<void> {
  return invoke<void>(COMMANDS.updateAutomation, {
    jobId,
    name,
    scheduleKind,
    scheduleValue,
    scheduleTz,
    payloadKind,
    text,
    wake,
  });
}

/** `set-automation-enabled`: `automations enable|disable <jobId> --json`. */
export async function setAutomationEnabled(jobId: string, enabled: boolean): Promise<void> {
  return invoke<void>(COMMANDS.setAutomationEnabled, { jobId, enabled });
}

/** `delete-automation`: `automations remove <jobId> --json`. */
export async function deleteAutomation(jobId: string): Promise<void> {
  return invoke<void>(COMMANDS.deleteAutomation, { jobId });
}

// --- Phase 8 command wrappers ------------------------------------------------

/** `get-gateway-status`: Phase 1 gateway status (read-only reuse). */
export async function getGatewayStatus(): Promise<GatewayStatus> {
  return invoke<GatewayStatus>(COMMANDS.getGatewayStatus);
}

/** `get-update-status`: update state plus current/latest versions.
 *
 * Fail-soft: an undeterminable state resolves to `state: "unknown"` with no
 * versions (a value, not an error — Phase 1 policy).
 */
export async function getUpdateStatus(): Promise<UpdateStatusDetail> {
  return invoke<UpdateStatusDetail>(COMMANDS.getUpdateStatus);
}

/** `get-agents`: all agent rows (read-only display). */
export async function getAgents(): Promise<AgentRow[]> {
  return invoke<AgentRow[]>(COMMANDS.getAgents);
}

/** `get-logs`: one-shot tail of at most `limit` lines (1..=1000, validated
 * in Rust before any CLI call; never `--follow`). */
export async function getLogs(limit: number): Promise<LogsResult> {
  return invoke<LogsResult>(COMMANDS.getLogs, { limit });
}

// --- Phase 8.1 command wrappers ------------------------------------------------

/** `update-node`: one-shot Node.js update (winget) for an unsupported
 * detected version. Guarded in Rust: an already-supported or missing Node
 * rejects with a stable code and 0 OS mutation. Returns the post-update
 * detection (the existing `NodeDetection` wire type). */
export async function updateNode(): Promise<NodeDetection> {
  return invoke<NodeDetection>(COMMANDS.updateNode);
}
