# Phase 1 — Windows Environment + OpenClaw Adapter

## 목표

Windows 실행 환경과 OpenClaw 상태를 **Rust adapter**를 통해 감지하고 structured
형식으로 반환한다. frontend는 Phase 0 scaffold를 유지한다(GUI 기능 없음).

## Exit Criteria

### 1. 감지 기능 (Rust, application service → port → adapter 경유)

- [ ] Windows version detect (OS build/version)
- [ ] Windows architecture detect (x64 지원, x64 외는 unsupported structured error)
- [ ] Node.js detect (존재 여부 + version, 부재는 structured "not found")
- [ ] OpenClaw executable detect (존재 여부 + 경로, 부재는 structured "not found")
- [ ] OpenClaw version (`--version` 결과 parsing)
- [ ] Gateway status
- [ ] Update status (latest stable 기준 "updated" / "update available" / unknown)

### 2. 인터페이스

- [ ] `ProcessRunner`: structured `executable + argv` spawn, timeout, exit code,
      stdout/stderr capture, structured error type
- [ ] `WindowsSystemPort` trait + `WindowsSystemAdapter` 구현
- [ ] `OpenClawPort` trait + `OpenClawAdapter` 구현
- [ ] 모든 spawn이 `ProcessRunner` 경유 (spawn 지점 단일화)
- [ ] shell string 조합 없이 argv만 (security invariant S1/S2)

### 3. 테스트 (`cargo test`, fake CLI fixture 사용)

- [ ] fake CLI happy path (version/status/update status parsing)
- [ ] malformed output (변형 stdout)
- [ ] missing executable
- [ ] timeout
- [ ] non-zero exit code
- [ ] JSON parse failure

fixture 위치: `tests/fixtures/openclaw/` (Phase 0에 디렉터리만 존재, fixture는 Phase 1에서 생성)

### 4. 검증 명령 (전부 통과)

- `cargo check`
- `cargo test`
- `cargo clippy`
- `cargo fmt --check`

frontend scaffold는 `pnpm typecheck` 통과를 유지한다.

## Non-Goals (이 phase에서 안 하는 것)

- **OpenClaw install** (Phase 2)
- **OpenClaw update 실행** (상태만 보고, 실행은 Phase 8/10 영역)
- **API key 설정** (저장/UI 모두 Phase 3)
- **GUI feature 구현** (frontend UI 변경 없음, scaffold 유지)
- real OpenClaw E2E (opt-in layer만 있고, 이 phase에서 불필요)

## Phase 종료 보고

`clawdesk-build` 완료 보고 형식 7개 항목 + exit criteria 체크리스트 전 항목 결과.
