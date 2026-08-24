import { describe, expect, it } from "vitest";
import {
  SKILLS_ERROR_CODES,
  initialSkillsToggleState,
  mapSkillsError,
  skillsToggleReducer,
  type SkillsToggleState,
} from "./skillsState";

describe("skillsToggleReducer", () => {
  it("starts a toggle from idle", () => {
    const next = skillsToggleReducer(initialSkillsToggleState, {
      type: "start",
      key: "weather",
    });
    expect(next.pending).toBe("weather");
    expect(next.error).toBeNull();
    expect(next.reloadCounter).toBe(0);
  });

  it("ignores a duplicate start while a toggle is pending", () => {
    const pending: SkillsToggleState = {
      pending: "weather",
      error: null,
      reloadCounter: 0,
    };
    const next = skillsToggleReducer(pending, { type: "start", key: "github" });
    expect(next).toBe(pending); // duplicate guard: state unchanged
  });

  it("finishes a successful toggle and bumps the re-query counter", () => {
    const pending: SkillsToggleState = {
      pending: "weather",
      error: null,
      reloadCounter: 2,
    };
    const next = skillsToggleReducer(pending, { type: "finish", error: null });
    expect(next.pending).toBeNull();
    expect(next.error).toBeNull();
    expect(next.reloadCounter).toBe(3); // success triggers a list re-query
  });

  it("finishes a failed toggle with the message and still bumps the counter", () => {
    const pending: SkillsToggleState = {
      pending: "weather",
      error: null,
      reloadCounter: 0,
    };
    const next = skillsToggleReducer(pending, {
      type: "finish",
      error: "설정 저장 실패",
    });
    expect(next.pending).toBeNull();
    expect(next.error).toBe("설정 저장 실패");
    // failure also triggers a re-query (no optimistic state)
    expect(next.reloadCounter).toBe(1);
  });

  it("clears the error when a new toggle starts", () => {
    const withError: SkillsToggleState = {
      pending: null,
      error: "old error",
      reloadCounter: 1,
    };
    const next = skillsToggleReducer(withError, { type: "start", key: "a" });
    expect(next.error).toBeNull();
    expect(next.pending).toBe("a");
  });
});

describe("mapSkillsError", () => {
  it("passes every known stable code through", () => {
    for (const code of SKILLS_ERROR_CODES) {
      expect(mapSkillsError({ code, message: "x" })).toBe(code);
    }
  });

  it("falls back for unknown codes", () => {
    expect(mapSkillsError({ code: "something-else", message: "x" })).toBe("fallback");
  });
});
