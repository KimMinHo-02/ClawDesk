import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getStrings } from "../../i18n/ko";
import type { EnvironmentReport, InstallResult, NodeDetection } from "../../lib/tauri";

const mockDetectEnvironment = vi.fn();
const mockInstallOpenClaw = vi.fn();
const mockUpdateNode = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));
vi.mock("../../lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/tauri")>();
  return {
    ...actual,
    detectEnvironment: (...args: unknown[]) => mockDetectEnvironment(...args),
    installOpenClaw: (...args: unknown[]) => mockInstallOpenClaw(...args),
    updateNode: (...args: unknown[]) => mockUpdateNode(...args),
  };
});

import { SetupFeature } from "./SetupFeature";

const t = getStrings("install");

function report(
  overrides: {
    node?: EnvironmentReport["node"];
    openclaw?: EnvironmentReport["openclaw"];
  } = {},
): EnvironmentReport {
  return {
    windows_version: { major_version: 11, build: 26100, ubr: 0, product_name: null },
    architecture: "x64",
    node: overrides.node ?? { status: "found", version: "22.22.3" },
    openclaw: overrides.openclaw ?? { status: "not-found" },
  };
}

const detectedOpenClaw = {
  status: "detected" as const,
  executable: "C:\\npm\\openclaw",
  version: "2026.7.1-2" as string | null,
  gateway: null,
  update: "updated" as const,
};

describe("SetupFeature", () => {
  beforeEach(() => {
    mockDetectEnvironment.mockReset();
    mockInstallOpenClaw.mockReset();
    mockUpdateNode.mockReset();
  });

  // RTL auto-cleanup needs framework globals; clean the DOM explicitly.
  afterEach(() => {
    cleanup();
  });

  it("shows the install button when OpenClaw is missing and Node is supported", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report());
    render(<SetupFeature />);

    const button = await screen.findByRole("button", { name: t.installButton });
    expect(button.hasAttribute("disabled")).toBe(false);
    expect(await screen.findByText(t.openclawNotInstalled)).toBeTruthy();
  });

  it("shows the existing OpenClaw version and no install button when installed", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report({ openclaw: detectedOpenClaw }));
    render(<SetupFeature />);

    expect(await screen.findByText(`${t.openclawInstalled} 2026.7.1-2`)).toBeTruthy();
    expect(screen.queryByRole("button", { name: t.installButton })).toBeNull();
  });

  it("shows Node guidance and a disabled button when Node is missing", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report({ node: { status: "not-found" } }));
    render(<SetupFeature />);

    expect(await screen.findByText(t.nodeNotInstalled)).toBeTruthy();
    const button = await screen.findByRole("button", { name: t.installButton });
    expect(button.hasAttribute("disabled")).toBe(true);
  });

  it("shows the unsupported Node message with the detected version", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(
      report({ node: { status: "found", version: "23.0.0" } }),
    );
    render(<SetupFeature />);

    expect(await screen.findByText(`${t.nodeUnsupported} (23.0.0)`)).toBeTruthy();
    const button = screen.getByRole("button", { name: t.installButton });
    expect(button.hasAttribute("disabled")).toBe(true);
  });

  it("installs and shows the installed version on success", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report());
    mockInstallOpenClaw.mockResolvedValueOnce({
      status: "installed",
      version: "2026.7.1-2",
    });
    render(<SetupFeature />);

    fireEvent.click(await screen.findByRole("button", { name: t.installButton }));

    expect(await screen.findByText(t.installed)).toBeTruthy();
    expect(await screen.findByText(`${t.version}: 2026.7.1-2`)).toBeTruthy();
    expect(mockInstallOpenClaw).toHaveBeenCalledTimes(1);
  });

  it("shows the already-installed message with the existing version", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report());
    mockInstallOpenClaw.mockResolvedValueOnce({
      status: "already-installed",
      version: "2025.1.0",
    });
    render(<SetupFeature />);

    fireEvent.click(await screen.findByRole("button", { name: t.installButton }));

    expect(await screen.findByText(t.alreadyInstalled)).toBeTruthy();
    expect(screen.getByText(`${t.version}: 2025.1.0`)).toBeTruthy();
  });

  it("maps a stable error code to its Korean message and offers retry", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report());
    mockInstallOpenClaw
      .mockRejectedValueOnce({ code: "node-not-found", message: "raw infra detail" })
      .mockResolvedValueOnce({ status: "installed", version: "2026.7.1-2" });
    render(<SetupFeature />);

    fireEvent.click(await screen.findByRole("button", { name: t.installButton }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(t.errors["node-not-found"]);
    expect(alert.textContent).not.toContain("raw infra detail");

    fireEvent.click(screen.getByRole("button", { name: t.retry }));
    expect(await screen.findByText(t.installed)).toBeTruthy();
    expect(mockInstallOpenClaw).toHaveBeenCalledTimes(2);
  });

  it("prevents duplicate install invocations while the install is in flight", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report());
    let resolveInstall!: (value: InstallResult) => void;
    mockInstallOpenClaw.mockImplementation(
      () =>
        new Promise<InstallResult>((resolve) => {
          resolveInstall = resolve;
        }),
    );
    render(<SetupFeature />);

    const button = await screen.findByRole("button", { name: t.installButton });
    fireEvent.click(button);
    fireEvent.click(button);

    expect(mockInstallOpenClaw).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: t.installButton })).toBeNull();
    expect(screen.getByText(t.installing)).toBeTruthy();

    resolveInstall({ status: "installed", version: "2026.7.1-2" });
    await waitFor(() => expect(screen.getByText(t.installed)).toBeTruthy());
  });

  it("falls back to the generic message when detect fails with a non-AppError", async () => {
    mockDetectEnvironment.mockRejectedValueOnce(new Error("boom"));
    render(<SetupFeature />);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(t.errors.fallback);
  });
});

describe("SetupFeature Node.js update (Phase 8.1)", () => {
  beforeEach(() => {
    mockDetectEnvironment.mockReset();
    mockInstallOpenClaw.mockReset();
    mockUpdateNode.mockReset();
  });

  afterEach(() => {
    cleanup();
  });

  it("offers the one-shot update when the detected Node is unsupported", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(
      report({ node: { status: "found", version: "23.0.0" } }),
    );
    render(<SetupFeature />);

    const button = await screen.findByRole("button", { name: t.nodeUpdateButton });
    expect(button.hasAttribute("disabled")).toBe(false);
  });

  it("does not offer the update for a supported or missing Node", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report());
    render(<SetupFeature />);
    await screen.findByRole("button", { name: t.installButton });
    expect(screen.queryByRole("button", { name: t.nodeUpdateButton })).toBeNull();
    cleanup();

    mockDetectEnvironment.mockResolvedValueOnce(report({ node: { status: "not-found" } }));
    render(<SetupFeature />);
    await screen.findByRole("button", { name: t.installButton });
    expect(screen.queryByRole("button", { name: t.nodeUpdateButton })).toBeNull();
  });

  it("merges the returned detection into the report on success", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(
      report({ node: { status: "found", version: "18.19.0" } }),
    );
    mockUpdateNode.mockResolvedValueOnce({ status: "found", version: "24.15.0" });
    render(<SetupFeature />);

    fireEvent.click(await screen.findByRole("button", { name: t.nodeUpdateButton }));

    expect(await screen.findByText(`${t.nodeVersion} 24.15.0`)).toBeTruthy();
    // The node is now supported: no update button, install becomes enabled.
    expect(screen.queryByRole("button", { name: t.nodeUpdateButton })).toBeNull();
    const install = screen.getByRole("button", { name: t.installButton });
    expect(install.hasAttribute("disabled")).toBe(false);
    expect(mockUpdateNode).toHaveBeenCalledTimes(1);
    expect(mockUpdateNode).toHaveBeenCalledWith();
  });

  it("shows the stable-code message when the update fails", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(
      report({ node: { status: "found", version: "18.19.0" } }),
    );
    mockUpdateNode.mockRejectedValueOnce({
      code: "winget-not-found",
      message: "raw detail",
    });
    render(<SetupFeature />);

    fireEvent.click(await screen.findByRole("button", { name: t.nodeUpdateButton }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(t.errors["winget-not-found"]);
    expect(alert.textContent).not.toContain("raw detail");
    // The offer stays available for a retry.
    expect(screen.getByRole("button", { name: t.nodeUpdateButton })).toBeTruthy();
  });

  it("prevents duplicate update invocations while in flight", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(
      report({ node: { status: "found", version: "18.19.0" } }),
    );
    let resolveUpdate!: (value: NodeDetection) => void;
    mockUpdateNode.mockImplementation(
      () =>
        new Promise<NodeDetection>((resolve) => {
          resolveUpdate = resolve;
        }),
    );
    render(<SetupFeature />);

    const button = await screen.findByRole("button", { name: t.nodeUpdateButton });
    fireEvent.click(button);
    fireEvent.click(button);

    expect(mockUpdateNode).toHaveBeenCalledTimes(1);
    expect(screen.getByText(t.nodeUpdating)).toBeTruthy();

    resolveUpdate({ status: "found", version: "24.15.0" });
    await waitFor(() => expect(screen.getByText(`${t.nodeVersion} 24.15.0`)).toBeTruthy());
  });

  it("offers the update from the unsupported-node-version install error", async () => {
    mockDetectEnvironment.mockResolvedValueOnce(report());
    mockInstallOpenClaw.mockRejectedValueOnce({
      code: "unsupported-node-version",
      message: "raw detail",
    });
    render(<SetupFeature />);

    fireEvent.click(await screen.findByRole("button", { name: t.installButton }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain(t.errors["unsupported-node-version"]);
    expect(alert.textContent).not.toContain("raw detail");
    const updateButton = screen.getByRole("button", { name: t.nodeUpdateButton });
    expect(updateButton.hasAttribute("disabled")).toBe(false);
  });
});
