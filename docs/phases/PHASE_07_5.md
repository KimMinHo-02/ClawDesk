# Phase 7.5 — Phase 0–7 Integration Wiring Fix

Status: completed (2026-08-26)

## 목표

Phase 0~7까지 구현된 기능의 backend/service logic은 유지하면서,
실제 Windows `tauri dev` 환경에서 frontend → Tauri IPC → Rust command
연결이 정상 동작하도록 공통 wiring 결함을 수정한다.

새 기능은 추가하지 않는다.

## Exit Criteria

### 1. Tauri dev main binary

- [x] `pnpm exec tauri dev` 실행 시 cargo가 실행 binary를 자동 선택한다.
- [x] ClawDesk main binary가 기본 실행 대상이다.
- [x] fake/helper binary는 그대로 유지한다.

### 2. Phase 2~7 IPC command naming

- [x] Phase 2~7 frontend invoke command 전체를 확인한다.
- [x] 각 frontend command가 실제 Rust `#[tauri::command]`에 연결된다.
- [x] 기존 frontend kebab-case IPC 계약을 유지한다.
- [x] Rust command는 현재 Tauri 2에서 지원하는 공식 rename 방식으로
      kebab-case command name을 명시한다.
- [x] Phase 2~7의 invoke command 누락이 없다.

대상 최소 범위:

- Setup / Installer
- Models / Providers
- Skills
- Plugins
- Tools / Security
- Channels
- Automations

### 3. 구조 유지

- [x] service / adapter / business logic 변경 없음
- [x] ProcessRunner 단일 spawn boundary 유지
- [x] frontend UI 구조 변경 없음
- [x] OpenClaw parser / CLI contract 변경 없음
- [x] Phase 8 기능 구현 없음

### 4. 테스트

- [x] frontend IPC wrapper 테스트가 실제 command naming 계약과 일치
- [x] Rust command registration과 command name 계약 검증
- [x] 기존 Phase 0~7 regression test 유지

### 5. 검증

전부 통과:

- cargo fmt --check
- cargo check --all-targets
- cargo test --all-targets
- cargo clippy --all-targets
- pnpm typecheck

## Non-Goals

- 새로운 Installer 기능
- 새로운 Provider / Model 기능
- Skills / Plugins 기능 확장
- Tools / Security 기능 확장
- Channels 기능 확장
- Automations 기능 확장
- Profile / Update / Diagnostics
- Integrated Chat
- Windows Release
- Phase 8 이후 기능

## 완료 후 수동 Smoke Test

자동 검증 완료 후 사용자가 직접:

pnpm exec tauri dev

를 실행한다.

다음 Phase 0~7 영역이 더 이상 공통 IPC command-not-found 오류로
실패하지 않는지 확인한다.

- OpenClaw 준비
- Models / Providers
- Skills
- Plugins
- Tools / Security
- Channels
- Automations

이 smoke test에서 새로 드러나는 내부 기능 오류는
Phase 7.5 완료 여부와 분리하여 실제 오류 기준으로 후속 수정한다.
