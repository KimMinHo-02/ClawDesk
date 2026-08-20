# Phase 1 — Windows Environment + OpenClaw Adapter

Status: completed (2026-08-18)

## 목표

Windows 실행 환경과 OpenClaw 상태를 **Rust adapter**를 통해 감지하고 structured
형식으로 반환한다. frontend는 Phase 0 scaffold를 유지한다(GUI 기능 없음).

## Exit Criteria

### 1. 감지 기능 (Rust, application service → port → adapter 경유)

- [x] Windows version detect (OS build/version)
- [x] Windows architecture detect (x64 지원, x64 외는 unsupported structured error)
- [x] Node.js detect (존재 여부 + version, 부재는 structured "not found")
- [x] OpenClaw executable detect (존재 여부 + 경로, 부재는 structured "not found")
- [x] OpenClaw version (`--version` 결과 parsing)
- [x] Gateway status
- [x] Update status (latest stable 기준 "updated" / "update available" / unknown)

### 2. 인터페이스

- [x] `ProcessRunner`: structured `executable + argv` spawn, timeout, exit code,
      stdout/stderr capture, structured error type
- [x] `WindowsSystemPort` trait + `WindowsSystemAdapter` 구현
- [x] `OpenClawPort` trait + `OpenClawAdapter` 구현
- [x] 모든 spawn이 `ProcessRunner` 경유 (spawn 지점 단일화)
- [x] shell string 조합 없이 argv만 (security invariant S1/S2)

### 3. 테스트 (`cargo test`, fake CLI fixture 사용)

- [x] fake CLI happy path (version/status/update status parsing)
- [x] malformed output (변형 stdout)
- [x] missing executable
- [x] timeout
- [x] non-zero exit code
- [x] JSON parse failure

fixture 위치: `tests/fixtures/openclaw/` (Phase 0에 디렉터리만 존재, fixture는 Phase 1에서 생성)

### 4. 검증 명령 (전부 통과)

- `cargo check`
- `cargo test`
- `cargo clippy`
- `cargo fmt --check`

frontend scaffold는 `pnpm typecheck` 통과를 유지한다.

완료 시 실제 검증 결과 (2026-08-18): 위 명령 전부 통과, `cargo test` 58 passed / 0 failed / 0 ignored.

## Non-Goals (이 phase에서 안 하는 것)

- **OpenClaw install** (Phase 2)
- **OpenClaw update 실행** (상태만 보고, 실행은 Phase 8/10 영역)
- **API key 설정** (저장/UI 모두 Phase 3)
- **GUI feature 구현** (frontend UI 변경 없음, scaffold 유지)
- real OpenClaw E2E (opt-in layer만 있고, 이 phase에서 불필요)

## Phase 종료 보고

`clawdesk-build` 완료 보고 형식 7개 항목 + exit criteria 체크리스트 전 항목 결과.
