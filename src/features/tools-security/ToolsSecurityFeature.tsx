/**
 * Tools / Security feature (Phase 5): tool policy (allow/deny/exec mode),
 * security profiles (builtin + user, apply), and the read-only security
 * audit.
 *
 * All OS/OpenClaw work goes through the Tauri IPC wrappers (`src/lib/tauri`)
 * — the component never touches processes (S1/S10). No optimistic updates:
 * after every finished mutation (success or failure) the actual state is
 * re-queried. The audit is fail-closed: a failed run never keeps a stale
 * "clean" result.
 */

import { useCallback, useEffect, useReducer, useState } from "react";
import { getStrings } from "../../i18n/ko";
import {
  EXEC_MODES,
  TOOL_PROFILES,
  applySecurityProfile,
  deleteSecurityProfile,
  getToolPolicy,
  listSecurityProfiles,
  normalizeAppError,
  runSecurityAudit,
  saveSecurityProfile,
  setExecMode,
  setToolAllow,
  setToolDeny,
  setToolProfile,
  type ExecMode,
  type SecurityProfile,
  type SecurityProfileList,
  type TauriAppError,
  type ToolPolicy,
  type ToolProfile,
} from "../../lib/tauri";
import {
  auditCategory,
  auditReducer,
  initialAuditState,
  initialPolicyDraft,
  initialPolicyMutationState,
  initialProfileActionState,
  initialProfileForm,
  isValidToolEntry,
  mapToolsSecurityError,
  policyDraftDirty,
  policyDraftReducer,
  policyMutationReducer,
  profileActionReducer,
  profileFormErrors,
  profileFormReducer,
  severityKey,
} from "./toolsSecurityState";

const t = getStrings("toolsSecurity");

/** Maps an IPC rejection to its Korean message (stable code based). */
function errorText(err: unknown): string {
  const appError: TauriAppError = normalizeAppError(err);
  return t.errors[mapToolsSecurityError(appError)];
}

/** Korean label for a (possibly unknown-raw) profile enum value. */
function profileLabel(value: string): string {
  return (t.profiles as Record<string, string>)[value] ?? t.profileUnknown;
}

/** Korean label for a (possibly unknown-raw) exec-mode enum value. */
function execModeLabel(value: string): string {
  return (t.execModes as Record<string, string>)[value] ?? t.execModeUnknown;
}

function readOnlyLabel(value: boolean | null): string {
  if (value === true) return t.on;
  if (value === false) return t.off;
  return t.unknown;
}

export function ToolsSecurityFeature() {
  const [policy, setPolicy] = useState<ToolPolicy | null>(null);
  const [policyError, setPolicyError] = useState<string | null>(null);
  const [draft, dispatchDraft] = useReducer(policyDraftReducer, initialPolicyDraft);
  const [mutation, dispatchMutation] = useReducer(
    policyMutationReducer,
    initialPolicyMutationState,
  );
  const [profileList, setProfileList] = useState<SecurityProfileList | null>(null);
  const [profilesError, setProfilesError] = useState<string | null>(null);
  const [profileAction, dispatchProfileAction] = useReducer(
    profileActionReducer,
    initialProfileActionState,
  );
  const [form, dispatchForm] = useReducer(profileFormReducer, initialProfileForm);
  const [audit, dispatchAudit] = useReducer(auditReducer, initialAuditState);

  const loadPolicy = useCallback(async () => {
    try {
      const loaded = await getToolPolicy();
      setPolicy(loaded);
      setPolicyError(null);
      // Re-sync the entry editors with the committed policy.
      dispatchDraft({ type: "load", allow: loaded.allow, deny: loaded.deny });
    } catch (err) {
      setPolicyError(errorText(err));
    }
  }, []);

  const loadProfiles = useCallback(async () => {
    try {
      setProfileList(await listSecurityProfiles());
      setProfilesError(null);
    } catch (err) {
      setProfilesError(errorText(err));
    }
  }, []);

  useEffect(() => {
    void loadPolicy();
    void loadProfiles();
  }, [loadPolicy, loadProfiles]);

  // Re-query the actual policy after every finished mutation (no optimistic
  // updates — the CLI is the source of truth).
  useEffect(() => {
    if (mutation.reloadCounter > 0) {
      void loadPolicy();
    }
  }, [mutation.reloadCounter, loadPolicy]);

  // Re-query the profile list after every finished profile action.
  useEffect(() => {
    if (profileAction.reloadCounter > 0) {
      void loadProfiles();
    }
  }, [profileAction.reloadCounter, loadProfiles]);

  // A finished apply changed the config → re-query the policy as well.
  useEffect(() => {
    if (profileAction.policyReloadCounter > 0) {
      void loadPolicy();
    }
  }, [profileAction.policyReloadCounter, loadPolicy]);

  const mutatePolicy = useCallback(
    (kind: "profile" | "exec-mode", run: () => Promise<void>) => {
      if (mutation.pending !== null) {
        return;
      }
      dispatchMutation({ type: "start", kind });
      run()
        .then(() => dispatchMutation({ type: "finish", error: null }))
        .catch((err) => dispatchMutation({ type: "finish", error: errorText(err) }));
    },
    [mutation.pending],
  );

  const selectProfile = useCallback(
    (value: ToolProfile) => {
      mutatePolicy("profile", () => setToolProfile(value));
    },
    [mutatePolicy],
  );

  const selectExecMode = useCallback(
    (value: ExecMode) => {
      mutatePolicy("exec-mode", () => setExecMode(value));
    },
    [mutatePolicy],
  );

  /** Saves the allow/deny chip editors (one pending period, re-query after). */
  const saveEntries = useCallback(() => {
    if (mutation.pending !== null || policy === null) {
      return;
    }
    const key = (entries: string[]) => entries.join("\u0000");
    const allowChanged = key(draft.allow) !== key(policy.allow);
    const denyChanged = key(draft.deny) !== key(policy.deny);
    if (!allowChanged && !denyChanged) {
      return;
    }
    dispatchMutation({ type: "start", kind: "allow" });
    // Sequential writes (same config file — no concurrent dry-run/commit).
    const ops: Array<() => Promise<void>> = [];
    if (allowChanged) {
      ops.push(() => setToolAllow(draft.allow));
    }
    if (denyChanged) {
      ops.push(() => setToolDeny(draft.deny));
    }
    ops
      .reduce<Promise<void>>((acc, op) => acc.then(() => op()), Promise.resolve())
      .then(() => dispatchMutation({ type: "finish", error: null }))
      .catch((err) => dispatchMutation({ type: "finish", error: errorText(err) }));
  }, [mutation.pending, policy, draft.allow, draft.deny]);

  const actionProfile = useCallback(
    (kind: "save" | "apply" | "delete", id: string, run: () => Promise<void>) => {
      if (profileAction.pending !== null) {
        return;
      }
      dispatchProfileAction({ type: "start", kind, id });
      run()
        .then(() => dispatchProfileAction({ type: "finish", kind, error: null }))
        .catch((err) =>
          dispatchProfileAction({ type: "finish", kind, error: errorText(err) }),
        );
    },
    [profileAction.pending],
  );

  const applyProfile = useCallback(
    (id: string) => {
      actionProfile("apply", id, () => applySecurityProfile(id));
    },
    [actionProfile],
  );

  const deleteProfile = useCallback(
    (profile: SecurityProfile) => {
      // eslint-disable-next-line no-alert -- confirmation before destructive action
      if (!window.confirm(`${t.deleteProfileConfirm} (${profile.id})`)) {
        return;
      }
      actionProfile("delete", profile.id, () => deleteSecurityProfile(profile.id));
    },
    [actionProfile],
  );

  const formErrors = form.mode !== null ? profileFormErrors(form) : [];
  const formValid = formErrors.length === 0;

  const saveProfile = useCallback(() => {
    if (form.mode === null || !formValid || profileAction.pending !== null) {
      return;
    }
    const profile: SecurityProfile = {
      id: form.id,
      name: form.name,
      baseProfile: form.baseProfile,
      allow: form.allow,
      deny: form.deny,
      execMode: form.execMode,
    };
    actionProfile("save", form.id, () => saveSecurityProfile(profile));
    dispatchForm({ type: "close" });
  }, [form, formValid, profileAction.pending]);

  const runAudit = useCallback(() => {
    if (audit.running) {
      return;
    }
    dispatchAudit({ type: "start" });
    runSecurityAudit()
      .then((result) => dispatchAudit({ type: "finish", result, error: null }))
      .catch((err) => dispatchAudit({ type: "finish", result: null, error: errorText(err) }));
  }, [audit.running]);

  const loading = policy === null || profileList === null;

  if (loading) {
    return (
      <section>
        <h2>{t.title}</h2>
        <p>{t.loading}</p>
        {policyError !== null && <p role="alert">{policyError}</p>}
        {profilesError !== null && <p role="alert">{profilesError}</p>}
      </section>
    );
  }

  const policyLoaded = policy;
  const profilesLoaded = profileList;
  const profilePending = profileAction.pending;
  const dirty = policyDraftDirty(draft, policyLoaded);

  return (
    <section>
      <h2>{t.title}</h2>
      {policyError !== null && <p role="alert">{policyError}</p>}
      {profilesError !== null && <p role="alert">{profilesError}</p>}

      {/* --- Tool policy --- */}
      <h3>{t.policy}</h3>
      <p>{t.policyHint}</p>
      {mutation.error !== null && <p role="alert">{mutation.error}</p>}

      <p>
        <label htmlFor="tool-profile">
          {t.profileLabel}:{" "}
          <select
            id="tool-profile"
            value={policyLoaded.profile ?? "full"}
            disabled={mutation.pending !== null}
            onChange={(e) => selectProfile(e.target.value as ToolProfile)}
          >
            {policyLoaded.profile !== null &&
              !(TOOL_PROFILES as readonly string[]).includes(policyLoaded.profile) && (
                <option value={policyLoaded.profile}>{t.profileUnknown}</option>
              )}
            {TOOL_PROFILES.map((p) => (
              <option key={p} value={p}>
                {t.profiles[p]}
              </option>
            ))}
          </select>
        </label>
      </p>
      <p>
        <label htmlFor="exec-mode">
          {t.execModeLabel}:{" "}
          <select
            id="exec-mode"
            value={policyLoaded.execMode ?? "full"}
            disabled={mutation.pending !== null}
            onChange={(e) => selectExecMode(e.target.value as ExecMode)}
          >
            {policyLoaded.execMode !== null &&
              !(EXEC_MODES as readonly string[]).includes(policyLoaded.execMode) && (
                <option value={policyLoaded.execMode}>{t.execModeUnknown}</option>
              )}
            {EXEC_MODES.map((m) => (
              <option key={m} value={m}>
                {t.execModes[m]}
              </option>
            ))}
          </select>
        </label>
      </p>
      <p>
        <span>
          {t.elevated} ({readOnlyLabel(policyLoaded.elevatedEnabled)})
        </span>{" "}
        <span>
          {t.fsWorkspaceOnly} ({readOnlyLabel(policyLoaded.fsWorkspaceOnly)})
        </span>{" "}
        <em>{t.readOnly}</em>
      </p>

      {/* Allow / deny chip editors */}
      <h4>
        {t.allowTitle} <em>({t.denyWins})</em>
      </h4>
      <p>{t.entryHint}</p>
      <ul>
        {policyLoaded.allow.length === 0 ? (
          <li>{t.emptyList}</li>
        ) : (
          policyLoaded.allow.map((entry) => (
            <li key={`allow-${entry}`}>
              {entry}{" "}
              <button
                type="button"
                disabled={mutation.pending !== null}
                onClick={() => dispatchDraft({ type: "remove-allow", entry })}
              >
                {t.remove}
              </button>
            </li>
          ))
        )}
      </ul>
      <p>
        <input
          aria-label={t.allowTitle}
          value={draft.allowInput}
          placeholder={t.entryPlaceholder}
          disabled={mutation.pending !== null}
          onChange={(e) => dispatchDraft({ type: "set-allow-input", value: e.target.value })}
        />{" "}
        <button
          type="button"
          disabled={
            mutation.pending !== null ||
            !isValidToolEntry(draft.allowInput.trim()) ||
            policyLoaded.allow.includes(draft.allowInput.trim())
          }
          onClick={() => dispatchDraft({ type: "add-allow" })}
        >
          {t.add}
        </button>
      </p>

      <h4>{t.denyTitle}</h4>
      <ul>
        {policyLoaded.deny.length === 0 ? (
          <li>{t.emptyList}</li>
        ) : (
          policyLoaded.deny.map((entry) => (
            <li key={`deny-${entry}`}>
              {entry}{" "}
              <button
                type="button"
                disabled={mutation.pending !== null}
                onClick={() => dispatchDraft({ type: "remove-deny", entry })}
              >
                {t.remove}
              </button>
            </li>
          ))
        )}
      </ul>
      <p>
        <input
          aria-label={t.denyTitle}
          value={draft.denyInput}
          placeholder={t.entryPlaceholder}
          disabled={mutation.pending !== null}
          onChange={(e) => dispatchDraft({ type: "set-deny-input", value: e.target.value })}
        />{" "}
        <button
          type="button"
          disabled={
            mutation.pending !== null ||
            !isValidToolEntry(draft.denyInput.trim()) ||
            policyLoaded.deny.includes(draft.denyInput.trim())
          }
          onClick={() => dispatchDraft({ type: "add-deny" })}
        >
          {t.add}
        </button>
      </p>
      {draft.allowInput.trim() !== "" && !isValidToolEntry(draft.allowInput.trim()) && (
        <p role="alert">{t.entryInvalid}</p>
      )}
      {draft.denyInput.trim() !== "" && !isValidToolEntry(draft.denyInput.trim()) && (
        <p role="alert">{t.entryInvalid}</p>
      )}
      <p>
        <button
          type="button"
          disabled={mutation.pending !== null || !dirty}
          onClick={saveEntries}
        >
          {mutation.pending !== null ? t.saving : t.savePolicy}
        </button>
      </p>

      {/* --- Security profiles --- */}
      <h3>{t.profilesSection}</h3>
      {profileAction.error !== null && <p role="alert">{profileAction.error}</p>}
      {profilesLoaded.policyReadFailed ? (
        <p>{t.policyReadFailed}</p>
      ) : profilesLoaded.currentApplied === null ? (
        <p>{t.customBadge}</p>
      ) : null}

      <table>
        <thead>
          <tr>
            <th>{t.profileColumn}</th>
            <th>{t.baseProfile}</th>
            <th>{t.execMode}</th>
            <th>{t.status}</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {[...profilesLoaded.builtins, ...profilesLoaded.users].map((profile) => {
            const isBuiltin = profilesLoaded.builtins.some((b) => b.id === profile.id);
            const applied = profilesLoaded.currentApplied === profile.id;
            const busy = profilePending !== null && profilePending.id === profile.id;
            return (
              <tr key={profile.id}>
                <td>
                  {profile.name} ({profile.id}){" "}
                  {isBuiltin && <em>{t.builtinBadge}</em>}
                </td>
                <td>{profileLabel(profile.baseProfile)}</td>
                <td>{execModeLabel(profile.execMode)}</td>
                <td>{applied ? t.appliedBadge : ""}</td>
                <td>
                  <button
                    type="button"
                    disabled={profilePending !== null}
                    onClick={() => applyProfile(profile.id)}
                  >
                    {busy && profilePending?.kind === "apply" ? t.applying : t.apply}
                  </button>{" "}
                  {!isBuiltin && (
                    <>
                      <button
                        type="button"
                        disabled={profilePending !== null || form.mode !== null}
                        onClick={() =>
                          dispatchForm({ type: "open-edit", profile })
                        }
                      >
                        {t.editProfile}
                      </button>{" "}
                      <button
                        type="button"
                        disabled={profilePending !== null}
                        onClick={() => deleteProfile(profile)}
                      >
                        {busy && profilePending?.kind === "delete" ? t.deleting : t.delete}
                      </button>
                    </>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <p>
        <button
          type="button"
          disabled={profilePending !== null || form.mode !== null}
          onClick={() =>
            dispatchForm({
              type: "open-create",
              policy: policyLoaded,
              builtins: profilesLoaded.builtins,
            })
          }
        >
          {t.createProfile}
        </button>
      </p>

      {/* Profile create/edit form */}
      {form.mode !== null && (
        <div>
          <h4>{form.mode === "create" ? t.createProfile : t.editProfile}</h4>
          {form.mode === "create" && (
            <p>
              <label htmlFor="profile-source">
                {t.source}:{" "}
                <select
                  id="profile-source"
                  value={form.source}
                  onChange={(e) =>
                    dispatchForm({
                      type: "set-source",
                      source: e.target.value as typeof form.source,
                      policy: policyLoaded,
                      builtins: profilesLoaded.builtins,
                    })
                  }
                >
                  <option value="current">{t.sourceCurrent}</option>
                  {profilesLoaded.builtins.map((b) => (
                    <option key={b.id} value={`builtin-${b.id}`}>
                      {t.sourceBuiltin} {b.name}
                    </option>
                  ))}
                </select>
              </label>
            </p>
          )}
          <p>
            <label htmlFor="profile-id">
              {t.profileId}:{" "}
              <input
                id="profile-id"
                value={form.id}
                disabled={form.mode === "edit"}
                onChange={(e) => dispatchForm({ type: "set-id", value: e.target.value })}
              />
            </label>{" "}
            <em>{t.profileIdHint}</em>
          </p>
          <p>
            <label htmlFor="profile-name">
              {t.profileName}:{" "}
              <input
                id="profile-name"
                value={form.name}
                onChange={(e) => dispatchForm({ type: "set-name", value: e.target.value })}
              />
            </label>{" "}
            <em>{t.profileNameHint}</em>
          </p>
          <p>
            <label htmlFor="profile-base">
              {t.baseProfile}:{" "}
              <select
                id="profile-base"
                value={form.baseProfile}
                onChange={(e) =>
                  dispatchForm({ type: "set-base-profile", value: e.target.value })
                }
              >
                {TOOL_PROFILES.map((p) => (
                  <option key={p} value={p}>
                    {t.profiles[p]}
                  </option>
                ))}
              </select>
            </label>{" "}
            <label htmlFor="profile-exec">
              {t.execMode}:{" "}
              <select
                id="profile-exec"
                value={form.execMode}
                onChange={(e) =>
                  dispatchForm({ type: "set-exec-mode", value: e.target.value })
                }
              >
                {EXEC_MODES.map((m) => (
                  <option key={m} value={m}>
                    {t.execModes[m]}
                  </option>
                ))}
              </select>
            </label>
          </p>
          <p>
            <label htmlFor="profile-allow">
              {t.allowEntries}:{" "}
              <input
                id="profile-allow"
                value={form.allow.join(", ")}
                onChange={(e) => dispatchForm({ type: "set-allow-text", value: e.target.value })}
              />
            </label>{" "}
            <label htmlFor="profile-deny">
              {t.denyEntries}:{" "}
              <input
                id="profile-deny"
                value={form.deny.join(", ")}
                onChange={(e) => dispatchForm({ type: "set-deny-text", value: e.target.value })}
              />
            </label>{" "}
            <em>{t.entriesHint}</em>
          </p>
          {(formErrors.includes("allow") || formErrors.includes("deny")) && (
            <p role="alert">{t.entryInvalid}</p>
          )}
          <p>
            <button
              type="button"
              disabled={profilePending !== null || !formValid}
              onClick={saveProfile}
            >
              {profilePending !== null ? t.saving : t.save}
            </button>{" "}
            <button
              type="button"
              disabled={profilePending !== null}
              onClick={() => dispatchForm({ type: "close" })}
            >
              {t.cancel}
            </button>
          </p>
        </div>
      )}

      {/* --- Security audit --- */}
      <h3>{t.audit}</h3>
      <p>{t.auditHint}</p>
      <p>
        <button type="button" disabled={audit.running} onClick={runAudit}>
          {audit.running ? t.auditRunning : t.auditRun}
        </button>
      </p>
      {audit.error !== null && <p role="alert">{t.auditFailed}</p>}
      {audit.result !== null && (
        <div>
          {audit.result.findings.length === 0 ? (
            <p>{t.noFindings}</p>
          ) : (
            <ul>
              {audit.result.findings.map((finding, index) => (
                <li key={`${finding.checkId}-${index}`}>
                  <strong>{t.severity[severityKey(finding.severity)]}</strong>{" "}
                  <em>{t.categories[auditCategory(finding.checkId)]}</em> {finding.checkId}
                  {finding.title !== null && finding.title !== undefined && (
                    <> — {finding.title}</>
                  )}
                  {finding.detail !== null && finding.detail !== undefined && (
                    <div>{t.detail}: {finding.detail}</div>
                  )}
                </li>
              ))}
            </ul>
          )}
          <p>
            {t.suppressedCount}: {audit.result.suppressedCount}
          </p>
        </div>
      )}
    </section>
  );
}
