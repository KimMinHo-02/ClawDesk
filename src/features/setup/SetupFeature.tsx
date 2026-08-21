/**
 * Setup feature (Phase 2): environment state display + OpenClaw install.
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
} from "../../lib/tauri";
import {
  canInstall,
  errorToMessage,
  nodeStateOf,
  openclawStateOf,
} from "./setupState";

const t = getStrings("install");

type Phase =
  | { kind: "loading" }
  | { kind: "ready"; report: EnvironmentReport }
  | { kind: "installing" }
  | { kind: "success"; version: string; fresh: boolean }
  | { kind: "error"; source: "detect" | "install"; message: string };

export function SetupFeature() {
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  // Synchronous duplicate-click guard across the async install.
  const installingRef = useRef(false);

  const loadEnvironment = useCallback(async () => {
    setPhase({ kind: "loading" });
    try {
      const report = await detectEnvironment();
      setPhase({ kind: "ready", report });
    } catch (err) {
      setPhase({ kind: "error", source: "detect", message: errorToMessage(err) });
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
      setPhase({ kind: "error", source: "install", message: errorToMessage(err) });
    } finally {
      installingRef.current = false;
    }
  }, []);

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
    return (
      <section>
        <h2>{t.title}</h2>
        <p role="alert">{phase.message}</p>
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
      {openclaw.kind === "not-installed" && (
        <button type="button" disabled={!installable} onClick={() => void startInstall()}>
          {t.installButton}
        </button>
      )}
    </section>
  );
}
