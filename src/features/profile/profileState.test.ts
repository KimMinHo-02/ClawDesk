import { describe, expect, it } from "vitest";
import {
  DEFAULT_LOG_LIMIT,
  LOG_LIMIT_MAX,
  LOG_LIMIT_MIN,
  LOG_LIMIT_OPTIONS,
  PROFILE_ERROR_CODES,
  formatLogEvent,
  isValidLogLimit,
  mapProfileError,
} from "./profileState";

describe("log limit validation", () => {
  it("every offered option is a valid limit", () => {
    for (const option of LOG_LIMIT_OPTIONS) {
      expect(isValidLogLimit(option)).toBe(true);
    }
    expect(DEFAULT_LOG_LIMIT).toBe(200);
    expect(DEFAULT_LOG_LIMIT).toBe(LOG_LIMIT_OPTIONS[2]);
  });

  it("accepts the full backend range 1..=1000", () => {
    expect(isValidLogLimit(LOG_LIMIT_MIN)).toBe(true);
    expect(isValidLogLimit(LOG_LIMIT_MAX)).toBe(true);
    expect(isValidLogLimit(999)).toBe(true);
  });

  it("rejects out-of-range and non-integer values (fail-closed)", () => {
    expect(isValidLogLimit(0)).toBe(false);
    expect(isValidLogLimit(1001)).toBe(false);
    expect(isValidLogLimit(1.5)).toBe(false);
    expect(isValidLogLimit(Number.NaN)).toBe(false);
    expect(isValidLogLimit(Number.POSITIVE_INFINITY)).toBe(false);
  });
});

describe("formatLogEvent", () => {
  it("renders a full log entry with time, level and subsystem", () => {
    const line = formatLogEvent({
      kind: "log",
      time: "2026-08-26T10:00:00.000Z",
      level: "info",
      subsystem: "gateway",
      message: "gateway started",
    });
    expect(line).toBe("2026-08-26T10:00:00.000Z INFO gateway gateway started");
  });

  it("omits absent optional fields (fail-soft)", () => {
    const line = formatLogEvent({ kind: "log", message: "upstream reset by peer" });
    expect(line).toBe("upstream reset by peer");
  });

  it("renders a meta event with its file and source kind", () => {
    const line = formatLogEvent({
      kind: "meta",
      file: "openclaw-2026-08-26.log",
      sourceKind: "file",
    });
    expect(line).toBe("[meta] openclaw-2026-08-26.log (file)");
  });

  it("renders a meta event without optional fields", () => {
    expect(formatLogEvent({ kind: "meta" })).toBe("[meta] ? (unknown)");
  });

  it("renders notice events with and without a message", () => {
    expect(
      formatLogEvent({ kind: "notice", message: "showing most recent lines", truncated: true }),
    ).toBe("[notice] showing most recent lines");
    expect(formatLogEvent({ kind: "notice", truncated: true })).toBe("[notice]");
  });

  it("renders raw lines verbatim", () => {
    expect(formatLogEvent({ kind: "raw", line: "unparsed legacy line 123" })).toBe(
      "unparsed legacy line 123",
    );
  });
});

describe("mapProfileError", () => {
  it("passes every known stable code through", () => {
    for (const code of PROFILE_ERROR_CODES) {
      expect(mapProfileError({ code, message: "x" })).toBe(code);
    }
  });

  it("falls back for unknown codes", () => {
    expect(mapProfileError({ code: "something-else", message: "x" })).toBe("fallback");
  });
});
