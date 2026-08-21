/**
 * Pure logic for the Phase 3 models feature (UI state, validation,
 * capability-based reasoning options, error mapping). No Tauri calls here —
 * keep this unit-testable without the IPC layer.
 */

import {
  type ModelEntry,
  type ModelRow,
  type ThinkingLevel,
  type TauriAppError,
  type ProviderInput,
} from "../../lib/tauri";

/** The standard 9-step thinking ladder in display order, with Korean labels. */
export const THINKING_LEVELS: ReadonlyArray<{ id: ThinkingLevel; label: string }> = [
  { id: "off", label: "끄기" },
  { id: "minimal", label: "최소" },
  { id: "low", label: "낮음" },
  { id: "medium", label: "보통" },
  { id: "high", label: "높음" },
  { id: "xhigh", label: "매우 높음" },
  { id: "adaptive", label: "적응형" },
  { id: "max", label: "최대" },
  { id: "ultra", label: "울트라" },
];

/** Known provider `api` types (mirrors the Rust `KNOWN_API_TYPES`). */
export const API_TYPES: readonly string[] = [
  "openai-completions",
  "openai-responses",
  "openai-chatgpt-responses",
  "anthropic-messages",
  "google-generative-ai",
  "google-vertex",
  "github-copilot",
  "bedrock-converse-stream",
  "ollama",
  "azure-openai-responses",
];

/** Input modalities a model entry accepts. */
export const INPUT_MODALITIES: readonly string[] = ["text", "image"];

export interface ReasoningOption {
  id: ThinkingLevel;
  label: string;
  /** Whether the option is selectable for the given model capability. */
  enabled: boolean;
}

/**
 * Reasoning options for the global thinking default, given the capability of
 * the current default model (contract §3, fail-closed):
 *
 * - no model / `reasoning: false` (or absent) → everything disabled;
 * - `supportedReasoningEfforts` present → that set (plus `off`) enabled;
 * - reasoning supported without a list → the full standard ladder enabled
 *   (OpenClaw auto-remaps unsupported levels — documented behavior).
 */
export function reasoningOptionsFor(
  model:
    | {
        reasoning: boolean;
        supportedReasoningEfforts?: ThinkingLevel[];
      }
    | null
    | undefined,
): ReasoningOption[] {
  const supported = Boolean(model?.reasoning);
  const efforts = supported ? (model?.supportedReasoningEfforts ?? []) : [];
  return THINKING_LEVELS.map(({ id, label }) => {
    let enabled = false;
    if (supported) {
      if (efforts.length > 0) {
        enabled = id === "off" || efforts.includes(id);
      } else {
        enabled = true;
      }
    }
    return { id, label, enabled };
  });
}

/** Whether the given model supports reasoning effort selection at all. */
export function modelSupportsReasoning(
  model:
    | { reasoning: boolean; supportedReasoningEfforts?: ThinkingLevel[] }
    | null
    | undefined,
): boolean {
  return Boolean(model?.reasoning);
}

// --- form state -----------------------------------------------------------------

/** A model row while editing in the provider form. */
export interface ModelForm {
  id: string;
  name: string;
  reasoning: boolean;
  input: string[];
  contextWindow: string;
  maxTokens: string;
  supportsReasoningEffort: boolean;
  supportedReasoningEfforts: ThinkingLevel[];
}

export interface ProviderForm {
  id: string;
  baseUrl: string;
  api: string;
  models: ModelForm[];
}

/** Creates an empty model form row. */
export function emptyModelForm(): ModelForm {
  return {
    id: "",
    name: "",
    reasoning: false,
    input: ["text"],
    contextWindow: "",
    maxTokens: "",
    supportsReasoningEffort: false,
    supportedReasoningEfforts: [],
  };
}

/** Creates an empty provider form (new provider). */
export function emptyProviderForm(): ProviderForm {
  return { id: "", baseUrl: "", api: API_TYPES[0], models: [emptyModelForm()] };
}

/** Converts a `get-provider` detail into an editable form. */
export function providerDetailToForm(detail: {
  id: string;
  baseUrl?: string;
  api?: string;
  models: ModelEntry[];
}): ProviderForm {
  return {
    id: detail.id,
    baseUrl: detail.baseUrl ?? "",
    api: detail.api ?? API_TYPES[0],
    models: detail.models.map((model) => ({
      id: model.id,
      name: model.name ?? "",
      reasoning: model.reasoning,
      input: model.input.length > 0 ? [...model.input] : ["text"],
      contextWindow: model.contextWindow?.toString() ?? "",
      maxTokens: model.maxTokens?.toString() ?? "",
      supportsReasoningEffort: model.compat?.supportsReasoningEffort ?? false,
      supportedReasoningEfforts: model.compat?.supportedReasoningEfforts ?? [],
    })),
  };
}

/** Id rules: alphanumeric start, then `[A-Za-z0-9._-]`, ≤128, no `..`. */
export function isEntryIdValid(id: string): boolean {
  if (id.length === 0 || id.length > 128) {
    return false;
  }
  if (!/^[A-Za-z0-9]/.test(id)) {
    return false;
  }
  if (id.includes("..")) {
    return false;
  }
  return /^[A-Za-z0-9._-]+$/.test(id);
}

/** baseUrl rules: absolute http(s) URL with a non-empty host and no
 * whitespace/control characters (mirrors the Rust `validate_base_url`). */
export function isBaseUrlValid(url: string): boolean {
  if (url === "") {
    return true; // baseUrl is optional.
  }
  const host = /^(http|https):\/\/([^/\s?#]+)/.exec(url)?.[2];
  if (host === undefined || host === "") {
    return false;
  }
  return [...url].every((ch) => {
    const code = ch.charCodeAt(0);
    return code >= 0x21 && code <= 0x7e;
  });
}

/** Non-empty number-or-empty text field. */
function isOptionalNumber(value: string): boolean {
  if (value === "") {
    return true;
  }
  const n = Number(value);
  return Number.isInteger(n) && n > 0;
}

export type FormErrorKey =
  | "providerIdInvalid"
  | "baseUrlInvalid"
  | "apiInvalid"
  | "modelIdInvalid"
  | "modelInputInvalid"
  | "modelNumberInvalid"
  | "effortsRequireReasoning"
  | "modelRequired";

/**
 * Validates a provider form. Returns `null` when valid, otherwise the first
 * error key (the UI maps it to a Korean message via i18n).
 */
export function validateProviderForm(form: ProviderForm): FormErrorKey | null {
  if (!isEntryIdValid(form.id)) {
    return "providerIdInvalid";
  }
  if (!isBaseUrlValid(form.baseUrl)) {
    return "baseUrlInvalid";
  }
  if (!API_TYPES.includes(form.api)) {
    return "apiInvalid";
  }
  if (form.models.length === 0) {
    return "modelRequired";
  }
  for (const model of form.models) {
    if (!isEntryIdValid(model.id)) {
      return "modelIdInvalid";
    }
    if (
      model.input.length === 0 ||
      !model.input.every((m) => INPUT_MODALITIES.includes(m))
    ) {
      return "modelInputInvalid";
    }
    if (!isOptionalNumber(model.contextWindow) || !isOptionalNumber(model.maxTokens)) {
      return "modelNumberInvalid";
    }
    if (
      model.supportsReasoningEffort &&
      model.supportedReasoningEfforts.length === 0
    ) {
      return "effortsRequireReasoning";
    }
  }
  return null;
}

/** Converts a valid form into the `save-provider` payload. */
export function providerFormToInput(form: ProviderForm): ProviderInput {
  return {
    id: form.id,
    baseUrl: form.baseUrl === "" ? undefined : form.baseUrl,
    api: form.api,
    models: form.models.map((model) => ({
      id: model.id,
      name: model.name === "" ? undefined : model.name,
      reasoning: model.reasoning,
      input: model.input,
      contextWindow: model.contextWindow === "" ? undefined : Number(model.contextWindow),
      maxTokens: model.maxTokens === "" ? undefined : Number(model.maxTokens),
      supportsReasoningEffort: model.supportsReasoningEffort,
      supportedReasoningEfforts: model.supportsReasoningEffort
        ? model.supportedReasoningEfforts
        : [],
    })),
  };
}

/**
 * Finds the current default model row (by `provider/model` full ref) to
 * drive the reasoning selector capability.
 */
export function findDefaultModelRow(
  rows: ModelRow[],
  defaultRef: string | null,
): ModelRow | null {
  if (defaultRef === null) {
    return null;
  }
  return rows.find((row) => row.full === defaultRef) ?? null;
}

// --- error mapping -----------------------------------------------------------------

/** Error codes the models feature can receive (stable, from the Rust layer). */
export const MODELS_ERROR_CODES = [
  "provider-id-invalid",
  "model-id-invalid",
  "thinking-level-invalid",
  "openclaw-config-read-failed",
  "openclaw-config-write-failed",
  "openclaw-config-invalid",
  "secret-store-unavailable",
  "secret-ref-registration-failed",
  "openclaw-not-found",
  "process-timeout",
  "process-failed",
] as const;

export type ModelsErrorCode = (typeof MODELS_ERROR_CODES)[number];

/**
 * Maps an IPC `AppError` to an i18n key. Unknown codes fall back so the UI
 * can always show a message (the fallback message is generic Korean).
 */
export function mapModelsError(error: TauriAppError): ModelsErrorCode | "fallback" {
  return (MODELS_ERROR_CODES as readonly string[]).includes(error.code)
    ? (error.code as ModelsErrorCode)
    : "fallback";
}
