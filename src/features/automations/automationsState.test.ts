import { describe, expect, it } from "vitest";
import type { AutomationJobRow, TauriAppError } from "../../lib/tauri";
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
  isValidAutomationId,
  isValidAutomationName,
  isValidPayload,
  isValidSchedule,
  mapAutomationsError,
  automationsListReducer,
  automationsOpReducer,
  payloadKindLabel,
  payloadSummary,
  scheduleKindLabel,
  scheduleSummary,
  toLocalInput,
  toUtcIsoZ,
} from "./automationsState";

const sampleRow: AutomationJobRow = {
  id: "job-1",
  name: "standup",
  enabled: true,
  status: "ok",
  nextRunAtMs: null,
  schedule: { kind: "at", value: "2027-01-01T00:00:00Z", tz: null },
  payload: { kind: "reminder", text: "standup" },
};

describe("isValidAutomationId", () => {
  it("accepts slug-like ids up to 64 chars", () => {
    for (const ok of ["job-1", "a", "A.b_c:d-e", "x".repeat(64)]) {
      expect(isValidAutomationId(ok)).toBe(true);
    }
  });

  it("rejects empty, long, and bad-shape ids", () => {
    for (const bad of ["", " ", "bad id", "id/", "id\\x", "id|", "id?", "x".repeat(65)]) {
      expect(isValidAutomationId(bad)).toBe(false);
    }
  });
});

describe("isValidAutomationName", () => {
  it("accepts normal names up to 128 chars (trimmed)", () => {
    for (const ok of ["name", "한글 이름 with spaces  and “quotes”", "  padded  ", "x".repeat(128)]) {
      expect(isValidAutomationName(ok)).toBe(true);
    }
  });

  it("rejects empty/blank, >128 chars, and control characters", () => {
    for (const bad of ["", "   ", "a\u0001b", "a\u007Fb", "x".repeat(129)]) {
      expect(isValidAutomationName(bad)).toBe(false);
    }
  });
});

describe("isValidSchedule", () => {
  it("at: requires explicit UTC offset (Z or ±offset), no tz", () => {
    for (const ok of [
      "2027-02-01T16:00:00Z",
      "2027-02-01T16:00:00+09:00",
      "2027-02-01T16:00:00-0500",
      "2027-02-01T16:00:00.123Z",
      "2027-02-01T16:00:00.123456789Z",
    ]) {
      expect(isValidSchedule("at", ok, null)).toBe(true);
    }
    for (const bad of [
      "2027-02-01T16:00:00", // offset-less
      "2027-02-01 16:00:00Z", // space instead of T
      "2027-02-01", // date only
      "2027-02-01T16:00Z", // minute precision
      "2027-13-01T16:00:00Z", // month 13
      "2027-02-31T16:00:00Z", // Feb 31
      "2027-02-29T16:00:00Z", // non-leap Feb 29
      "2027-02-01T24:00:00Z", // hour 24
      "2027-02-01T16:60:00Z", // minute 60
      "2027-02-01T16:00:60Z", // second 60
      "2027-02-01T16:00:00z", // lowercase z
      "2027-02-01T16:00:00Zextra", // trailing junk
    ]) {
      expect(isValidSchedule("at", bad, null)).toBe(false);
    }
    expect(isValidSchedule("at", "2028-02-29T16:00:00Z", null)).toBe(true); // leap year
    expect(isValidSchedule("at", "2027-02-01T16:00:00Z", "Asia/Seoul")).toBe(false); // at + tz
    // A blank tz counts as absent.
    expect(isValidSchedule("at", "2027-02-01T16:00:00Z", "   ")).toBe(true);
  });

  it("every: fixed interval, no tz", () => {
    for (const ok of ["1m", "10m", "1h", "24h", "1d", "30d"]) {
      expect(isValidSchedule("every", ok, null)).toBe(true);
    }
    for (const bad of ["", "0m", "01m", "1x", "m", "1 m", "+1m", "1.5m", "10M"]) {
      expect(isValidSchedule("every", bad, null)).toBe(false);
    }
    expect(isValidSchedule("every", "10m", "Asia/Seoul")).toBe(false);
  });

  it("cron: 5/6 fields + optional IANA tz", () => {
    for (const ok of ["0 9 * * 1", "*/5 * * * *", "0 9 * * 1 2027", "5,10 4,16 1,15 * 3-5", "0-30/5 * * * *"]) {
      expect(isValidSchedule("cron", ok, null)).toBe(true);
    }
    for (const bad of ["", "0 9 * *", "0 9 * * 1 2 3", "a 9 * * *", "0 9 * * * (mon)"]) {
      expect(isValidSchedule("cron", bad, null)).toBe(false);
    }
    for (const tz of ["Asia/Seoul", "America/New_York", "UTC", "Etc/GMT+8"]) {
      expect(isValidSchedule("cron", "0 9 * * 1", tz)).toBe(true);
    }
    for (const badTz of ["Asia Seoul", "Asia/Seoul/", "x".repeat(65)]) {
      expect(isValidSchedule("cron", "0 9 * * 1", badTz)).toBe(false);
    }
    // A blank tz counts as absent.
    expect(isValidSchedule("cron", "0 9 * * 1", "   ")).toBe(true);
  });

  it("rejects unknown kinds", () => {
    expect(isValidSchedule("stream", "whatever", null)).toBe(false);
    expect(isValidSchedule("on-exit", "x", null)).toBe(false);
  });
});

describe("isValidPayload", () => {
  it("reminder: text 1–8000, wake optional and enum-gated", () => {
    expect(isValidPayload("reminder", "약속", null)).toBe(true);
    expect(isValidPayload("reminder", "약속", "now")).toBe(true);
    expect(isValidPayload("reminder", "약속", "next-heartbeat")).toBe(true);
    expect(isValidPayload("reminder", "x".repeat(8000), null)).toBe(true);
    expect(isValidPayload("reminder", "", null)).toBe(false);
    expect(isValidPayload("reminder", "   ", null)).toBe(false);
    expect(isValidPayload("reminder", "x".repeat(8001), null)).toBe(false);
    expect(isValidPayload("reminder", "text", "later")).toBe(false);
  });

  it("task: text 1–8000, wake always rejected", () => {
    expect(isValidPayload("task", "보고서", null)).toBe(true);
    expect(isValidPayload("task", "보고서", "now")).toBe(false);
    expect(isValidPayload("task", "보고서", "next-heartbeat")).toBe(false);
  });

  it("rejects unknown kinds", () => {
    expect(isValidPayload("command", "ls", null)).toBe(false);
    expect(isValidPayload("script", "x", null)).toBe(false);
    expect(isValidPayload("", "text", null)).toBe(false);
  });
});

describe("draft helpers", () => {
  it("draftScheduleTz returns null for blank tz", () => {
    expect(draftScheduleTz({ ...emptyAutomationDraft, scheduleTz: "" })).toBeNull();
    expect(draftScheduleTz({ ...emptyAutomationDraft, scheduleTz: "   " })).toBeNull();
    expect(draftScheduleTz({ ...emptyAutomationDraft, scheduleTz: "  Asia/Seoul  " })).toBe("Asia/Seoul");
  });

  it("draftWake is null for tasks and blank wake", () => {
    expect(draftWake({ ...emptyAutomationDraft, payloadKind: "task", wake: "now" })).toBeNull();
    expect(draftWake({ ...emptyAutomationDraft, payloadKind: "reminder", wake: "" })).toBeNull();
    expect(draftWake({ ...emptyAutomationDraft, payloadKind: "reminder", wake: "next-heartbeat" })).toBe("next-heartbeat");
  });

  it("isDraftValid is false on a fresh draft and true on a complete one", () => {
    expect(isDraftValid(emptyAutomationDraft)).toBe(false);
    const valid = {
      name: "새 이름",
      scheduleKind: "cron",
      scheduleValue: "0 9 * * 1",
      scheduleUnit: "m",
      scheduleTz: "Asia/Seoul",
      payloadKind: "task",
      text: "보고서",
      wake: "now",
    };
    expect(isDraftValid(valid)).toBe(true);
  });

  it("isDraftValid enforces the payload-kind/wake cross-rule", () => {
    const base = {
      name: "n",
      scheduleKind: "every",
      scheduleValue: "10",
      scheduleUnit: "m",
      scheduleTz: "",
      text: "t",
      wake: "now",
    };
    // task: the wake field is ignored (always submitted as null)
    expect(isDraftValid({ ...base, payloadKind: "task" })).toBe(true);
    expect(isDraftValid({ ...base, payloadKind: "reminder", wake: "later" })).toBe(false);
    expect(isDraftValid({ ...base, payloadKind: "reminder", wake: "" })).toBe(true);
  });

  it("every: wire value = number + selected unit (1+/1x-style rules via the validator)", () => {
    const base = {
      name: "n",
      scheduleKind: "every",
      scheduleUnit: "m",
      scheduleTz: "",
      text: "t",
      wake: "now",
      payloadKind: "reminder",
    };
    expect(draftScheduleValue({ ...base, scheduleValue: "10", scheduleUnit: "h" })).toBe("10h");
    expect(isDraftValid({ ...base, scheduleValue: "10", scheduleUnit: "h" })).toBe(true);
    expect(isDraftValid({ ...base, scheduleValue: "1", scheduleUnit: "d" })).toBe(true);
    expect(isDraftValid({ ...base, scheduleValue: "", scheduleUnit: "m" })).toBe(false);
    expect(isDraftValid({ ...base, scheduleValue: "0", scheduleUnit: "m" })).toBe(false);
  });

  it("draftFromJob seeds a draft (wake defaults to now)", () => {
    const job = {
      id: "job-1",
      name: "주말 보고",
      enabled: true,
      status: "ok",
      schedule: { kind: "cron", value: "0 9 * * 1", tz: "Asia/Seoul" },
      payload: { kind: "task", text: "보고서 작성" },
    };
    const draft = draftFromJob(job);
    expect(draft.name).toBe("주말 보고");
    expect(draft.scheduleKind).toBe("cron");
    expect(draft.scheduleValue).toBe("0 9 * * 1");
    expect(draft.scheduleTz).toBe("Asia/Seoul");
    expect(draft.payloadKind).toBe("task");
    expect(draft.text).toBe("보고서 작성");
    expect(draft.wake).toBe("now");
  });

  it("draftFromJob converts an `at` wire value to the local datetime-local value", () => {
    const job = {
      id: "job-2",
      name: "일회성",
      enabled: true,
      status: "idle",
      schedule: { kind: "at", value: "2027-01-01T09:30:00+09:00", tz: null },
      payload: { kind: "reminder", text: "약속" },
    };
    const draft = draftFromJob(job);
    expect(draft.scheduleKind).toBe("at");
    expect(draft.scheduleValue).toBe(toLocalInput("2027-01-01T09:30:00+09:00"));
    expect(draft.scheduleUnit).toBe("m");
    expect(isDraftValid(draft)).toBe(true);
  });

  it("draftFromJob splits an `every` wire value into number + unit", () => {
    const job = {
      id: "job-3",
      name: "간격",
      enabled: true,
      status: "idle",
      schedule: { kind: "every", value: "10m", tz: null },
      payload: { kind: "task", text: "라운드" },
    };
    const draft = draftFromJob(job);
    expect(draft.scheduleKind).toBe("every");
    expect(draft.scheduleValue).toBe("10");
    expect(draft.scheduleUnit).toBe("m");
    expect(isDraftValid(draft)).toBe(true);
  });

  it("isPayloadKindChanged only fires in edit mode", () => {
    const create = { jobId: null, originalPayloadKind: null };
    expect(isPayloadKindChanged(create, { ...emptyAutomationDraft, payloadKind: "task" })).toBe(false);
    const editReminder = { jobId: "job-1", originalPayloadKind: "reminder" };
    expect(isPayloadKindChanged(editReminder, { ...emptyAutomationDraft, payloadKind: "reminder" })).toBe(false);
    expect(isPayloadKindChanged(editReminder, { ...emptyAutomationDraft, payloadKind: "task" })).toBe(true);
  });
});

describe("automationsListReducer", () => {
  it("start is a no-op while loading", () => {
    const loading = automationsListReducer(initialAutomationsListState, { type: "start" });
    expect(automationsListReducer(loading, { type: "start" })).toBe(loading);
  });

  it("finish with an error clears previous rows (fail-closed)", () => {
    const withRows = automationsListReducer(
      automationsListReducer(initialAutomationsListState, { type: "start" }),
      { type: "finish", jobs: [sampleRow], error: null },
    );
    const failed = automationsListReducer(
      automationsListReducer(withRows, { type: "start" }),
      { type: "finish", jobs: null, error: "boom" },
    );
    expect(failed.jobs).toBeNull();
    expect(failed.error).toBe("boom");
    expect(failed.loading).toBe(false);
  });

  it("finish without an error stores the rows", () => {
    const done = automationsListReducer(initialAutomationsListState, {
      type: "finish",
      jobs: [sampleRow],
      error: null,
    });
    expect(done.jobs).toEqual([sampleRow]);
    expect(done.error).toBeNull();
  });
});

describe("automationsOpReducer", () => {
  it("start is a no-op while an op is pending (duplicate-submit guard)", () => {
    const pending = automationsOpReducer(initialAutomationsOpState, {
      type: "start",
      kind: "create",
      jobId: null,
    });
    expect(pending.pending).toEqual({ kind: "create", jobId: null });
    expect(
      automationsOpReducer(pending, { type: "start", kind: "delete", jobId: "job-1" }),
    ).toBe(pending);
  });

  it("finish clears pending and bumps reloadCounter (success and failure)", () => {
    const pending = automationsOpReducer(initialAutomationsOpState, {
      type: "start",
      kind: "delete",
      jobId: "job-1",
    });
    const failed = automationsOpReducer(pending, { type: "finish", error: "boom" });
    expect(failed.pending).toBeNull();
    expect(failed.error).toBe("boom");
    expect(failed.reloadCounter).toBe(1);
    const done = automationsOpReducer(
      automationsOpReducer(failed, { type: "start", kind: "create", jobId: null }),
      { type: "finish", error: null },
    );
    expect(done.error).toBeNull();
    expect(done.reloadCounter).toBe(2);
  });
});

describe("display helpers", () => {
  it("scheduleKindLabel / payloadKindLabel degrade to unknown", () => {
    expect(scheduleKindLabel("at")).toBe("at");
    expect(scheduleKindLabel("stream")).toBe("unknown");
    expect(scheduleKindLabel(null)).toBe("unknown");
    expect(payloadKindLabel("task")).toBe("task");
    expect(payloadKindLabel("command")).toBe("unknown");
    expect(payloadKindLabel(null)).toBe("unknown");
  });

  it("scheduleSummary / payloadSummary handle absent views", () => {
    expect(scheduleSummary(null)).toBeNull();
    expect(scheduleSummary({ kind: "cron", value: "0 9 * * 1", tz: "Asia/Seoul" })).toBe(
      "cron 0 9 * * 1 (Asia/Seoul)",
    );
    expect(scheduleSummary({ kind: "at", value: null, tz: null })).toBe("한 번 (at)");
    expect(scheduleSummary({ kind: "every", value: null, tz: null })).toBe("반복 (every)");
    expect(payloadSummary(null)).toBeNull();
    expect(payloadSummary({ kind: "task", text: "보고서" })).toBe("task: 보고서");
    expect(payloadSummary({ kind: "reminder", text: null })).toBe("reminder");
  });

  it("scheduleSummary renders at as a local datetime", () => {
    expect(scheduleSummary({ kind: "at", value: "2027-02-01T16:00:00Z", tz: null })).toBe(
      "한 번 (at) 2027. 2. 2. AM 1:00:00", // host TZ: UTC+9 (KST); ICU locale output
    );
    // Unparseable values fail soft to raw.
    expect(scheduleSummary({ kind: "at", value: "not-a-date", tz: null })).toBe(
      "한 번 (at) not-a-date",
    );
  });

  it("scheduleSummary renders every as 'N분마다' 류", () => {
    expect(scheduleSummary({ kind: "every", value: "10m", tz: null })).toBe("반복 (every) 10분마다");
    expect(scheduleSummary({ kind: "every", value: "2h", tz: null })).toBe("반복 (every) 2시간마다");
    expect(scheduleSummary({ kind: "every", value: "1d", tz: null })).toBe("반복 (every) 1일마다");
    // Non-matching values fail soft to raw.
    expect(scheduleSummary({ kind: "every", value: "1x", tz: null })).toBe("반복 (every) 1x");
  });
});

describe("toUtcIsoZ / toLocalInput", () => {
  it("converts a local datetime-local value to UTC ISO 8601 (Z suffix)", () => {
    expect(toUtcIsoZ("2027-01-31T16:00")).toBe("2027-01-31T07:00:00Z"); // host TZ: UTC+9 (KST)
  });

  it("converts a wire value to the local datetime-local value (seconds dropped)", () => {
    expect(toLocalInput("2027-01-01T09:30:45Z")).toBe("2027-01-01T18:30"); // host TZ: UTC+9 (KST)
    expect(toLocalInput("2027-01-01T09:30:00+09:00")).toBe("2027-01-01T09:30");
  });

  it("round-trips local -> UTC -> local", () => {
    expect(toLocalInput(toUtcIsoZ("2027-01-31T16:00"))).toBe("2027-01-31T16:00");
  });

  it("fails soft on unparseable/empty input", () => {
    expect(toUtcIsoZ("")).toBe("");
    expect(toUtcIsoZ("not-a-date")).toBe("");
    expect(toLocalInput("not-a-date")).toBe("not-a-date");
  });
});

describe("mapAutomationsError", () => {
  it("maps each known stable code to itself", () => {
    for (const code of [
      "automation-id-invalid",
      "automation-name-invalid",
      "automation-schedule-invalid",
      "automation-payload-invalid",
      "openclaw-automations-failed",
      "openclaw-not-found",
      "process-timeout",
      "process-failed",
    ]) {
      const error: TauriAppError = { code, message: "detail" };
      expect(mapAutomationsError(error)).toBe(code);
    }
  });

  it("falls back for unknown codes", () => {
    const error: TauriAppError = { code: "ipc-failed", message: "boom" };
    expect(mapAutomationsError(error)).toBe("fallback");
  });
});
