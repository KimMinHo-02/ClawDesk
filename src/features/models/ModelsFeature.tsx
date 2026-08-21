/**
 * Models feature (Phase 3): provider/model CRUD, API key registration
 * (DPAPI-only), default model, and the global reasoning default.
 *
 * All OS/OpenClaw work goes through the Tauri IPC wrappers
 * (`src/lib/tauri`) — the component never touches processes (S1/S10).
 * API key values are one-way: entered once, masked immediately, never
 * displayed again (S3/S7).
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { getStrings } from "../../i18n/ko";
import {
  type ApiKeyStatus,
  type ModelRow,
  type ProviderDetail,
  type ProviderInput,
  type ProviderSummary,
  type ThinkingLevel,
  deleteProvider,
  deleteProviderApiKey,
  getProvider,
  listApiKeys,
  listModels,
  listProviders,
  saveProvider,
  setProviderApiKey,
  normalizeAppError,
  type TauriAppError,
  getDefaultModel,
  getReasoningDefault,
  setDefaultModel,
  setReasoningDefault,
} from "../../lib/tauri";
import {
  API_TYPES,
  INPUT_MODALITIES,
  THINKING_LEVELS,
  type FormErrorKey,
  type ProviderForm,
  emptyModelForm,
  emptyProviderForm,
  findDefaultModelRow,
  mapModelsError,
  modelSupportsReasoning,
  providerDetailToForm,
  providerFormToInput,
  reasoningOptionsFor,
  validateProviderForm,
} from "./modelsState";

const t = getStrings("models");
const common = getStrings("common");

/** Maps a form error key to its Korean message. */
function formErrorText(key: FormErrorKey): string {
  const map: Record<FormErrorKey, string> = {
    providerIdInvalid: t.errors["provider-id-invalid"],
    baseUrlInvalid: t.errors["provider-id-invalid"],
    apiInvalid: t.errors["provider-id-invalid"],
    modelIdInvalid: t.errors["model-id-invalid"],
    modelInputInvalid: t.errors["model-id-invalid"],
    modelNumberInvalid: t.errors["model-id-invalid"],
    effortsRequireReasoning: t.errors["model-id-invalid"],
    modelRequired: t.errors["model-id-invalid"],
  };
  return map[key];
}

/** Maps an IPC rejection to its Korean message (stable code based). */
function errorText(err: unknown): string {
  const appError: TauriAppError = normalizeAppError(err);
  return t.errors[mapModelsError(appError)];
}

export function ModelsFeature() {
  const [providers, setProviders] = useState<ProviderSummary[] | null>(null);
  const [rows, setRows] = useState<ModelRow[] | null>(null);
  const [defaultModel, setDefaultModelRef] = useState<string | null>(null);
  const [reasoningDefault, setReasoningDefaultState] =
    useState<ThinkingLevel | null>(null);
  const [keyStatus, setKeyStatus] = useState<ApiKeyStatus[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Provider editor state.
  const [editing, setEditing] = useState<ProviderForm | null>(null);
  const [editingIsNew, setEditingIsNew] = useState(false);
  const [formError, setFormError] = useState<FormErrorKey | null>(null);

  // API key dialog state (provider id + one-way key value).
  const [keyTarget, setKeyTarget] = useState<string | null>(null);
  const [keyValue, setKeyValue] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);

  const busyRef = useRef(false);
  const keyBusyRef = useRef(false);

  const isKeyRegistered = useCallback(
    (providerId: string) =>
      keyStatus?.some((status) => status.providerId === providerId && status.registered) ??
      false,
    [keyStatus],
  );

  const reload = useCallback(async () => {
    setError(null);
    try {
      const [providerList, modelRows, defaultRef, reasoning, keys] =
        await Promise.all([
          listProviders(),
          listModels(),
          getDefaultModel(),
          getReasoningDefault(),
          listApiKeys(),
        ]);
      setProviders(providerList);
      setRows(modelRows);
      setDefaultModelRef(defaultRef);
      setReasoningDefaultState(reasoning);
      setKeyStatus(keys);
    } catch (err) {
      setError(errorText(err));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // --- provider editor -----------------------------------------------------

  const openNew = useCallback(() => {
    setFormError(null);
    setEditingIsNew(true);
    setEditing(emptyProviderForm());
  }, []);

  const openEdit = useCallback(async (providerId: string) => {
    setFormError(null);
    try {
      const detail: ProviderDetail = await getProvider(providerId);
      setEditingIsNew(false);
      setEditing(providerDetailToForm(detail));
    } catch (err) {
      setError(errorText(err));
    }
  }, []);

  const closeEditor = useCallback(() => {
    setEditing(null);
    setFormError(null);
  }, []);

  const saveForm = useCallback(async () => {
    if (!editing || busyRef.current) {
      return;
    }
    const invalid = validateProviderForm(editing);
    if (invalid !== null) {
      setFormError(invalid);
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setFormError(null);
    try {
      const payload: ProviderInput = providerFormToInput(editing);
      await saveProvider(payload);
      setEditing(null);
      await reload();
    } catch (err) {
      setError(errorText(err));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }, [editing, reload]);

  const removeProvider = useCallback(
    async (providerId: string) => {
      if (busyRef.current) {
        return;
      }
      // eslint-disable-next-line no-alert -- confirmation before destructive action
      if (!window.confirm(`${t.deleteProviderConfirm} (${providerId})`)) {
        return;
      }
      busyRef.current = true;
      setBusy(true);
      setError(null);
      try {
        await deleteProvider(providerId);
        await reload();
      } catch (err) {
        setError(errorText(err));
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
    },
    [reload],
  );

  const updateModel = useCallback(
    (index: number, patch: Partial<ProviderForm["models"][number]>) => {
      setEditing((current) =>
        current === null
          ? current
          : {
              ...current,
              models: current.models.map((model, i) =>
                i === index ? { ...model, ...patch } : model,
              ),
            },
      );
    },
    [],
  );

  // --- default model / reasoning default -------------------------------------

  const applyDefaultModel = useCallback(
    async (modelRef: string) => {
      if (busyRef.current || modelRef === "") {
        return;
      }
      busyRef.current = true;
      setBusy(true);
      setError(null);
      try {
        await setDefaultModel(modelRef);
        await reload();
      } catch (err) {
        setError(errorText(err));
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
    },
    [reload],
  );

  const applyReasoningDefault = useCallback(
    async (level: ThinkingLevel) => {
      if (busyRef.current) {
        return;
      }
      busyRef.current = true;
      setBusy(true);
      setError(null);
      try {
        await setReasoningDefault(level);
        await reload();
      } catch (err) {
        setError(errorText(err));
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
    },
    [reload],
  );

  // --- API key -----------------------------------------------------------------

  const openKeyDialog = useCallback((providerId: string) => {
    setKeyValue("");
    setKeyTarget(providerId);
  }, []);

  const closeKeyDialog = useCallback(() => {
    // Never retain the entered value in state after close.
    setKeyValue("");
    setKeyTarget(null);
  }, []);

  const registerKey = useCallback(async () => {
    if (!keyTarget || keyBusyRef.current) {
      return;
    }
    if (keyValue.trim() === "") {
      return;
    }
    keyBusyRef.current = true;
    setKeyBusy(true);
    setError(null);
    try {
      await setProviderApiKey(keyTarget, keyValue);
      // The value is one-way: clear immediately, never display again (S3).
      setKeyValue("");
      setKeyTarget(null);
      await reload();
    } catch (err) {
      setError(errorText(err));
    } finally {
      keyBusyRef.current = false;
      setKeyBusy(false);
    }
  }, [keyTarget, keyValue, reload]);

  const removeKey = useCallback(
    async (providerId: string) => {
      // eslint-disable-next-line no-alert -- confirmation before destructive action
      if (!window.confirm(`${t.apiKeyDeleteConfirm} (${providerId})`)) {
        return;
      }
      if (keyBusyRef.current) {
        return;
      }
      keyBusyRef.current = true;
      setKeyBusy(true);
      setError(null);
      try {
        await deleteProviderApiKey(providerId);
        await reload();
      } catch (err) {
        setError(errorText(err));
      } finally {
        keyBusyRef.current = false;
        setKeyBusy(false);
      }
    },
    [reload],
  );

  // --- render ----------------------------------------------------------------------

  if (providers === null || rows === null || keyStatus === null) {
    return (
      <section>
        <h2>{t.title}</h2>
        <p>{t.loading}</p>
        {error !== null && <p role="alert">{error}</p>}
      </section>
    );
  }

  const defaultRow = findDefaultModelRow(rows, defaultModel);
  const reasoningEnabled = modelSupportsReasoning(defaultRow);
  const reasoningOptions = reasoningOptionsFor(defaultRow);

  return (
    <section>
      <h2>{t.title}</h2>
      {error !== null && <p role="alert">{error}</p>}

      {/* Providers */}
      <h3>{t.providers}</h3>
      {providers.length === 0 ? (
        <p>{t.noProviders}</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>{t.providerId}</th>
              <th>{t.baseUrl}</th>
              <th>{t.apiType}</th>
              <th>{t.modelCount}</th>
              <th>{t.apiKey}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {providers.map((provider) => (
              <tr key={provider.id}>
                <td>{provider.id}</td>
                <td>{provider.baseUrl ?? "—"}</td>
                <td>{provider.api ?? "—"}</td>
                <td>{provider.modelCount}</td>
                <td>
                  {isKeyRegistered(provider.id) ? t.apiKeyRegistered : t.apiKeyUnregistered}
                </td>
                <td>
                  <button type="button" disabled={busy} onClick={() => void openEdit(provider.id)}>
                    {t.editProvider}
                  </button>{" "}
                  <button type="button" disabled={busy} onClick={() => void removeProvider(provider.id)}>
                    {common.delete}
                  </button>{" "}
                  <button type="button" disabled={keyBusy} onClick={() => openKeyDialog(provider.id)}>
                    {t.apiKeyRegister}
                  </button>{" "}
                  <button
                    type="button"
                    disabled={keyBusy || !isKeyRegistered(provider.id)}
                    onClick={() => void removeKey(provider.id)}
                  >
                    {common.delete}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <button type="button" disabled={busy} onClick={openNew}>
        {t.addProvider}
      </button>

      {/* Default model */}
      <h3>{t.defaultModel}</h3>
      {defaultModel === null ? (
        <p>{t.defaultModelNone}</p>
      ) : (
        <p>{defaultModel}</p>
      )}
      <label>
        {t.setDefault}{" "}
        <select
          disabled={busy}
          value={defaultModel ?? ""}
          onChange={(event) => void applyDefaultModel(event.target.value)}
        >
          <option value="">{t.defaultModelNone}</option>
          {rows.map((row) => (
            <option key={row.full} value={row.full}>
              {row.full}
            </option>
          ))}
        </select>
      </label>

      {/* Reasoning default */}
      <h3>{t.reasoning}</h3>
      {reasoningDefault === null ? <p>{t.reasoningNone}</p> : <p>{reasoningDefault}</p>}
      {!reasoningEnabled && (
        <p role="note">{t.reasoningDisabledNote}</p>
      )}
      <label>
        {t.reasoning}{" "}
        <select
          disabled={busy || !reasoningEnabled}
          value={reasoningDefault ?? ""}
          onChange={(event) =>
            event.target.value !== "" &&
            void applyReasoningDefault(event.target.value as ThinkingLevel)
          }
        >
          <option value="">{t.reasoningNone}</option>
          {reasoningOptions.map((option) => (
            <option key={option.id} value={option.id} disabled={!option.enabled}>
              {option.label} ({option.id})
            </option>
          ))}
        </select>
      </label>

      {/* Provider editor */}
      {editing !== null && (
        <div>
          <h3>{editingIsNew ? t.addProvider : `${t.editProvider}: ${editing.id}`}</h3>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void saveForm();
            }}
          >
            <label>
              {t.providerId}{" "}
              <input
                value={editing.id}
                disabled={!editingIsNew || busy}
                placeholder={t.providerIdHint}
                onChange={(event) =>
                  setEditing({ ...editing, id: event.target.value })
                }
              />
            </label>
            <label>
              {t.baseUrl}{" "}
              <input
                value={editing.baseUrl}
                disabled={busy}
                placeholder={t.baseUrlHint}
                onChange={(event) =>
                  setEditing({ ...editing, baseUrl: event.target.value })
                }
              />
            </label>
            <label>
              {t.apiType}{" "}
              <select
                value={editing.api}
                disabled={busy}
                onChange={(event) =>
                  setEditing({ ...editing, api: event.target.value })
                }
              >
                {API_TYPES.map((apiType) => (
                  <option key={apiType} value={apiType}>
                    {apiType}
                  </option>
                ))}
              </select>
            </label>

            {editing.models.map((model, index) => (
              <fieldset key={index}>
                <legend>{`${t.modelId} ${index + 1}`}</legend>
                <label>
                  {t.modelId}{" "}
                  <input
                    value={model.id}
                    disabled={busy}
                    onChange={(event) =>
                      updateModel(index, { id: event.target.value })
                    }
                  />
                </label>
                <label>
                  {t.modelName}{" "}
                  <input
                    value={model.name}
                    disabled={busy}
                    onChange={(event) =>
                      updateModel(index, { name: event.target.value })
                    }
                  />
                </label>
                <label>
                  <input
                    type="checkbox"
                    checked={model.reasoning}
                    disabled={busy}
                    onChange={(event) =>
                      updateModel(index, { reasoning: event.target.checked })
                    }
                  />{" "}
                  {t.modelReasoning}
                </label>
                <span>
                  {t.modelInput}{" "}
                  {INPUT_MODALITIES.map((modality) => (
                    <label key={modality}>
                      <input
                        type="checkbox"
                        checked={model.input.includes(modality)}
                        disabled={busy}
                        onChange={(event) =>
                          updateModel(index, {
                            input: event.target.checked
                              ? [...model.input, modality]
                              : model.input.filter((m) => m !== modality),
                          })
                        }
                      />{" "}
                      {modality}
                    </label>
                  ))}
                </span>
                <label>
                  {t.contextWindow}{" "}
                  <input
                    value={model.contextWindow}
                    disabled={busy}
                    inputMode="numeric"
                    onChange={(event) =>
                      updateModel(index, { contextWindow: event.target.value })
                    }
                  />
                </label>
                <label>
                  {t.maxTokens}{" "}
                  <input
                    value={model.maxTokens}
                    disabled={busy}
                    inputMode="numeric"
                    onChange={(event) =>
                      updateModel(index, { maxTokens: event.target.value })
                    }
                  />
                </label>
                <label>
                  <input
                    type="checkbox"
                    checked={model.supportsReasoningEffort}
                    disabled={busy}
                    onChange={(event) =>
                      updateModel(index, {
                        supportsReasoningEffort: event.target.checked,
                      })
                    }
                  />{" "}
                  {t.supportsEffort}
                </label>
                {model.reasoning && model.supportsReasoningEffort && (
                  <span>
                    {t.supportedEfforts}{" "}
                    {THINKING_LEVELS.filter(({ id }) => id !== "off").map(({ id, label }) => (
                      <label key={id}>
                        <input
                          type="checkbox"
                          checked={model.supportedReasoningEfforts.includes(id)}
                          disabled={busy}
                          onChange={(event) =>
                            updateModel(index, {
                              supportedReasoningEfforts: event.target.checked
                                ? [
                                    ...model.supportedReasoningEfforts,
                                    id,
                                  ]
                                : model.supportedReasoningEfforts.filter(
                                    (e) => e !== id,
                                  ),
                            })
                          }
                        />{" "}
                        {label} ({id})
                      </label>
                    ))}
                  </span>
                )}
                <button
                  type="button"
                  disabled={busy}
                  onClick={() =>
                    setEditing({
                      ...editing,
                      models: editing.models.filter((_, i) => i !== index),
                    })
                  }
                >
                  {common.delete}
                </button>
              </fieldset>
            ))}
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                setEditing({
                  ...editing,
                  models: [...editing.models, emptyModelForm()],
                })
              }
            >
              {t.addModel}
            </button>
            {formError !== null && <p role="alert">{formErrorText(formError)}</p>}
            <button type="submit" disabled={busy}>
              {busy ? t.saving : common.save}
            </button>{" "}
            <button type="button" disabled={busy} onClick={closeEditor}>
              {common.cancel}
            </button>
          </form>
        </div>
      )}

      {/* API key dialog */}
      {keyTarget !== null && (
        <div>
          <h3>{`${t.apiKey}: ${keyTarget}`}</h3>
          <p>{t.apiKeyRegisterHint}</p>
          <label>
            API Key{" "}
            <input
              type="password"
              value={keyValue}
              autoComplete="new-password"
              disabled={keyBusy}
              onChange={(event) => setKeyValue(event.target.value)}
            />
          </label>
          <button type="button" disabled={keyBusy || keyValue.trim() === ""} onClick={() => void registerKey()}>
            {keyBusy ? t.saving : t.apiKeyRegister}
          </button>{" "}
          <button type="button" disabled={keyBusy} onClick={closeKeyDialog}>
            {common.cancel}
          </button>
        </div>
      )}
    </section>
  );
}
