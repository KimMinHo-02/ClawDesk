/**
 * Automations feature (Phase 7): job list/detail, create, edit, enable
 * toggle, and delete. Manual execution (`automations run`/`runs`) is a
 * non-goal and is never offered.
 *
 * All OS/OpenClaw work goes through the Tauri IPC wrappers (`src/lib/tauri`)
 * — the component never touches processes (S1/S10). No optimistic updates:
 * after every finished mutation (success or failure) the actual state is
 * re-queued. One mutation runs at a time (duplicate-submit guard).
 *
 * The session field never crosses the wire — the Rust layer fixes the
 * reminder/task pairing. The payload kind cannot change on edit (kind
 * change = delete + recreate, blocked here).
 */

import { useCallback, useEffect, useReducer, useState } from "react";
import { getStrings } from "../../i18n/ko";
import {
  WAKE_VALUES,
  createAutomation,
  deleteAutomation,
  getAutomations,
  normalizeAppError,
  setAutomationEnabled,
  updateAutomation,
  type AutomationJobRow,
  type TauriAppError,
} from "../../lib/tauri";
import {
  draftFromJob,
  draftScheduleTz,
  draftScheduleValue,
  draftWake,
  emptyAutomationDraft,
  initialAutomationsListState,
  initialAutomationsOpState,
  isDraftValid,
  isPayloadKindChanged,
  isValidAutomationName,
  isValidSchedule,
  mapAutomationsError,
  type AutomationDraft,
  type AutomationEditor,
  automationsListReducer,
  automationsOpReducer,
  payloadSummary,
  scheduleSummary,
} from "./automationsState";

const t = getStrings("automations");

/** Maps an IPC rejection to its Korean message (stable code based). */
function errorText(err: unknown): string {
  const appError: TauriAppError = normalizeAppError(err);
  return t.errors[mapAutomationsError(appError)];
}

/** A short `nextRunAtMs` display (fail-soft). */
function formatNextRun(ms: number | null): string {
  if (ms === null) {
    return t.nextRunUnknown;
  }
  return new Date(ms).toLocaleString();
}

/** Pinpoints the schedule sub-field issue for an inline hint (best effort). */
function scheduleHint(draft: AutomationDraft): string | null {
  if (isValidSchedule(draft.scheduleKind, draftScheduleValue(draft), draftScheduleTz(draft))) {
    return null;
  }
  if (draft.scheduleKind === "at") {
    return t.scheduleInvalidAt;
  }
  if (draft.scheduleKind === "every") {
    return t.scheduleInvalidEvery;
  }
  if (draft.scheduleKind === "cron") {
    return isValidSchedule("cron", draft.scheduleValue, null)
      ? t.scheduleInvalidTz
      : t.scheduleInvalidCron;
  }
  return t.scheduleInvalidCron;
}

/** Pinpoints the payload sub-field issue for an inline hint (best effort). */
function payloadHint(kind: string, text: string, wake: string | null): string | null {
  const trimmed = text.trim();
  if (trimmed.length === 0 || [...trimmed].length > 8000) {
    return t.payloadInvalid;
  }
  if (kind === "reminder" && wake !== null && !(WAKE_VALUES as readonly string[]).includes(wake)) {
    return t.wakeInvalid;
  }
  if (kind === "task" && wake !== null) {
    return t.wakeInvalid;
  }
  return kind === "reminder" || kind === "task" ? null : t.payloadInvalid;
}

export function AutomationsFeature() {
  const [list, dispatchList] = useReducer(automationsListReducer, initialAutomationsListState);
  const [op, dispatchOp] = useReducer(automationsOpReducer, initialAutomationsOpState);
  const [editor, setEditor] = useState<AutomationEditor | null>(null);
  const [draft, setDraft] = useState<AutomationDraft>(emptyAutomationDraft);

  /** Re-queries the actual job list (single-flight, fail-closed). */
  const refresh = useCallback(() => {
    if (list.loading) {
      return;
    }
    dispatchList({ type: "start" });
    getAutomations()
      .then((result) => dispatchList({ type: "finish", jobs: result.jobs, error: null }))
      .catch((err) => dispatchList({ type: "finish", jobs: null, error: errorText(err) }));
  }, [list.loading]);

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mount + explicit re-query
  }, [op.reloadCounter]);

  /** Runs one mutation with the global single-flight guard. */
  const runOp = useCallback(
    (kind: "create" | "update" | "enabled" | "delete", jobId: string | null, run: () => Promise<unknown>) => {
      if (op.pending !== null) {
        return;
      }
      dispatchOp({ type: "start", kind, jobId });
      void run()
        .then(() => dispatchOp({ type: "finish", error: null }))
        .catch((err) => dispatchOp({ type: "finish", error: errorText(err) }));
    },
    [op.pending],
  );

  const openCreate = useCallback(() => {
    setEditor({ jobId: null, originalPayloadKind: null });
    setDraft(emptyAutomationDraft);
  }, []);

  const openEdit = useCallback((row: AutomationJobRow) => {
    setEditor({ jobId: row.id, originalPayloadKind: row.payload?.kind ?? null });
    setDraft(draftFromJob(row));
  }, []);

  const closeEditor = useCallback(() => {
    setEditor(null);
  }, []);

  const formValid = isDraftValid(draft);
  const kindBlocked = editor !== null && isPayloadKindChanged(editor, draft);
  const canSubmit = editor !== null && formValid && !kindBlocked && op.pending === null;

  const submit = useCallback(() => {
    if (!formValid || kindBlocked || editor === null) {
      return;
    }
    const name = draft.name.trim();
    // Wire form: `at` → UTC ISO 8601 (Z), `every` → `Nm`/`Nh`/`Nd`.
    const value = draftScheduleValue(draft);
    const text = draft.text.trim();
    const tz = draftScheduleTz(draft);
    const wake = draftWake(draft);
    if (editor.jobId === null) {
      runOp("create", null, () =>
        createAutomation(name, draft.scheduleKind, value, tz, draft.payloadKind, text, wake),
      );
    } else {
      const jobId = editor.jobId;
      runOp("update", jobId, () =>
        updateAutomation(jobId, name, draft.scheduleKind, value, tz, draft.payloadKind, text, wake),
      );
    }
    setEditor(null);
  }, [formValid, kindBlocked, editor, draft, runOp]);

  const toggle = useCallback(
    (row: AutomationJobRow) => {
      const next = row.enabled === true ? false : true;
      // Disabling (deactivating) needs an explicit confirmation; enabling does not.
      if (next === false) {
        // eslint-disable-next-line no-alert -- confirmation before deactivating
        if (!window.confirm(`${t.disableConfirm} (${row.name ?? row.id})`)) {
          return;
        }
      }
      runOp("enabled", row.id, () => setAutomationEnabled(row.id, next));
    },
    [runOp],
  );

  const remove = useCallback(
    (row: AutomationJobRow) => {
      // eslint-disable-next-line no-alert -- confirmation before destructive action
      if (!window.confirm(`${t.deleteConfirm} (${row.name ?? row.id})`)) {
        return;
      }
      runOp("delete", row.id, () => deleteAutomation(row.id));
    },
    [runOp],
  );

  if (list.loading && list.jobs === null) {
    return (
      <section>
        <h2>{t.title}</h2>
        <p>{t.loading}</p>
        {list.error !== null && <p role="alert">{list.error}</p>}
      </section>
    );
  }

  const jobs = list.jobs ?? [];
  const nameInvalid = editor !== null && draft.name.trim() !== "" && !isValidAutomationName(draft.name);
  const schedHint = editor !== null ? scheduleHint(draft) : null;
  const paylHint =
    editor !== null ? payloadHint(draft.payloadKind, draft.text, draftWake(draft)) : null;
  const isReminder = draft.payloadKind === "reminder";
  const isCron = draft.scheduleKind === "cron";

  return (
    <section>
      <h2>{t.title}</h2>
      <p>{t.hint}</p>
      <p>{t.manualRunNote}</p>
      {list.error !== null && <p role="alert">{list.error}</p>}
      {op.error !== null && <p role="alert">{op.error}</p>}

      {/* --- Job list --- */}
      {jobs.length === 0 ? (
        <p>{t.noJobs}</p>
      ) : (
        <ul>
          {jobs.map((row) => {
            const busy = op.pending !== null;
            const rowBusy = busy && op.pending?.jobId === row.id;
            return (
              <li key={row.id}>
                <p>
                  <strong>{row.name ?? t.nameUnknown}</strong>{" "}
                  <em>({row.id})</em> —{" "}
                  {row.enabled === true ? t.enabled : row.enabled === false ? t.disabled : t.stateUnknown}
                  {" · "}
                  {t.schedule}: {scheduleSummary(row.schedule) ?? t.scheduleUnknown}
                  {" · "}
                  {t.payload}: {payloadSummary(row.payload) ?? t.payloadUnknown}
                  {" · "}
                  {t.status}: {row.status ?? t.stateUnknown}
                  {" · "}
                  {t.nextRun}: {formatNextRun(row.nextRunAtMs)}
                </p>
                <p>
                  <button type="button" disabled={busy} onClick={() => toggle(row)}>
                    {rowBusy && op.pending?.kind === "enabled"
                      ? t.toggling
                      : row.enabled === true
                        ? t.disable
                        : t.enable}
                  </button>{" "}
                  <button type="button" disabled={busy} onClick={() => openEdit(row)}>
                    {t.edit}
                  </button>{" "}
                  <button type="button" disabled={busy} onClick={() => remove(row)}>
                    {rowBusy && op.pending?.kind === "delete" ? t.deleting : t.delete}
                  </button>
                </p>
              </li>
            );
          })}
        </ul>
      )}

      {/* --- Create / edit form --- */}
      {editor === null ? (
        <p>
          <button type="button" disabled={op.pending !== null} onClick={openCreate}>
            {t.add}
          </button>
        </p>
      ) : (
        <div>
          <h3>{editor.jobId === null ? t.createTitle : t.editTitle}</h3>
          <p>
            <label>
              {t.nameLabel}:{" "}
              <input
                aria-label={t.nameLabel}
                value={draft.name}
                disabled={op.pending !== null}
                onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
              />
            </label>{" "}
            <em>{t.nameHint}</em>
          </p>
          {nameInvalid && <p role="alert">{t.nameInvalid}</p>}

          <h4>{t.scheduleLabel}</h4>
          <p>
            <label>
              {t.scheduleKindLabel}:{" "}
              <select
                value={draft.scheduleKind}
                disabled={op.pending !== null}
                onChange={(e) => setDraft((d) => ({ ...d, scheduleKind: e.target.value }))}
              >
                <option value="at">{t.scheduleKindAt}</option>
                <option value="every">{t.scheduleKindEvery}</option>
                <option value="cron">{t.scheduleKindCron}</option>
              </select>
            </label>
          </p>
          {draft.scheduleKind === "at" ? (
            <p>
              <label>
                {t.scheduleValueLabel}:{" "}
                <input
                  aria-label={t.scheduleValueLabel}
                  type="datetime-local"
                  value={draft.scheduleValue}
                  disabled={op.pending !== null}
                  onChange={(e) => setDraft((d) => ({ ...d, scheduleValue: e.target.value }))}
                />
              </label>{" "}
              <em>{t.scheduleValueHintAt}</em>
            </p>
          ) : draft.scheduleKind === "every" ? (
            <p>
              <label>
                {t.scheduleValueLabel}:{" "}
                <input
                  aria-label={t.scheduleValueLabel}
                  type="number"
                  min={1}
                  step={1}
                  value={draft.scheduleValue}
                  disabled={op.pending !== null}
                  onChange={(e) => setDraft((d) => ({ ...d, scheduleValue: e.target.value }))}
                />
              </label>{" "}
              <label>
                {t.scheduleUnitLabel}:{" "}
                <select
                  aria-label={t.scheduleUnitLabel}
                  value={draft.scheduleUnit}
                  disabled={op.pending !== null}
                  onChange={(e) => setDraft((d) => ({ ...d, scheduleUnit: e.target.value }))}
                >
                  <option value="m">{t.everyUnitMin}</option>
                  <option value="h">{t.everyUnitHour}</option>
                  <option value="d">{t.everyUnitDay}</option>
                </select>
              </label>{" "}
              <em>{t.scheduleValueHintEvery}</em>
            </p>
          ) : (
            <p>
              <label>
                {t.scheduleValueLabel}:{" "}
                <input
                  aria-label={t.scheduleValueLabel}
                  placeholder={t.scheduleValueHintCron}
                  value={draft.scheduleValue}
                  disabled={op.pending !== null}
                  onChange={(e) => setDraft((d) => ({ ...d, scheduleValue: e.target.value }))}
                />
              </label>
            </p>
          )}
          {isCron && (
            <p>
              <label>
                {t.scheduleTzLabel}:{" "}
                <input
                  aria-label={t.scheduleTzLabel}
                  placeholder={t.scheduleTzHint}
                  value={draft.scheduleTz}
                  disabled={op.pending !== null}
                  onChange={(e) => setDraft((d) => ({ ...d, scheduleTz: e.target.value }))}
                />
              </label>
            </p>
          )}
          {schedHint !== null && <p role="alert">{schedHint}</p>}

          <h4>{t.payloadLabel}</h4>
          <p>
            <label>
              {t.payloadKindLabel}:{" "}
              <select
                value={draft.payloadKind}
                disabled={op.pending !== null}
                onChange={(e) => setDraft((d) => ({ ...d, payloadKind: e.target.value }))}
              >
                <option value="reminder">{t.kindReminder}</option>
                <option value="task">{t.kindTask}</option>
              </select>
            </label>
          </p>
          <p>
            <label>
              {t.textLabel}:{" "}
              <input
                aria-label={t.textLabel}
                value={draft.text}
                disabled={op.pending !== null}
                onChange={(e) => setDraft((d) => ({ ...d, text: e.target.value }))}
              />
            </label>{" "}
            <em>{t.textHint}</em>
          </p>
          {isReminder && (
            <p>
              <label>
                {t.wakeLabel}:{" "}
                <select
                  value={draft.wake === "" ? "now" : draft.wake}
                  disabled={op.pending !== null}
                  onChange={(e) => setDraft((d) => ({ ...d, wake: e.target.value }))}
                >
                  {WAKE_VALUES.map((w) => (
                    <option key={w} value={w}>
                      {w === "now" ? t.wakeNow : t.wakeNextHeartbeat}
                    </option>
                  ))}
                </select>
              </label>
            </p>
          )}
          {kindBlocked && <p role="alert">{t.kindChangeNote}</p>}
          {paylHint !== null && <p role="alert">{paylHint}</p>}

          <p>
            <button type="button" disabled={!canSubmit} onClick={submit}>
              {op.pending !== null && op.pending.kind === (editor.jobId === null ? "create" : "update")
                ? editor.jobId === null
                  ? t.creating
                  : t.saving
                : editor.jobId === null
                  ? t.create
                  : t.save}
            </button>{" "}
            <button type="button" disabled={op.pending !== null} onClick={closeEditor}>
              {t.cancel}
            </button>
          </p>
        </div>
      )}
    </section>
  );
}
