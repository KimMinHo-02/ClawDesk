/**
 * Channels feature (Phase 6): Discord / Telegram token lifecycle, connect,
 * enable toggle, DM/group access policy, and pairing requests.
 *
 * All OS/OpenClaw work goes through the Tauri IPC wrappers (`src/lib/tauri`)
 * — the component never touches processes (S1/S10). No optimistic updates:
 * after every finished mutation (success or failure) the actual state is
 * re-queried. One mutation runs at a time (duplicate-submit guard).
 *
 * The channel token value travels to Rust only (S7): it is never kept in
 * feature state after a successful submit and is never displayed.
 */

import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { getStrings } from "../../i18n/ko";
import {
  DM_POLICIES,
  GROUP_POLICIES,
  approvePairing,
  connectChannel,
  deleteChannelToken,
  getChannelConfig,
  getChannels,
  listPairingRequests,
  normalizeAppError,
  setChannelEnabled,
  setChannelToken,
  setDmAccess,
  setGroupPolicy,
  type ChannelConfig,
  type ChannelsOverview,
  type TauriAppError,
} from "../../lib/tauri";
import {
  CHANNEL_IDS,
  type ChannelId,
  type ChannelsOpKind,
  dmAccessDraftDirty,
  dmAccessDraftFromConfig,
  type DmAccessDraft,
  type DmAccessDraftAction,
  dmAccessDraftReducer,
  groupPolicyDirty,
  initialChannelsOpState,
  initialPairingState,
  isDmAccessConsistent,
  isKnownDmPolicy,
  isKnownGroupPolicy,
  isValidAllowFromEntry,
  isValidPairingCode,
  mapChannelsError,
  pairingReducer,
  runtimeStateLabel,
  type PairingState,
  channelsOpReducer,
  tokenView,
} from "./channelsState";

const t = getStrings("channels");

const EMPTY_CONFIG: ChannelConfig = {
  enabled: null,
  tokenState: "absent",
  dmPolicy: null,
  allowFrom: [],
  groupPolicy: null,
};

/** Maps an IPC rejection to its Korean message (stable code based). */
function errorText(err: unknown): string {
  const appError: TauriAppError = normalizeAppError(err);
  return t.errors[mapChannelsError(appError)];
}

/** Korean channel display name. */
function channelName(id: string): string {
  return id === "discord" ? t.channelDiscord : t.channelTelegram;
}

export function ChannelsFeature() {
  const [overview, setOverview] = useState<ChannelsOverview | null>(null);
  const [configs, setConfigs] = useState<Record<ChannelId, ChannelConfig> | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [op, dispatchOp] = useReducer(channelsOpReducer, initialChannelsOpState);
  const [drafts, setDrafts] = useState<Record<ChannelId, DmAccessDraft>>(() => ({
    discord: dmAccessDraftFromConfig(EMPTY_CONFIG),
    telegram: dmAccessDraftFromConfig(EMPTY_CONFIG),
  }));
  const [groupDrafts, setGroupDrafts] = useState<Record<ChannelId, string>>(() => ({
    discord: "allowlist",
    telegram: "allowlist",
  }));
  const [tokenInputs, setTokenInputs] = useState<Record<ChannelId, string>>(() => ({
    discord: "",
    telegram: "",
  }));
  const [pairing, setPairing] = useState<Record<ChannelId, PairingState>>(() => ({
    discord: initialPairingState,
    telegram: initialPairingState,
  }));
  const [pairingCodeInputs, setPairingCodeInputs] = useState<Record<ChannelId, string>>(() => ({
    discord: "",
    telegram: "",
  }));
  const pairingBusy = useRef<Record<string, boolean>>({});

  /** Re-queries the actual state (overview + per-channel configs). */
  const refresh = useCallback(async () => {
    try {
      const [ov, discord, telegram] = await Promise.all([
        getChannels(),
        getChannelConfig("discord"),
        getChannelConfig("telegram"),
      ]);
      setOverview(ov);
      setConfigs({ discord, telegram });
      setLoadError(null);
      // Re-seed the DM drafts from the committed state (in-progress input
      // is kept by the reducer).
      setDrafts((prev) => ({
        discord: dmAccessDraftReducer(prev.discord, { type: "load", config: discord }),
        telegram: dmAccessDraftReducer(prev.telegram, { type: "load", config: telegram }),
      }));
      // Group drafts keep the user's in-progress selection; seed only once.
      setGroupDrafts((prev) => ({
        discord: prev.discord ?? discord.groupPolicy ?? "allowlist",
        telegram: prev.telegram ?? telegram.groupPolicy ?? "allowlist",
      }));
      setTokenInputs((prev) => ({
        discord: prev.discord ?? "",
        telegram: prev.telegram ?? "",
      }));
    } catch (err) {
      setLoadError(errorText(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Re-query the actual state after every finished mutation (no optimistic
  // updates — the CLI is the source of truth).
  useEffect(() => {
    if (op.reloadCounter > 0) {
      void refresh();
    }
  }, [op.reloadCounter, refresh]);

  const dispatchDraft = useCallback((channel: ChannelId, action: DmAccessDraftAction) => {
    setDrafts((prev) => ({
      ...prev,
      [channel]: dmAccessDraftReducer(prev[channel], action),
    }));
  }, []);

  /** Loads the pairing requests for one channel (single-flight, fail-closed). */
  const loadPairing = useCallback((channel: ChannelId) => {
    if (pairingBusy.current[channel]) {
      return;
    }
    pairingBusy.current[channel] = true;
    setPairing((prev) => ({
      ...prev,
      [channel]: pairingReducer(prev[channel], { type: "start" }),
    }));
    listPairingRequests(channel)
      .then((requests) =>
        setPairing((prev) => ({
          ...prev,
          [channel]: pairingReducer(prev[channel], { type: "finish", requests, error: null }),
        })),
      )
      .catch((err) =>
        setPairing((prev) => ({
          ...prev,
          [channel]: pairingReducer(prev[channel], {
            type: "finish",
            requests: null,
            error: errorText(err),
          }),
        })),
      )
      .finally(() => {
        pairingBusy.current[channel] = false;
      });
  }, []);

  /** Runs one mutation with the global single-flight guard. */
  const runOp = useCallback(
    (
      kind: ChannelsOpKind,
      channel: string,
      run: () => Promise<void>,
      hooks?: { onSuccess?: () => void; after?: () => void },
    ) => {
      if (op.pending !== null) {
        return;
      }
      dispatchOp({ type: "start", kind, channel });
      void run()
        .then(() => {
          dispatchOp({ type: "finish", error: null });
          hooks?.onSuccess?.();
        })
        .catch((err) => dispatchOp({ type: "finish", error: errorText(err) }))
        .finally(() => {
          hooks?.after?.();
        });
    },
    [op.pending],
  );

  const saveToken = useCallback(
    (channel: ChannelId) => {
      const token = tokenInputs[channel];
      if (token.trim().length === 0) {
        return;
      }
      runOp("token", channel, () => setChannelToken(channel, token), {
        onSuccess: () =>
          setTokenInputs((prev) => ({ ...prev, [channel]: "" })),
      });
    },
    [runOp, tokenInputs],
  );

  const deleteToken = useCallback(
    (channel: ChannelId) => {
      // eslint-disable-next-line no-alert -- confirmation before destructive action
      if (!window.confirm(`${t.tokenDeleteConfirm} (${channelName(channel)})`)) {
        return;
      }
      runOp("token", channel, () => deleteChannelToken(channel));
    },
    [runOp],
  );

  const connect = useCallback(
    (channel: ChannelId) => {
      runOp("connect", channel, () => connectChannel(channel));
    },
    [runOp],
  );

  const toggleEnabled = useCallback(
    (channel: ChannelId, enabled: boolean) => {
      // Confirmation only for the disable direction (enabled -> disabled).
      if (enabled) {
        // eslint-disable-next-line no-alert -- confirmation before disabling the channel
        if (!window.confirm(`${t.disableConfirm} (${channelName(channel)})`)) {
          return;
        }
      }
      runOp("enabled", channel, () => setChannelEnabled(channel, !enabled));
    },
    [runOp],
  );

  const saveDmAccess = useCallback(
    (channel: ChannelId) => {
      const config = configs?.[channel];
      const draft = drafts[channel];
      if (config === undefined || !dmAccessDraftDirty(draft, config)) {
        return;
      }
      if (!isDmAccessConsistent(draft.dmPolicy, draft.allowFrom)) {
        return;
      }
      runOp("dm-access", channel, () => setDmAccess(channel, draft.dmPolicy, draft.allowFrom));
    },
    [runOp, configs, drafts],
  );

  const saveGroupPolicy = useCallback(
    (channel: ChannelId) => {
      const config = configs?.[channel];
      const value = groupDrafts[channel];
      if (config === undefined || !groupPolicyDirty(value, config)) {
        return;
      }
      runOp("group-policy", channel, () => setGroupPolicy(channel, value));
    },
    [runOp, configs, groupDrafts],
  );

  const approve = useCallback(
    (channel: ChannelId, code: string) => {
      runOp("approve-pairing", channel, () => approvePairing(channel, code), {
        // Clear the manual code input after a successful approval (harmless
        // for the list-based buttons — it only empties the input field).
        onSuccess: () => setPairingCodeInputs((prev) => ({ ...prev, [channel]: "" })),
        after: () => loadPairing(channel),
      });
    },
    [runOp, loadPairing],
  );

  const loading = overview === null || configs === null;

  if (loading) {
    return (
      <section>
        <h2>{t.title}</h2>
        <p>{t.loading}</p>
        {loadError !== null && <p role="alert">{loadError}</p>}
      </section>
    );
  }

  const overviewLoaded = overview;
  const configsLoaded = configs;

  return (
    <section>
      <h2>{t.title}</h2>
      {loadError !== null && <p role="alert">{loadError}</p>}
      {overviewLoaded.gatewayReachable ? (
        <p>{t.autoApplyNote}</p>
      ) : (
        <p>{t.configOnlyNote}</p>
      )}
      {op.error !== null && <p role="alert">{op.error}</p>}

      {CHANNEL_IDS.map((channel) => {
        const row =
          overviewLoaded.channels.find((c) => c.id === channel) ?? {
            id: channel,
            installed: false,
            configured: false,
            enabled: false,
            runtimeState: null,
          };
        const config = configsLoaded[channel];
        const draft = drafts[channel];
        const groupDraft = groupDrafts[channel];
        const view = tokenView(config);
        const tokenInput = tokenInputs[channel];
        const pairingState = pairing[channel];
        const busy = op.pending !== null;
        const dmDirty = dmAccessDraftDirty(draft, config);
        const dmConsistent = isDmAccessConsistent(draft.dmPolicy, draft.allowFrom);
        const groupDirty = groupPolicyDirty(groupDraft, config);

        return (
          <div key={channel}>
            <h3>{channelName(channel)}</h3>
            <p>
              <span>{row.installed ? t.installed : t.notInstalled}</span> ·{" "}
              <span>{row.configured ? t.configured : t.notConfigured}</span> ·{" "}
              <span>{row.enabled ? t.enabled : t.disabled}</span> ·{" "}
              <span>
                {t.runtime}:{" "}
                {row.runtimeState === null
                  ? t.runtimeHidden
                  : runtimeStateLabel(row.runtimeState) === "connected"
                    ? t.runtimeConnected
                    : t.runtimeUnknown}
              </span>
            </p>

            {/* --- Token --- */}
            <h4>
              {t.token}:{" "}
              {view === "managed" ? (
                t.tokenManaged
              ) : view === "external" ? (
                t.tokenExternal
              ) : (
                t.tokenAbsent
              )}
            </h4>
            {view === "external" ? (
              <p>{t.tokenExternalHint}</p>
            ) : (
              <p>
                <input
                  type="password"
                  aria-label={t.tokenInput}
                  placeholder="••••••••"
                  value={tokenInput}
                  disabled={busy}
                  onChange={(e) =>
                    setTokenInputs((prev) => ({ ...prev, [channel]: e.target.value }))
                  }
                />{" "}
                <button
                  type="button"
                  disabled={busy || tokenInput.trim().length === 0}
                  onClick={() => saveToken(channel)}
                >
                  {op.pending !== null && op.pending.kind === "token" && op.pending.channel === channel
                    ? t.tokenSaving
                    : t.tokenSave}
                </button>{" "}
                {view === "managed" && (
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => deleteToken(channel)}
                  >
                    {t.tokenDelete}
                  </button>
                )}
              </p>
            )}
            <p>
              <em>{t.tokenHint}</em>
              {tokenInput.trim().length === 0 && view !== "external" && (
                <span role="alert"> {t.tokenInvalid}</span>
              )}
            </p>

            {/* --- Connect / enabled --- */}
            <p>
              <button
                type="button"
                disabled={busy || view !== "managed"}
                onClick={() => connect(channel)}
              >
                {op.pending !== null && op.pending.kind === "connect" && op.pending.channel === channel
                  ? t.connecting
                  : t.connect}
              </button>{" "}
              {view !== "managed" && <em>{t.connectDisabledHint}</em>}
              {row.configured && (
                <>
                  {" "}
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => toggleEnabled(channel, row.enabled)}
                  >
                    {row.enabled ? t.disable : t.enable}
                  </button>
                </>
              )}
            </p>

            {/* --- DM access --- */}
            <h4>{t.dmAccess}</h4>
            <p>
              <label>
                {t.dmPolicyLabel}:{" "}
                <select
                  value={draft.dmPolicy}
                  disabled={busy}
                  onChange={(e) =>
                    dispatchDraft(channel, { type: "set-policy", value: e.target.value })
                  }
                >
                  {!isKnownDmPolicy(draft.dmPolicy) && (
                    <option value={draft.dmPolicy}>{t.dmPolicyUnknown}</option>
                  )}
                  {DM_POLICIES.map((p) => (
                    <option key={p} value={p}>
                      {t.dmPolicies[p]}
                    </option>
                  ))}
                </select>
              </label>{" "}
              {config.dmPolicy === null && <em>{t.dmUnsetNote}</em>}
            </p>
            <p>
              <span>{t.allowFromTitle}</span> <em>({t.allowFromHint})</em>
            </p>
            <ul>
              {draft.allowFrom.length === 0 ? (
                <li>{t.emptyList}</li>
              ) : (
                draft.allowFrom.map((entry) => (
                  <li key={entry}>
                    {entry}{" "}
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => dispatchDraft(channel, { type: "remove-entry", entry })}
                    >
                      {t.remove}
                    </button>
                  </li>
                ))
              )}
            </ul>
            <p>
              <input
                aria-label={t.allowFromTitle}
                placeholder={t.allowFromPlaceholder}
                value={draft.input}
                disabled={busy}
                onChange={(e) => dispatchDraft(channel, { type: "set-input", value: e.target.value })}
              />{" "}
              <button
                type="button"
                disabled={
                  busy ||
                  !isValidAllowFromEntry(draft.input.trim()) ||
                  draft.allowFrom.includes(draft.input.trim())
                }
                onClick={() => dispatchDraft(channel, { type: "add-entry" })}
              >
                {t.add}
              </button>{" "}
              <button
                type="button"
                disabled={busy || !dmDirty || !dmConsistent}
                onClick={() => saveDmAccess(channel)}
              >
                {op.pending !== null &&
                op.pending.kind === "dm-access" &&
                op.pending.channel === channel
                  ? t.saving
                  : t.saveDmAccess}
              </button>
            </p>
            {draft.input.trim() !== "" && !isValidAllowFromEntry(draft.input.trim()) && (
              <p role="alert">{t.allowFromInvalid}</p>
            )}
            {dmDirty && !dmConsistent && <p role="alert">{t.dmAccessInconsistent}</p>}

            {/* --- Group policy --- */}
            <p>
              <label>
                {t.groupPolicyLabel}:{" "}
                <select
                  value={groupDraft}
                  disabled={busy}
                  onChange={(e) =>
                    setGroupDrafts((prev) => ({ ...prev, [channel]: e.target.value }))
                  }
                >
                  {!isKnownGroupPolicy(groupDraft) && (
                    <option value={groupDraft}>{t.groupPolicyUnknown}</option>
                  )}
                  {GROUP_POLICIES.map((p) => (
                    <option key={p} value={p}>
                      {t.groupPolicies[p]}
                    </option>
                  ))}
                </select>
              </label>{" "}
              {config.groupPolicy === null && <em>{t.groupUnsetNote}</em>}{" "}
              <button
                type="button"
                disabled={busy || !groupDirty}
                onClick={() => saveGroupPolicy(channel)}
              >
                {op.pending !== null &&
                op.pending.kind === "group-policy" &&
                op.pending.channel === channel
                  ? t.saving
                  : t.saveGroupPolicy}
              </button>
            </p>

            {/* --- Pairing --- */}
            <h4>{t.pairing}</h4>
            <p>{t.pairingHint}</p>
            <p>{t.pairingOwnerNote}</p>
            <p>
              <button type="button" disabled={busy || pairingState.loading} onClick={() => loadPairing(channel)}>
                {pairingState.loading ? t.pairingLoading : t.pairingLoad}
              </button>
            </p>
            {pairingState.error !== null && <p role="alert">{pairingState.error}</p>}
            {pairingState.requests !== null &&
              (pairingState.requests.length === 0 ? (
                <p>{t.pairingEmpty}</p>
              ) : (
                <ul>
                  {pairingState.requests.map((request) => (
                    <li key={request.code}>
                      {request.code} —{" "}
                      {request.sender === null || request.sender === ""
                        ? t.pairingSenderUnknown
                        : request.sender}{" "}
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => approve(channel, request.code)}
                      >
                        {t.pairingApprove}
                      </button>
                    </li>
                  ))}
                </ul>
              ))}
            <p>
              <input
                aria-label={t.pairingCodeInput}
                placeholder={t.pairingCodeInput}
                value={pairingCodeInputs[channel]}
                disabled={busy}
                onChange={(e) =>
                  setPairingCodeInputs((prev) => ({ ...prev, [channel]: e.target.value }))
                }
              />{" "}
              <button
                type="button"
                disabled={busy || !isValidPairingCode(pairingCodeInputs[channel].trim())}
                onClick={() => approve(channel, pairingCodeInputs[channel].trim())}
              >
                {t.pairingApprove}
              </button>
            </p>
            {pairingCodeInputs[channel].trim() !== "" &&
              !isValidPairingCode(pairingCodeInputs[channel].trim()) && (
                <p role="alert">{t.pairingCodeInvalid}</p>
              )}
          </div>
        );
      })}
    </section>
  );
}
