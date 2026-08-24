/**
 * Skills feature (Phase 4): skill list + enable/disable toggle.
 *
 * All OS/OpenClaw work goes through the Tauri IPC wrappers
 * (`src/lib/tauri`) — the component never touches processes (S1/S10).
 * Toggles are non-optimistic: after every toggle (success or failure) the
 * list is re-queried, and the UI notes that changes apply from the next
 * new session.
 */

import { useCallback, useEffect, useReducer, useState } from "react";
import { getStrings } from "../../i18n/ko";
import {
  type SkillRow,
  listSkills,
  normalizeAppError,
  setSkillEnabled,
  type TauriAppError,
} from "../../lib/tauri";
import {
  initialSkillsToggleState,
  mapSkillsError,
  skillsToggleReducer,
} from "./skillsState";

const t = getStrings("skills");

/** Maps an IPC rejection to its Korean message (stable code based). */
function errorText(err: unknown): string {
  const appError: TauriAppError = normalizeAppError(err);
  return t.errors[mapSkillsError(appError)];
}

/** The enabled state label (fail-soft: null → unknown). */
function stateLabel(enabled: boolean | null | undefined): string {
  if (enabled === true) return t.enabled;
  if (enabled === false) return t.disabled;
  return t.stateUnknown;
}

/** The eligibility badge label (fail-soft: null → unknown). */
function eligibleLabel(eligible: boolean | null | undefined): string {
  if (eligible === true) return t.eligible;
  if (eligible === false) return t.ineligible;
  return t.stateUnknown;
}

export function SkillsFeature() {
  const [skills, setSkills] = useState<SkillRow[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [toggle, dispatchToggle] = useReducer(
    skillsToggleReducer,
    initialSkillsToggleState,
  );

  const reload = useCallback(async () => {
    try {
      setSkills(await listSkills());
      setListError(null);
    } catch (err) {
      setListError(errorText(err));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // Re-query after every finished toggle (no optimistic updates).
  useEffect(() => {
    if (toggle.reloadCounter > 0) {
      void reload();
    }
  }, [toggle.reloadCounter, reload]);

  const toggleSkill = useCallback(
    (name: string, currentEnabled: boolean | null | undefined) => {
      if (toggle.pending !== null) {
        return;
      }
      const target = !(currentEnabled ?? true);
      dispatchToggle({ type: "start", key: name });
      setSkillEnabled(name, target)
        .then(() => dispatchToggle({ type: "finish", error: null }))
        .catch((err) => dispatchToggle({ type: "finish", error: errorText(err) }));
    },
    [toggle.pending],
  );

  if (skills === null) {
    return (
      <section>
        <h2>{t.title}</h2>
        <p>{t.loading}</p>
        {listError !== null && <p role="alert">{listError}</p>}
      </section>
    );
  }

  const anyIneligible = skills.some((skill) => skill.eligible === false);

  return (
    <section>
      <h2>{t.title}</h2>
      <p role="note">{t.sessionNote}</p>
      {listError !== null && <p role="alert">{listError}</p>}
      {toggle.error !== null && <p role="alert">{toggle.error}</p>}

      {skills.length === 0 ? (
        <p>{t.noSkills}</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>{t.name}</th>
              <th>{t.description}</th>
              <th>{t.status}</th>
              <th>{t.eligibleColumn}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {skills.map((skill) => {
              const isEnabled = skill.enabled ?? true;
              const busy = toggle.pending !== null;
              return (
                <tr key={skill.name}>
                  <td>{skill.name}</td>
                  <td>{skill.description ?? "—"}</td>
                  <td>{stateLabel(skill.enabled)}</td>
                  <td>{eligibleLabel(skill.eligible)}</td>
                  <td>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => toggleSkill(skill.name, skill.enabled)}
                    >
                      {toggle.pending === skill.name
                        ? t.toggling
                        : isEnabled
                          ? t.disable
                          : t.enable}
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      {anyIneligible && <p role="note">{t.ineligibleNote}</p>}
    </section>
  );
}
