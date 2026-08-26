/**
 * Profile feature (Phase 8): agents / update / gateway / diagnostics.
 *
 * Read-only display (PRODUCT_CONTRACT §4.7). All OS/OpenClaw work goes
 * through the Tauri IPC wrappers (`src/lib/tauri`) — the component never
 * touches processes (S1/S10). Each section is loaded independently with
 * its own loading/error/data state and a refresh action; there are no
 * optimistic updates and no mutations (no `--follow`, no lifecycle).
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { getStrings } from "../../i18n/ko";
import {
  detectEnvironment,
  getAgents,
  getGatewayStatus,
  getLogs,
  getUpdateStatus,
  normalizeAppError,
  type AgentRow,
  type EnvironmentReport,
  type GatewayStatus,
  type LogsResult,
  type TauriAppError,
  type UpdateState,
  type UpdateStatusDetail,
  type WindowsVersionInfo,
} from "../../lib/tauri";
import {
  DEFAULT_LOG_LIMIT,
  LOG_LIMIT_OPTIONS,
  formatLogEvent,
  mapProfileError,
} from "./profileState";

const t = getStrings("profile");

/** Maps an IPC rejection to its Korean message (stable code based). */
function errorText(err: unknown): string {
  const appError: TauriAppError = normalizeAppError(err);
  return t.errors[mapProfileError(appError)];
}

/**
 * One read-only section: loads on mount, then only on an explicit refresh.
 * The loader is kept in a ref so dependency changes (e.g. the log limit)
 * do not trigger an automatic reload.
 */
function useReadonlySection<T>(loader: () => Promise<T>) {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const loaderRef = useRef(loader);
  loaderRef.current = loader;

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setData(await loaderRef.current());
    } catch (err) {
      setError(errorText(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return { data, loading, error, reload };
}

/** The update state label (fail-soft: unknown → 미확인). */
function updateStateLabel(state: UpdateState): string {
  switch (state) {
    case "updated":
      return t.updateStateUpdated;
    case "update-available":
      return t.updateStateAvailable;
    case "unknown":
      return t.updateStateUnknown;
  }
}

/** The OS summary line (product name + build; fail-soft on the name). */
function envOsLabel(windows: WindowsVersionInfo): string {
  const name = windows.product_name ?? `Windows ${windows.major_version}`;
  return `${name} (build ${windows.build})`;
}

/** A section header with its refresh action. */
function SectionHeader({
  title,
  onRefresh,
  disabled,
}: {
  title: string;
  onRefresh: () => void;
  disabled: boolean;
}) {
  return (
    <div>
      <h2>{title}</h2>
      <button type="button" onClick={onRefresh} disabled={disabled}>
        {t.refresh}
      </button>
    </div>
  );
}

export function ProfileFeature() {
  const agents = useReadonlySection<AgentRow[]>(() => getAgents());
  const update = useReadonlySection<UpdateStatusDetail>(() => getUpdateStatus());
  const gateway = useReadonlySection<GatewayStatus>(() => getGatewayStatus());
  const environment = useReadonlySection<EnvironmentReport>(() => detectEnvironment());

  const [limit, setLimit] = useState<number>(DEFAULT_LOG_LIMIT);
  const logs = useReadonlySection<LogsResult>(() => getLogs(limit));

  return (
    <section>
      <h1>{t.title}</h1>

      {/* --- Profile / agents (read-only) --- */}
      <SectionHeader
        title={t.agentsTitle}
        onRefresh={() => void agents.reload()}
        disabled={agents.loading}
      />
      {agents.error !== null && <p role="alert">{agents.error}</p>}
      {agents.data === null ? (
        <p>{t.loading}</p>
      ) : agents.data.length === 0 ? (
        <p>{t.agentsNoAgents}</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>{t.agentsColumnId}</th>
              <th>{t.agentsColumnName}</th>
              <th>{t.agentsColumnDefault}</th>
              <th>{t.agentsColumnWorkspace}</th>
              <th>{t.agentsColumnBindings}</th>
            </tr>
          </thead>
          <tbody>
            {agents.data.map((row) => (
              <tr key={row.id}>
                <td>{row.id}</td>
                <td>
                  {row.emoji ? `${row.emoji} ` : ""}
                  {row.name ?? t.unknown}
                </td>
                <td>{row.default ? t.agentsDefaultBadge : "—"}</td>
                <td>{row.workspace ?? "—"}</td>
                <td>{row.bindings ?? "—"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {/* --- Update status --- */}
      <SectionHeader
        title={t.updateTitle}
        onRefresh={() => void update.reload()}
        disabled={update.loading}
      />
      {update.error !== null && <p role="alert">{update.error}</p>}
      {update.data === null ? (
        <p>{t.loading}</p>
      ) : (
        <dl>
          <div>
            <dt>{t.updateState}</dt>
            <dd>{updateStateLabel(update.data.state)}</dd>
          </div>
          <div>
            <dt>{t.updateCurrent}</dt>
            <dd>{update.data.current ?? t.versionUnknown}</dd>
          </div>
          <div>
            <dt>{t.updateLatest}</dt>
            <dd>{update.data.latest ?? t.versionUnknown}</dd>
          </div>
        </dl>
      )}
      <p role="note">{t.updateNote}</p>

      {/* --- API (gateway) status --- */}
      <SectionHeader
        title={t.gatewayTitle}
        onRefresh={() => void gateway.reload()}
        disabled={gateway.loading}
      />
      {gateway.error !== null && <p role="alert">{gateway.error}</p>}
      {gateway.data === null ? (
        <p>{t.loading}</p>
      ) : (
        <dl>
          <div>
            <dt>{t.gatewayState}</dt>
            <dd>{gateway.data.state}</dd>
          </div>
          <div>
            <dt>{t.gatewayVersion}</dt>
            <dd>{gateway.data.version ?? t.versionUnknown}</dd>
          </div>
          <div>
            <dt>{t.gatewayPort}</dt>
            <dd>{gateway.data.port ?? t.versionUnknown}</dd>
          </div>
        </dl>
      )}
      <p role="note">{t.gatewayNote}</p>

      {/* --- Diagnostics: environment summary + log viewer --- */}
      <SectionHeader
        title={t.diagnosticsTitle}
        onRefresh={() => {
          void environment.reload();
          void logs.reload();
        }}
        disabled={environment.loading || logs.loading}
      />
      {environment.error !== null && <p role="alert">{environment.error}</p>}
      {environment.data === null ? (
        <p>{t.loading}</p>
      ) : (
        <dl>
          <div>
            <dt>{t.envOs}</dt>
            <dd>{envOsLabel(environment.data.windows_version)}</dd>
          </div>
          <div>
            <dt>{t.envArchitecture}</dt>
            <dd>{environment.data.architecture}</dd>
          </div>
          <div>
            <dt>{t.envNode}</dt>
            <dd>
              {environment.data.node.status === "found"
                ? environment.data.node.version
                : t.envNotInstalled}
            </dd>
          </div>
          <div>
            <dt>{t.envOpenClaw}</dt>
            <dd>
              {environment.data.openclaw.status === "detected"
                ? (environment.data.openclaw.version ?? t.versionUnknown)
                : t.envNotInstalled}
            </dd>
          </div>
        </dl>
      )}

      <h3>{t.logsTitle}</h3>
      <p role="note">{t.logsHint}</p>
      <label>
        {t.logsLimitLabel}{" "}
        <select
          value={limit}
          onChange={(event) => setLimit(Number(event.target.value))}
        >
          {LOG_LIMIT_OPTIONS.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
      </label>
      <button type="button" onClick={() => void logs.reload()} disabled={logs.loading}>
        {logs.loading ? t.logsRefreshing : t.logsRefresh}
      </button>
      {logs.error !== null && <p role="alert">{logs.error}</p>}
      {logs.data === null ? (
        <p>{t.logsLoading}</p>
      ) : logs.data.lines.length === 0 ? (
        <p>{t.logsEmpty}</p>
      ) : (
        <pre>{logs.data.lines.map((line) => formatLogEvent(line)).join("\n")}</pre>
      )}
      {logs.data?.source && (
        <p role="note">
          {t.logsSource}: {logs.data.source}
        </p>
      )}
      {logs.data?.truncated && <p role="note">{t.logsTruncatedNote}</p>}
    </section>
  );
}
