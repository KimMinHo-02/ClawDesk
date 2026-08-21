/**
 * Unit tests for the pure Phase 3 models logic (contract §3 frontend test):
 * reasoning option enable/disable, supportedReasoningEfforts restriction,
 * error-code → i18n mapping, provider/model form validation.
 */

import { describe, expect, it } from "vitest";
import { getStrings } from "../../i18n/ko";
import type { TauriAppError } from "../../lib/tauri";
import {
  API_TYPES,
  THINKING_LEVELS,
  mapModelsError,
  modelSupportsReasoning,
  providerDetailToForm,
  providerFormToInput,
  reasoningOptionsFor,
  validateProviderForm,
  emptyProviderForm,
  isBaseUrlValid,
} from "./modelsState";

const errors = getStrings("models").errors;

// --- reasoning options ---------------------------------------------------------

describe("reasoningOptionsFor", () => {
  it("disables everything when no default model is set", () => {
    const options = reasoningOptionsFor(null);
    expect(options).toHaveLength(9);
    expect(options.every((option) => !option.enabled)).toBe(true);
  });

  it("disables everything for a non-reasoning model (fail-closed)", () => {
    const options = reasoningOptionsFor({ reasoning: false });
    expect(options.every((option) => !option.enabled)).toBe(true);
  });

  it("treats an absent capability as non-reasoning (fail-closed)", () => {
    const options = reasoningOptionsFor(undefined);
    expect(options.every((option) => !option.enabled)).toBe(true);
    expect(modelSupportsReasoning(undefined)).toBe(false);
  });

  it("exposes the full ladder when reasoning is supported without a list", () => {
    const options = reasoningOptionsFor({ reasoning: true });
    expect(options.every((option) => option.enabled)).toBe(true);
    expect(options.map((option) => option.id)).toEqual(
      THINKING_LEVELS.map((level) => level.id),
    );
  });

  it("restricts options to supportedReasoningEfforts plus off", () => {
    const options = reasoningOptionsFor({
      reasoning: true,
      supportedReasoningEfforts: ["low", "high"],
    });
    const enabled = options.filter((option) => option.enabled).map((o) => o.id);
    expect(enabled).toEqual(["off", "low", "high"]);
  });

  it("keeps off selectable even when restricted", () => {
    const options = reasoningOptionsFor({
      reasoning: true,
      supportedReasoningEfforts: ["xhigh"],
    });
    const off = options.find((option) => option.id === "off");
    expect(off?.enabled).toBe(true);
  });
});

// --- form validation -------------------------------------------------------------

function validForm() {
  return emptyProviderForm();
}

describe("validateProviderForm", () => {
  it("accepts a minimal valid form", () => {
    const form = validForm();
    form.id = "acme";
    form.models[0].id = "m1";
    expect(validateProviderForm(form)).toBeNull();
  });

  it("rejects provider ids with traversal or bad characters", () => {
    for (const bad of ["../evil", "a/b", "a:b", "a b", ".hidden", "-dash", ""]) {
      const form = validForm();
      form.id = bad;
      form.models[0].id = "m1";
      expect(validateProviderForm(form)).toBe("providerIdInvalid");
    }
  });

  it("rejects ids over 128 characters and with dot-dot", () => {
    const form = validForm();
    form.id = "a".repeat(129);
    form.models[0].id = "m1";
    expect(validateProviderForm(form)).toBe("providerIdInvalid");
    form.id = "a..b";
    expect(validateProviderForm(form)).toBe("providerIdInvalid");
  });

  it("requires absolute http(s) base urls", () => {
    expect(isBaseUrlValid("")).toBe(true);
    expect(isBaseUrlValid("https://api.acme.test/v1")).toBe(true);
    expect(isBaseUrlValid("http://localhost:8080")).toBe(true);
    expect(isBaseUrlValid("ftp://x.test")).toBe(false);
    expect(isBaseUrlValid("https://")).toBe(false);
    expect(isBaseUrlValid("https://bad url.test")).toBe(false);
  });

  it("rejects unknown api types", () => {
    const form = validForm();
    form.id = "acme";
    form.models[0].id = "m1";
    form.api = "not-an-api";
    expect(validateProviderForm(form)).toBe("apiInvalid");
    expect(API_TYPES).toContain("openai-completions");
  });

  it("requires at least one model and valid model ids", () => {
    const form = validForm();
    form.id = "acme";
    form.models = [];
    expect(validateProviderForm(form)).toBe("modelRequired");

    form.models = [
      {
        id: "../x",
        name: "",
        reasoning: false,
        input: ["text"],
        contextWindow: "",
        maxTokens: "",
        supportsReasoningEffort: false,
        supportedReasoningEfforts: [],
      },
    ];
    expect(validateProviderForm(form)).toBe("modelIdInvalid");
  });

  it("rejects empty or invalid input modalities", () => {
    const form = validForm();
    form.id = "acme";
    form.models[0].id = "m1";
    form.models[0].input = [];
    expect(validateProviderForm(form)).toBe("modelInputInvalid");
    form.models[0].input = ["audio"];
    expect(validateProviderForm(form)).toBe("modelInputInvalid");
    form.models[0].input = ["text", "image"];
    expect(validateProviderForm(form)).toBeNull();
  });

  it("rejects non-positive numbers for context window / max tokens", () => {
    const form = validForm();
    form.id = "acme";
    form.models[0].id = "m1";
    form.models[0].contextWindow = "0";
    expect(validateProviderForm(form)).toBe("modelNumberInvalid");
    form.models[0].contextWindow = "12.5";
    expect(validateProviderForm(form)).toBe("modelNumberInvalid");
    form.models[0].contextWindow = "128000";
    form.models[0].maxTokens = "";
    expect(validateProviderForm(form)).toBeNull();
  });

  it("requires a non-empty effort list when supportsReasoningEffort is on", () => {
    const form = validForm();
    form.id = "acme";
    form.models[0].id = "m1";
    form.models[0].reasoning = true;
    form.models[0].supportsReasoningEffort = true;
    expect(validateProviderForm(form)).toBe("effortsRequireReasoning");
    form.models[0].supportedReasoningEfforts = ["low", "high"];
    expect(validateProviderForm(form)).toBeNull();
  });
});

// --- form conversion -----------------------------------------------------------------

describe("providerFormToInput / providerDetailToForm", () => {
  it("converts a form into the save-provider payload", () => {
    const form = emptyProviderForm();
    form.id = "acme";
    form.baseUrl = "https://api.acme.test/v1";
    form.api = "openai-completions";
    form.models[0].id = "m1";
    form.models[0].name = "M1";
    form.models[0].reasoning = true;
    form.models[0].contextWindow = "128000";
    form.models[0].supportsReasoningEffort = true;
    form.models[0].supportedReasoningEfforts = ["low", "high"];

    const input = providerFormToInput(form);
    expect(input.id).toBe("acme");
    expect(input.baseUrl).toBe("https://api.acme.test/v1");
    expect(input.api).toBe("openai-completions");
    expect(input.models).toEqual([
      {
        id: "m1",
        name: "M1",
        reasoning: true,
        input: ["text"],
        contextWindow: 128000,
        maxTokens: undefined,
        supportsReasoningEffort: true,
        supportedReasoningEfforts: ["low", "high"],
      },
    ]);
  });

  it("omits empty optional fields in the payload", () => {
    const form = emptyProviderForm();
    form.id = "acme";
    form.models[0].id = "m1";
    const input = providerFormToInput(form);
    expect(input.baseUrl).toBeUndefined();
    expect(input.models?.[0]?.name).toBeUndefined();
    expect(input.models?.[0]?.contextWindow).toBeUndefined();
  });

  it("round-trips a provider detail into an editable form", () => {
    const form = providerDetailToForm({
      id: "acme",
      baseUrl: "https://api.acme.test/v1",
      api: "anthropic-messages",
      models: [
        {
          id: "m1",
          reasoning: true,
          input: ["text"],
          contextWindow: 200000,
          compat: {
            supportsReasoningEffort: true,
            supportedReasoningEfforts: ["high", "xhigh"],
          },
        },
      ],
    });
    expect(form.id).toBe("acme");
    expect(form.models).toHaveLength(1);
    expect(form.models[0].contextWindow).toBe("200000");
    expect(form.models[0].supportsReasoningEffort).toBe(true);
    expect(form.models[0].supportedReasoningEfforts).toEqual(["high", "xhigh"]);
    expect(validateProviderForm(form)).toBeNull();
  });
});

// --- error mapping --------------------------------------------------------------------

describe("mapModelsError", () => {
  it("maps known stable codes", () => {
    const cases: Record<string, TauriAppError> = {
      "provider-id-invalid": { code: "provider-id-invalid", message: "x" },
      "model-id-invalid": { code: "model-id-invalid", message: "x" },
      "thinking-level-invalid": { code: "thinking-level-invalid", message: "x" },
      "openclaw-config-read-failed": {
        code: "openclaw-config-read-failed",
        message: "x",
      },
      "openclaw-config-write-failed": {
        code: "openclaw-config-write-failed",
        message: "x",
      },
      "openclaw-config-invalid": { code: "openclaw-config-invalid", message: "x" },
      "secret-store-unavailable": { code: "secret-store-unavailable", message: "x" },
      "secret-ref-registration-failed": {
        code: "secret-ref-registration-failed",
        message: "x",
      },
      "openclaw-not-found": { code: "openclaw-not-found", message: "x" },
      "process-timeout": { code: "process-timeout", message: "x" },
      "process-failed": { code: "process-failed", message: "x" },
    };
    for (const [code, error] of Object.entries(cases)) {
      expect(mapModelsError(error)).toBe(code);
      // Every mapped code has a Korean message in the i18n models namespace.
      expect(errors[code as keyof typeof errors]).toBeTruthy();
    }
  });

  it("falls back for unknown codes and always has a Korean fallback", () => {
    expect(mapModelsError({ code: "something-else", message: "x" })).toBe(
      "fallback",
    );
    expect(errors.fallback).toBeTruthy();
  });
});
