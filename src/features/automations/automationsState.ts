/**
 * Pure logic for the Phase 7 automations feature. No Tauri calls here —
 * keep it unit-testable.
 *
 * Contracts (no optimistic updates):
 * - One pending mutation at a time across the whole feature
 *   (duplicate-submit guard); every finished mutation (success OR failure)
 *   bumps `reloadCounter` → the component re-queries the actual state.
 * - Create/edit form: frontend pre-validation mirroring the Rust
 *   validators (S2 UX — the backend re-validates; it is the security
 *   boundary). The payload kind cannot change on edit (kind change =
 *   delete + recreate, blocked here).
 * - Manual execution (`automations run`/`runs`) is a non-goal and is never
 *   offered.
 */

import { getStrings } from "../../i18n/ko";
import {
  WAKE_VALUES,
  type AutomationJob,
  type AutomationJobRow,
  type AutomationScheduleView,
  type TauriAppError,
} from "../../lib/tauri";

// --- input validation (mirrors the Rust pre-checks, S2 UX) ---------------------

/**
 * A job id: 1–64 chars of `[A-Za-z0-9._:-]`. Mirrors the Rust validator.
 * (UX only — the Rust layer re-validates before any argv use.)
 */
export function isValidAutomationId(id: string): boolean {
  return id.length >= 1 && id.length <= 64 && /^[A-Za-z0-9._:-]+$/.test(id);
}

/** A job name: non-empty after trimming, ≤128 chars, no control characters. */
export function isValidAutomationName(name: string): boolean {
  const trimmed = name.trim();
  return (
    trimmed.length > 0 && [...trimmed].length <= 128 && !/[\u0000-\u001F\u007F-\u009F]/.test(trimmed)
  );
}

/** `YYYY-MM-DD` day-range helper (leap-year aware). */
function daysInMonth(year: number, month: number): number {
  switch (month) {
    case 1:
    case 3:
    case 5:
    case 7:
    case 8:
    case 10:
    case 12:
      return 31;
    case 4:
    case 6:
    case 9:
    case 11:
      return 30;
    case 2:
      return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0 ? 29 : 28;
    default:
      return 0;
  }
}

/** Explicit UTC ISO 8601 (`Z` or a `±offset`); offset-less is rejected. */
function isExplicitUtcIso8601(value: string): boolean {
  // Minimum layout: `YYYY-MM-DDTHH:MM:SSZ` (20 chars, zone at index 19).
  if (value.length < 20) return false;
  const isDigits = (s: string) => /^\d+$/.test(s);
  if (!isDigits(value.slice(0, 4))) return false;
  if (value[4] !== "-") return false;
  if (!isDigits(value.slice(5, 7))) return false;
  if (value[7] !== "-") return false;
  if (!isDigits(value.slice(8, 10))) return false;
  if (value[10] !== "T") return false;
  if (!isDigits(value.slice(11, 13))) return false;
  if (value[13] !== ":") return false;
  if (!isDigits(value.slice(14, 16))) return false;
  if (value[16] !== ":") return false;
  if (!isDigits(value.slice(17, 19))) return false;
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(5, 7));
  const day = Number(value.slice(8, 10));
  const hour = Number(value.slice(11, 13));
  const minute = Number(value.slice(14, 16));
  const second = Number(value.slice(17, 19));
  if (!(month >= 1 && month <= 12)) return false;
  if (!(day >= 1 && day <= daysInMonth(year, month))) return false;
  if (hour > 23 || minute > 59 || second > 59) return false;
  let index = 19;
  if (value[index] === ".") {
    index += 1;
    const start = index;
    while (index < value.length && /\d/.test(value[index])) index += 1;
    if (index === start || index - start > 9) return false;
  }
  if (index >= value.length) return false;
  const zone = value[index];
  if (zone === "Z") return index + 1 === value.length;
  if (zone === "+" || zone === "-") {
    index += 1;
    if (value.length - index < 2 || !isDigits(value.slice(index, index + 2))) return false;
    index += 2;
    // The colon between offset hours and minutes is optional (±HH:MM / ±HHMM).
    if (index < value.length && value[index] === ":") index += 1;
    if (value.length - index < 2 || !isDigits(value.slice(index, index + 2))) return false;
    index += 2;
    return index === value.length;
  }
  return false;
}

/** `^[1-9][0-9]*[mhd]$` — fixed interval without a leading zero. */
function isEveryInterval(value: string): boolean {
  return /^[1-9]\d*[mhd]$/.test(value);
}

/** 5/6 whitespace fields, each `^[\d*,/-]+$` (semantics are the CLI's). */
function isCronExpression(value: string): boolean {
  const fields = value.split(/\s+/).filter((f) => f.length > 0);
  return (
    (fields.length === 5 || fields.length === 6) &&
    fields.every((f) => /^[\d*,/-]+$/.test(f))
  );
}

/** IANA timezone id (existence is the CLI's); alphanumeric first/last. */
function isIanaTimezone(value: string): boolean {
  if (value.length < 1 || value.length > 64) return false;
  if (!/[A-Za-z0-9]/.test(value[0])) return false;
  if (!/[A-Za-z0-9]/.test(value[value.length - 1])) return false;
  return /^[A-Za-z0-9+/_-]+$/.test(value);
}

/**
 * A schedule `{kind, value, tz}` (mirrors Rust `validate_schedule`):
 * `at` = explicit UTC ISO 8601 (no tz), `every` = interval (no tz),
 * `cron` = 5/6 fields (tz allowed, IANA-shaped). An empty/blank tz counts
 * as absent.
 */
export function isValidSchedule(kind: string, value: string, tz: string | null): boolean {
  const trimmedTz = (tz ?? "").trim();
  const tzOpt = trimmedTz === "" ? null : trimmedTz;
  switch (kind) {
    case "at":
      return tzOpt === null && isExplicitUtcIso8601(value);
    case "every":
      return tzOpt === null && isEveryInterval(value);
    case "cron":
      return isCronExpression(value) && (tzOpt === null || isIanaTimezone(tzOpt));
    default:
      return false;
  }
}

/**
 * A payload `{kind, text, wake}` (mirrors Rust `validate_automation_payload`):
 * text 1–8000 chars after trimming; `wake` is reminder-only and must be a
 * known wake value.
 */
export function isValidPayload(kind: string, text: string, wake: string | null): boolean {
  const trimmed = text.trim();
  if (trimmed.length === 0 || [...trimmed].length > 8000) return false;
  switch (kind) {
    case "reminder":
      return wake === null || (WAKE_VALUES as readonly string[]).includes(wake);
    case "task":
      return wake === null;
    default:
      return false;
  }
}

// --- schedule value conversion (local UI value <-> wire value) -------------------

/**
 * A `datetime-local` raw value (`2027-01-31T16:00`, interpreted as local
 * time) → explicit UTC ISO 8601 with a `Z` suffix (`2027-01-31T07:00:00Z`).
 * Unparseable input fails soft to `""` (the form validation then blocks it).
 */
export function toUtcIsoZ(local: string): string {
  const date = new Date(local);
  if (Number.isNaN(date.getTime())) {
    return "";
  }
  return date.toISOString().replace(/\.\d{3}Z$/, "Z");
}

/**
 * A wire schedule value (`2027-02-01T16:00:00Z`, `...+09:00`, ...) → the
 * local `YYYY-MM-DDTHH:MM` shape for a `datetime-local` input (seconds
 * dropped). Unparseable input fails soft to the raw value.
 */
export function toLocalInput(wire: string): string {
  const date = new Date(wire);
  if (Number.isNaN(date.getTime())) {
    return wire;
  }
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}`;
}

// --- draft (create/edit form) ---------------------------------------------------

/** The editable fields of a job (create + edit share the shape). */
export interface AutomationDraft {
  name: string;
  scheduleKind: string;
  /**
   * Raw value for the kind: `at` = `datetime-local` string (local time),
   * `every` = the number part (`10`), `cron` = the expression.
   */
  scheduleValue: string;
  /** `every` unit (`"m" | "h" | "d"`); ignored by the other kinds. */
  scheduleUnit: string;
  scheduleTz: string;
  payloadKind: string;
  text: string;
  /** `""` = the reminder default (`now`); ignored for tasks. */
  wake: string;
}

export const emptyAutomationDraft: AutomationDraft = {
  name: "",
  scheduleKind: "at",
  scheduleValue: "",
  scheduleUnit: "m",
  scheduleTz: "",
  payloadKind: "reminder",
  text: "",
  wake: "now",
};

/** The draft's timezone as submitted (`null` when blank). */
export function draftScheduleTz(draft: AutomationDraft): string | null {
  const tz = draft.scheduleTz.trim();
  return tz === "" ? null : tz;
}

/** The draft's wake as submitted (`null` for tasks / blank). */
export function draftWake(draft: AutomationDraft): string | null {
  if (draft.payloadKind === "task") return null;
  const wake = draft.wake.trim();
  return wake === "" ? null : wake;
}

/**
 * The schedule value as it crosses the wire (the Rust layer validates the
 * same shape): `at` → local → UTC ISO 8601 (`Z`), `every` → `Nm`/`Nh`/`Nd`,
 * `cron` → the raw expression.
 */
export function draftScheduleValue(draft: AutomationDraft): string {
  const value = draft.scheduleValue.trim();
  if (draft.scheduleKind === "at") {
    return toUtcIsoZ(value);
  }
  if (draft.scheduleKind === "every") {
    return `${value}${draft.scheduleUnit}`;
  }
  return value;
}

/** Whether the whole draft passes the frontend pre-checks (S2 UX). */
export function isDraftValid(draft: AutomationDraft): boolean {
  return (
    isValidAutomationName(draft.name) &&
    isValidSchedule(draft.scheduleKind, draftScheduleValue(draft), draftScheduleTz(draft)) &&
    isValidPayload(draft.payloadKind, draft.text, draftWake(draft))
  );
}

/** The `every` wire shape (`10m`) split into number + unit. */
const EVERY_WIRE_PATTERN = /^(\d+)([mhd])$/;

/** Seeds a draft from a job detail (edit mode; wake defaults to `now`). */
export function draftFromJob(job: AutomationJob): AutomationDraft {
  const schedule = job.schedule;
  const payload = job.payload;
  const kind = schedule?.kind ?? "at";
  const rawValue = schedule?.value ?? "";
  let value = rawValue;
  let unit = "m";
  if (kind === "at") {
    value = toLocalInput(rawValue);
  } else if (kind === "every") {
    const match = EVERY_WIRE_PATTERN.exec(rawValue);
    if (match !== null) {
      value = match[1];
      unit = match[2];
    }
  }
  return {
    name: job.name ?? "",
    scheduleKind: kind,
    scheduleValue: value,
    scheduleUnit: unit,
    scheduleTz: schedule?.tz ?? "",
    payloadKind: payload?.kind ?? "reminder",
    text: payload?.text ?? "",
    wake: "now",
  };
}

// --- editor mode ----------------------------------------------------------------

/** Which job the open form edits (`null` id = create a new job). */
export interface AutomationEditor {
  jobId: string | null;
  /** The payload kind at edit start (change = block save, contract §4). */
  originalPayloadKind: string | null;
}

/** Payload-kind change detection (kind change = delete + recreate). */
export function isPayloadKindChanged(editor: AutomationEditor, draft: AutomationDraft): boolean {
  return (
    editor.jobId !== null &&
    editor.originalPayloadKind !== null &&
    draft.payloadKind !== editor.originalPayloadKind
  );
}

// --- list state (single-flight read) ----------------------------------------------

export interface AutomationsListState {
  loading: boolean;
  /** The loaded rows (`null` = not loaded yet / cleared on failure). */
  jobs: AutomationJobRow[] | null;
  /** Korean message of the last failed load (null = none). */
  error: string | null;
}

export const initialAutomationsListState: AutomationsListState = {
  loading: false,
  jobs: null,
  error: null,
};

export type AutomationsListAction =
  | { type: "start" }
  | { type: "finish"; jobs: AutomationJobRow[] | null; error: string | null };

/**
 * List reducer:
 * - `start` while loading is ignored (duplicate-run guard);
 * - `finish` with an error is fail-closed: any previous rows are cleared
 *   (the UI never keeps showing possibly stale job state).
 */
export function automationsListReducer(
  state: AutomationsListState,
  action: AutomationsListAction,
): AutomationsListState {
  switch (action.type) {
    case "start":
      if (state.loading) {
        return state;
      }
      return { loading: true, jobs: null, error: null };
    case "finish":
      if (action.error !== null) {
        return { loading: false, jobs: null, error: action.error };
      }
      return { loading: false, jobs: action.jobs, error: null };
  }
}

// --- mutation state (single pending op, re-query after) ---------------------------

/** The automations mutations the feature can run (one at a time). */
export type AutomationsOpKind = "create" | "update" | "enabled" | "delete";

/** Mutation state: one pending operation at a time (duplicate-submit guard). */
export interface AutomationsOpState {
  pending: { kind: AutomationsOpKind; jobId: string | null } | null;
  /** Korean message of the last failed mutation (null = none). */
  error: string | null;
  /** Bumped on every finished mutation → triggers a state re-query. */
  reloadCounter: number;
}

export const initialAutomationsOpState: AutomationsOpState = {
  pending: null,
  error: null,
  reloadCounter: 0,
};

export type AutomationsOpAction =
  | { type: "start"; kind: AutomationsOpKind; jobId: string | null }
  | { type: "finish"; error: string | null };

/**
 * Mutation reducer:
 * - `start` while an operation is pending is ignored (duplicate-submit guard);
 * - `finish` always bumps `reloadCounter` (re-query on success AND failure).
 */
export function automationsOpReducer(
  state: AutomationsOpState,
  action: AutomationsOpAction,
): AutomationsOpState {
  switch (action.type) {
    case "start":
      if (state.pending !== null) {
        return state;
      }
      return {
        pending: { kind: action.kind, jobId: action.jobId },
        error: null,
        reloadCounter: state.reloadCounter,
      };
    case "finish":
      return {
        pending: null,
        error: action.error,
        reloadCounter: state.reloadCounter + 1,
      };
  }
}

// --- display helpers --------------------------------------------------------------

/** Known schedule-kind labels (the UI degrades to raw for unknown kinds). */
export type ScheduleKindLabel = "at" | "every" | "cron" | "unknown";
export function scheduleKindLabel(kind: string | null): ScheduleKindLabel {
  if (kind === "at" || kind === "every" || kind === "cron") {
    return kind;
  }
  return "unknown";
}

/** Known payload-kind labels (the UI degrades to raw for unknown kinds). */
export type PayloadKindLabel = "reminder" | "task" | "unknown";
export function payloadKindLabel(kind: string | null): PayloadKindLabel {
  if (kind === "reminder" || kind === "task") {
    return kind;
  }
  return "unknown";
}

/** `every` unit → Korean suffix (contract: "10분마다" 류). */
const EVERY_UNIT_SUFFIX: Record<string, string> = {
  m: getStrings("automations").everySuffixMin,
  h: getStrings("automations").everySuffixHour,
  d: getStrings("automations").everySuffixDay,
};

/**
 * A short, human-readable schedule summary for a row (`null` when the view
 * is absent): `at` → local datetime, `every` → "10분마다" 류, `cron` → raw
 * expression + tz. Unparseable values fail soft to raw.
 */
export function scheduleSummary(view: AutomationScheduleView | null): string | null {
  if (view === null) {
    return null;
  }
  const tz = view.tz !== null ? ` (${view.tz})` : "";
  const value = view.value;
  const kindStrings = getStrings("automations");
  if (view.kind === "at") {
    const label = kindStrings.scheduleKindAt;
    if (value === null) {
      return label;
    }
    const parsed = new Date(value);
    const shown = Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString("ko-KR");
    return `${label} ${shown}${tz}`;
  }
  if (view.kind === "every") {
    const label = kindStrings.scheduleKindEvery;
    if (value === null) {
      return label;
    }
    const match = EVERY_WIRE_PATTERN.exec(value);
    const shown =
      match !== null ? `${match[1]}${EVERY_UNIT_SUFFIX[match[2]] ?? ""}` : value;
    return `${label} ${shown}${tz}`;
  }
  if (view.kind === "cron") {
    return value !== null ? `cron ${value}${tz}` : "cron";
  }
  return value !== null ? `${view.kind} ${value}${tz}` : view.kind;
}

/** A short payload summary for a row (`null` when the view is absent). */
export function payloadSummary(view: { kind: string; text: string | null } | null): string | null {
  if (view === null) {
    return null;
  }
  return view.text !== null ? `${view.kind}: ${view.text}` : view.kind;
}

// --- error mapping ------------------------------------------------------------------

/** Error codes the automations feature can receive (stable, from Rust). */
export const AUTOMATIONS_ERROR_CODES = [
  "automation-id-invalid",
  "automation-name-invalid",
  "automation-schedule-invalid",
  "automation-payload-invalid",
  "openclaw-automations-failed",
  "openclaw-not-found",
  "process-timeout",
  "process-failed",
] as const;

export type AutomationsErrorCode = (typeof AUTOMATIONS_ERROR_CODES)[number];

/**
 * Maps an IPC `AppError` to an i18n key. Unknown codes fall back so the UI
 * can always show a message (the fallback message is generic Korean).
 */
export function mapAutomationsError(error: TauriAppError): AutomationsErrorCode | "fallback" {
  return (AUTOMATIONS_ERROR_CODES as readonly string[]).includes(error.code)
    ? (error.code as AutomationsErrorCode)
    : "fallback";
}
