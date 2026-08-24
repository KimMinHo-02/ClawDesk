/**
 * Plugins feature (Phase 4): plugin list + enable/disable toggle +
 * on-demand runtime state display.
 *
 * All OS/OpenClaw work goes through the Tauri IPC wrappers
 * (`src/lib/tauri`) — the component never touches processes (S1/S10).
 * Toggles are non-optimistic: after every toggle (success or failure) the
 * list is re-queried. The runtime inspect (`plugins inspect --runtime`)
 * loads plugin modules, so it runs ONLY on demand (button), never while
 * loading the list.
 */

import { useCallback, useEffect, useReducer, useState } from "react";
import { getStrings } from "../../i18n/ko";
import {
  type PluginRow,
  type PluginRuntime,
  getPluginRuntime,
  listPlugins,
  normalizeAppError,
  setPluginEnabled,
  type TauriAppError,
} from "../../lib/tauri";
import {
  initialPluginsRuntimeState,
  initialPluginsToggleState,
  mapPluginsError,
  pluginsRuntimeReducer,
  pluginsToggleReducer,
} from "./pluginsState";

const t = getStrings("plugins");

/** Maps an IPC rejection to its Korean message (stable code based). */
function errorText(err: unknown): string {
  const appError: TauriAppError = normalizeAppError(err);
  return t.errors[mapPluginsError(appError)];
}

/** The enabled state label (fail-soft: null → unknown). */
function stateLabel(enabled: boolean | null | undefined): string {
  if (enabled === true) return t.enabled;
  if (enabled === false) return t.disabled;
  return t.stateUnknown;
}

/** The registered surfaces of a runtime payload, in display order. */
function runtimeSurfaces(data: PluginRuntime): Array<[string, string[]]> {
  return [
    [t.tools, data.tools],
    [t.hooks, data.hooks],
    [t.services, data.services],
    [t.cliCommands, data.cliCommands],
    [t.gatewayMethods, data.gatewayMethods],
    [t.routes, data.routes],
  ];
}

export function PluginsFeature() {
  const [plugins, setPlugins] = useState<PluginRow[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [toggle, dispatchToggle] = useReducer(
    pluginsToggleReducer,
    initialPluginsToggleState,
  );
  const [runtime, dispatchRuntime] = useReducer(
    pluginsRuntimeReducer,
    initialPluginsRuntimeState,
  );

  const reload = useCallback(async () => {
    try {
      setPlugins(await listPlugins());
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

  // On-demand runtime inspect: runs only for a dispatched `request` —
  // the list-load path never triggers it.
  useEffect(() => {
    if (runtime.loadingId === null) {
      return;
    }
    const id = runtime.loadingId;
    getPluginRuntime(id)
      .then((data) => dispatchRuntime({ type: "finish", id, error: null, data }))
      .catch((err) =>
        dispatchRuntime({ type: "finish", id, error: errorText(err), data: null }),
      );
  }, [runtime.loadingId]);

  const togglePlugin = useCallback(
    (id: string, currentEnabled: boolean | null | undefined) => {
      if (toggle.pending !== null) {
        return;
      }
      const target = !(currentEnabled ?? true);
      dispatchToggle({ type: "start", key: id });
      setPluginEnabled(id, target)
        .then(() => dispatchToggle({ type: "finish", error: null }))
        .catch((err) => dispatchToggle({ type: "finish", error: errorText(err) }));
    },
    [toggle.pending],
  );

  const requestRuntime = useCallback(
    (id: string) => {
      if (runtime.loadingId !== null) {
        return;
      }
      dispatchRuntime({ type: "request", id });
    },
    [runtime.loadingId],
  );

  if (plugins === null) {
    return (
      <section>
        <h2>{t.title}</h2>
        <p>{t.loading}</p>
        {listError !== null && <p role="alert">{listError}</p>}
      </section>
    );
  }

  return (
    <section>
      <h2>{t.title}</h2>
      {listError !== null && <p role="alert">{listError}</p>}
      {toggle.error !== null && <p role="alert">{toggle.error}</p>}

      {plugins.length === 0 ? (
        <p>{t.noPlugins}</p>
      ) : (
        <table>
          <thead>
            <tr>
              <th>{t.id}</th>
              <th>{t.name}</th>
              <th>{t.format}</th>
              <th>{t.status}</th>
              <th>{t.dependency}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {plugins.map((plugin) => {
              const isEnabled = plugin.enabled ?? true;
              const busy = toggle.pending !== null;
              return (
                <tr key={plugin.id}>
                  <td>{plugin.id}</td>
                  <td>{plugin.name ?? "—"}</td>
                  <td>{plugin.format ?? "—"}</td>
                  <td>{stateLabel(plugin.enabled)}</td>
                  <td>{plugin.dependencyStatus ?? "—"}</td>
                  <td>
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => togglePlugin(plugin.id, plugin.enabled)}
                    >
                      {toggle.pending === plugin.id
                        ? t.toggling
                        : isEnabled
                          ? t.disable
                          : t.enable}
                    </button>{" "}
                    <button
                      type="button"
                      disabled={runtime.loadingId !== null || busy}
                      onClick={() => requestRuntime(plugin.id)}
                    >
                      {runtime.loadingId === plugin.id
                        ? t.inspecting
                        : t.inspectButton}
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}

      {/* Runtime state (on-demand) */}
      <h3>{t.runtime}</h3>
      {runtime.loadingId !== null ? (
        <p>{t.inspecting}</p>
      ) : runtime.error !== null ? (
        // Fail-closed: no previous "loaded" state is kept after a failure.
        <p role="alert">{t.runtimeUnknown}</p>
      ) : runtime.data !== null ? (
        <div>
          <p>{runtime.data.id}</p>
          {runtimeSurfaces(runtime.data).map(([label, names]) => (
            <p key={label}>
              <strong>{label}</strong> ({names.length}):{" "}
              {names.length === 0 ? t.surfaceEmpty : names.join(", ")}
            </p>
          ))}
          {Array.isArray(runtime.data.diagnostics) &&
            runtime.data.diagnostics.length > 0 && (
              <p>
                <strong>{t.diagnostics}</strong>:{" "}
                {runtime.data.diagnostics.join("; ")}
              </p>
            )}
        </div>
      ) : (
        <p>{t.runtimeHint}</p>
      )}
    </section>
  );
}
