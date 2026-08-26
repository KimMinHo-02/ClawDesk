/**
 * Setup feature (Phase 2): environment state display + OpenClaw install.
 * Phase 8.1: one-shot Node.js update (winget) when the detected Node is
 * unsupported.
 *
 * State machine: loading → ready → installing → success | error.
 * A synchronous ref guard plus the disabled button prevent duplicate
 * install invocations; there is no progress stream — one invoke, one
 * result.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { getStrings } from "../../i18n/ko";
import {
  type EnvironmentReport,
  detectEnvironment,
  installOpenClaw,
  updateNode,
} from "../../lib/tauri";
import {
  canInstall,
  errorCodeOf,
  errorToMessage,
  isNodeUpdateNeeded,
  nodeStateOf,
  openclawStateOf,
} from "./setupState";

const t = getStrings("install");

type Phase =
  | { kind: "loading" }
  | { kind: "ready"; report: EnvironmentReport }
  | { kind: "installing" }
  | { kind: "success"; version: string; fresh: boolean }
  | { kind: "error"; source: "detect" | "install"; message: string; code?: string };

export function SetupFeature() {
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  // Synchronous duplicate-click guard across the async install.
  const installingRef = useRef(false);
  // Phase 8.1: node update state + duplicate-click guard.
  const [nodeUpdating, setNodeUpdating] = useState(false);
  const [nodeUpdateError, setNodeUpdateError] = useState<string | null>(null);
  const nodeUpdatingRef = useRef(false);
  // Read the current phase inside async callbacks without stale closures.
  const phaseRef = useRef<Phase>(phase);
  phaseRef.current = phase;

  const loadEnvironment = useCallback(async () => {
    setPhase({ kind: "loading" });
    try {
      const report = await detectEnvironment();
      setPhase({ kind: "ready", report });
    } catch (err) {
      setPhase({
        kind: "error",
        source: "detect",
        message: errorToMessage(err),
        code: errorCodeOf(err),
      });
    }
  }, []);

  useEffect(() => {
    void loadEnvironment();
  }, [loadEnvironment]);

  const startInstall = useCallback(async () => {
    if (installingRef.current) {
      return;
    }
    installingRef.current = true;
    setPhase({ kind: "installing" });
    try {
      const result = await installOpenClaw();
      setPhase({
        kind: "success",
        version: result.version,
        fresh: result.status === "installed",
      });
    } catch (err) {
      setPhase({
        kind: "error",
        source: "install",
        message: errorToMessage(err),
        code: errorCodeOf(err),
      });
    } finally {
      installingRef.current = false;
    }
  }, []);

  /**
   * Phase 8.1 one-shot Node.js update. From the ready screen the returned
   * detection is merged into the report; from the install-error screen
   * detection is re-run to rebuild the ready state.
   */
  const startNodeUpdate = useCallback(async () => {
    if (nodeUpdatingRef.current) {
      return;
    }
    nodeUpdatingRef.current = true;
    setNodeUpdating(true);
    setNodeUpdateError(null);
    try {
      const detected = await updateNode();
      if (phaseRef.current.kind === "ready") {
        setPhase((prev) =>
          prev.kind === "ready"
            ? { kind: "ready", report: { ...prev.report, node: detected } }
            : prev,
        );
      } else {
        void loadEnvironment();
      }
    } catch (err) {
      setNodeUpdateError(errorToMessage(err));
    } finally {
      nodeUpdatingRef.current = false;
      setNodeUpdating(false);
    }
  }, [loadEnvironment]);

  if (phase.kind === "loading") {
    return (
      <section>
        <h2>{t.title}</h2>
        <p>{t.detecting}</p>
      </section>
    );
  }

  if (phase.kind === "installing") {
    return (
      <section>
        <h2>{t.title}</h2>
        <p aria-live="polite">{t.installing}</p>
      </section>
    );
  }

  if (phase.kind === "success") {
    return (
      <section>
        <h2>{t.title}</h2>
        <p>{phase.fresh ? t.installed : t.alreadyInstalled}</p>
        <p>
          {t.version}: {phase.version}
        </p>
      </section>
    );
  }

  if (phase.kind === "error") {
    const retry =
      phase.source === "install"
        ? () => void startInstall()
        : () => void loadEnvironment();
    // Phase 8.1: when the install failed on an unsupported Node, the
    // one-shot update is offered right next to the retry.
    const showNodeUpdate =
      phase.source === "install" && phase.code === "unsupported-node-version";
    return (
      <section>
        <h2>{t.title}</h2>
        <p role="alert">{phase.message}</p>
        {nodeUpdateError !== null && <p role="alert">{nodeUpdateError}</p>}
        {showNodeUpdate && (
          <button type="button" disabled={nodeUpdating} onClick={() => void startNodeUpdate()}>
            {nodeUpdating ? t.nodeUpdating : t.nodeUpdateButton}
          </button>
        )}
        <button type="button" onClick={retry}>
          {t.retry}
        </button>
      </section>
    );
  }

  // ready
  const node = nodeStateOf(phase.report.node);
  const openclaw = openclawStateOf(phase.report.openclaw);
  const installable = canInstall(phase.report);
  const nodeUpdateNeeded = isNodeUpdateNeeded(phase.report.node);
  return (
    <section>
      <h2>{t.title}</h2>
      <dl>
        <dt>{t.openclawLabel}</dt>
        <dd>
          {openclaw.kind === "installed"
            ? `${t.openclawInstalled} ${openclaw.version ?? t.versionUnknown}`
            : t.openclawNotInstalled}
        </dd>
        <dt>{t.nodeLabel}</dt>
        <dd>
          {node.kind === "not-found" && t.nodeNotInstalled}
          {node.kind === "unsupported" && `${t.nodeUnsupported} (${node.version})`}
          {node.kind === "supported" && `${t.nodeVersion} ${node.version}`}
        </dd>
      </dl>
      {nodeUpdateNeeded && (
        <div>
          <button type="button" disabled={nodeUpdating} onClick={() => void startNodeUpdate()}>
            {nodeUpdating ? t.nodeUpdating : t.nodeUpdateButton}
          </button>
          {nodeUpdateError !== null && <p role="alert">{nodeUpdateError}</p>}
        </div>
      )}
      {openclaw.kind === "not-installed" && (
        <button type="button" disabled={!installable} onClick={() => void startInstall()}>
          {t.installButton}
        </button>
      )}
    </section>
  );
}
