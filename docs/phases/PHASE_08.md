# Phase 8 — Profile / Update / Diagnostics

Status: completed (2026-08-26)

## 목표

PRODUCT_CONTRACT §4.7의 **계정/프로필 정보 표시, OpenClaw update 상태, API(gateway) 상태, Diagnostics(환경 요약 + 로그 조회)**를 terminal 없이(한국어 UI) 제공한다.

- §4.7 범위를 초과하는 새 기능은 추가하지 않는다. 읽기 전용 표시 + one-shot 로그 tail만.
- 모든 OpenClaw 작업은 `openclaw` CLI(structured argv, timeout) 경유만 → `ProcessRunner` 단일 spawn boundary(S1/S2).
- Phase 1 이미 구현된 detection 데이터(gateway status, update state, environment)는 **재사용**한다 — 재구현 0.
- IPC naming은 Phase 7.5 계약 준수: frontend kebab-case + Rust `#[tauri::command(rename = "...")]`.

## 설계 근거 (2026-08-26 접속 기준, `docs.openclaw.ai` live docs)

- `https://docs.openclaw.ai/cli` (CLI index — status/logs/agents/update surface)
- `https://docs.openclaw.ai/cli/status` (status --json: overview + update info, read-only)
- `https://docs.openclaw.ai/cli/logs` (logs tail — options, fallback)
- `https://docs.openclaw.ai/cli/agents` (agents list — multi-agent identity)
- `https://docs.openclaw.ai/logging` (file log JSONL, CLI --json event types, redaction, Windows log dir)
- 재사용: Phase 1 `OpenClawAdapter`(gateway/update), `EnvironmentReport`, `ProcessRunner`, Phase 4~7 port/adapter/contract test 패턴, Phase 7.5 IPC naming 계약

## 핵심 사실 (상기 docs 기준)

### Profile / 계정 (agents)
- OpenClaw **agents** = 격리된 identity(workspace + auth + routing). `main`은 기본 agent.
- ClawDesk의 "계정/프로필" = agents 표시. **read-only**: `openclaw agents list --json`만.
  add/delete/bind/set-identity는 non-goal.
- `agents list --json`의 exact JSON 스키마는 docs에 예시가 없다 → **fail-soft parser**(기존 skills/channels/automations parser 패턴)로 unknown 필드 무시, 부재 필드 → `None`. real E2E(opt-in)에서 real 출력으로 확인.

### Update 상태
- `openclaw update status --json` → `{current, latest, updateAvailable}` (Phase 1에서 구현·검증 완료: `OpenClawAdapter::update_state`, adapter.rs:103-115).
- Phase 1 `UpdateState` 삼항(`updated`/`update-available`/`unknown`)은 setup flow용으로 **그대로 유지**.
- Phase 8 추가: current/latest 버전 문자열을 함께 노출하는 detail 타입 — PRODUCT_CONTRACT §4.7 "버전 차이" 요구.
- 실제 update 실행(`openclaw update` run/repair/wizard)은 non-goal (Phase 2 installer / real E2E 영역).

### API 상태 (gateway)
- `openclaw gateway status --json` → state(`running`/`stopped`) + version + port (Phase 1 구현: `GatewayStatus`, adapter.rs:78-101).
- Phase 8: Profile 페이지용 standalone command로 노출 (Phase 1 adapter 메서드 재사용).
- gateway lifecycle(start/stop/restart/install/uninstall/call/probe)은 non-goal — 읽기 전용.

### Diagnostics
- **환경 요약**: `detect-environment`(Phase 1/2 `EnvironmentReport`) 재사용 — 신규 command 0.
- **로그 조회**: `openclaw logs --limit <n> --json` **one-shot tail** (RPC → 실패 시 file log 자동 fallback → gateway stopped 상태에서도 조회 가능).
  - `--json` = line-delimited **type-tagged events**:
    - `meta`: stream 메타데이터(file, source, sourceKind, service, cursor, size)
    - `log`: parsed log entry(time, level, subsystem, message + 선택적 top-level 필드 hostname/agent_id/session_id/channel 등)
    - `notice`: truncation/rotation 힌트
    - `raw`: unparsed log line
    - `error`: gateway 연결 실패 (**stderr 전용** — stdout parse 대상 아님)
  - `--limit <n>`: max lines(default 200) — ClawDesk는 `limit`을 1..=1000 사전 검증, 기본 200.
  - `--follow` **금지** (스트리밍 = terminal UX non-goal). fake CLI도 `--follow`를 reject(exit 2).
  - 로그 파일 위치(참고용): Windows는 OS temp의 user-scoped `openclaw-<uid>` 디렉터리, `openclaw-YYYY-MM-DD.log`(24h prune). ClawDesk는 **파일 직접 읽기 0** — CLI 경유만(S4).
  - secret 처리: OpenClaw 자체 redaction은 file log 작성 시 항상 적용 + ClawDesk는 S8 masking pipeline(ProcessRunner chokepoint)을 process output에 **다시** 적용. 로그 UI/오류에 secret plain 노출 0(S3).
  - empty stdout(로그 없음) = 정상(0 lines) — error 아님.

## Scope (in-scope)

### Rust (`src-tauri/`)
1. `src/domain/models/diagnostics.rs` (신규)
   - `UpdateStatusDetail { state: UpdateState, current: Option<String>, latest: Option<String> }` (Phase 1 `UpdateState` 재사용)
   - `AgentRow { id: String, default: bool, name: Option<String>, emoji: Option<String>, workspace: Option<String>, bindings: Option<u64> }` — fail-soft parser
   - `LogEvent`(kind: `log`/`raw`/`meta`/`notice` — 각 필드 fail-soft) + `LogsResult { lines: Vec<LogEvent>, source: Option<String>, truncated: bool }`
2. `src/domain/ports/openclaw_diagnostics.rs` (신규)
   - `OpenClawDiagnosticsPort`: `list_agents()`, `update_detail()`, `tail_logs(limit: u32)` (전부 `Result<_, AppError>`)
   - gateway_status는 Phase 1 `OpenClawPort`에 그대로 (이동 0).
3. `src/infrastructure/openclaw/diagnostics.rs` (신규)
   - `OpenClawDiagnosticsAdapter` (ProcessPort):
     - `["agents", "list", "--json"]` (timeout 15s)
     - `["update", "status", "--json"]` (timeout 15s, Phase 1과 동일) — process failure → `Unknown` (fail-soft, Phase 1 policy)
     - `["logs", "--limit", "<n>", "--json"]` (timeout 30s) — `n`은 u32 `Display`만 (S2: 문자열 보간 0)
   - exact argv, exit code, parse contract은 Phase 4~7 adapter 패턴.
4. `src/application/services/diagnostics.rs` (신규)
   - `DiagnosticsService`: `logs limit` 1..=1000 사전 검증(위반 시 0 CLI, fail-closed), port 조합(gateway_status는 `OpenClawPort`), `production()` wiring.
5. `src/commands/diagnostics.rs` (신규) — IPC command 4개 (kebab + `rename` 속성, Phase 7.5 계약):
   - `get-gateway-status` → `GatewayStatus` (Phase 1 adapter 재사용)
   - `get-update-status` → `UpdateStatusDetail`
   - `get-agents` → `Vec<AgentRow>`
   - `get-logs` → `LogsResult` (argument: `limit: u32`)
6. `src/error.rs` — stable code +2 (기존 `openclaw-<area>-read-failed` 컨벤션 준수):
   - `openclaw-agents-read-failed` (agents list non-zero / top-level malformed)
   - `openclaw-logs-read-failed` (logs tail non-zero)
   - gateway/update는 기존 코드 재사용(`openclaw-gateway-parse`, `process-failed`, `process-timeout`, `openclaw-not-found` — update는 fail-soft라 신규 코드 없음)
7. `src/lib.rs` — command 4개 `generate_handler!` 등록.
8. `tests/ipc_name_contract.rs` — `IPC_CONTRACT` +4 (45 → **49**), `registered_names()` +4.
9. `fixtures/fake-openclaw/main.rs` — handler +2 (기존 `gateway status`/`update status` handler는 유지):
   - `agents list --json`: fixture agent 2개(`main` default + identity name/emoji/workspace/bindings, 그 외 1개)
   - `logs --limit <n> --json`: type-tagged events(`meta` + `log` 다수 + `raw` 1 + `notice` truncation), `--limit` 준수, fake `sk-` 토큰 포함 1줄(masking 검증용 — fake token, real secret 아님)
   - `logs --follow` reject(exit 2) — non-goal flag guard (Phase 7 패턴)
10. `tests/diagnostics_contract.rs` (신규) — contract test (Phase 4~7 패턴 1:1):
    - exact argv(agents/update/logs — logs는 limit byte-match), Unicode/byte-match
    - fail-soft parser(case별), missing executable, non-zero exit, timeout, empty stdout
    - fake `--follow` reject, non-goal flag reject
    - log masking 확인(fake `sk-` 토큰이 masked 상태로만 노출)
11. `tests/real_e2e.rs` — `real_openclaw_profile_diagnostics_flow` +1 (opt-in S9 gate, 기본 NOT-RUN):
    - **read-only 전용**: get-gateway-status, get-update-status, get-agents, get-logs(limit 50) — mutation 0.

### Frontend (`src/`)
1. `src/features/profile/ProfileFeature.tsx` (신규) — 기존 feature와 같은 stacked section 패턴, 4개 섹션:
   - **프로필/agent 목록**: id, default 마커, name/emoji, workspace, bindings — `get-agents`
   - **Update 상태**: 상태 + current/latest 버전("버전 차이") — `get-update-status`
   - **API 상태**: gateway state/version/port — `get-gateway-status`
   - **Diagnostics**: 환경 요약(`detect-environment` 재사용 표시) + 로그 뷰어(limit 선택 50/100/200/500, 기본 200, 새로고침) — `get-logs`
   - 각 섹션: 로딩/에러(한국어, stable code 매핑)/데이터 상태, refresh button
2. `src/features/profile/profileState.ts` (+ `profileState.test.ts`) — 상태 로직 + error code → 한국어 메시지 매핑 (기존 `*State.ts` 패턴)
3. `src/lib/tauri/index.ts` — COMMANDS +4, wrapper +4, wire types +4; `index.test.ts` +4 (kebab 이름 + camelCase args 계약)
4. `src/i18n/ko/index.ts` — `profile` namespace 추가
5. `src/App.tsx` — `<ProfileFeature />` 마운트

### Docs
- 이 파일(계약) — 완료 시 `Status: completed (YYYY-MM-DD)` + exit criteria `[x]`
- `ROADMAP.md` Phase 8 상태 갱신

## Exit Criteria

### 1. IPC command (4개)
- [x] 4개 command 전부 kebab `rename` 속성으로 등록 (Phase 7.5 계약), `ipc_name_contract` **49/49** 통과
- [x] frontend COMMANDS/wrapper/test 4쌍이 Rust side와 1:1 일치

### 2. Profile / Agents
- [x] `get-agents` exact argv `["agents", "list", "--json"]` (contract test)
- [x] fail-soft row parser (unknown 필드 무시, 부재 → None)
- [x] non-zero / top-level malformed → `openclaw-agents-read-failed`
- [x] UI 표시 (한국어, default 마커 포함)

### 3. Update 상태
- [x] `get-update-status` exact argv `["update", "status", "--json"]`, current/latest parse
- [x] parse 실패/부재 → `Unknown` + version None (fail-soft, 신규 error 아님)
- [x] UI: 상태 + current/latest ("버전 차이" 표시)

### 4. API 상태 (gateway)
- [x] `get-gateway-status`가 Phase 1 `gateway_status` 재사용 (re-implementation 0)
- [x] UI: state/version/port

### 5. Diagnostics (logs)
- [x] `get-logs` exact argv `["logs", "--limit", "<n>", "--json"]`; `limit` 1..=1000 사전 검증, 위반 시 **0 CLI** (fail-closed)
- [x] type-tagged event(line-by-line) parse — `log`/`raw`/`meta`/`notice`, non-JSON line → `raw`
- [x] non-zero → `openclaw-logs-read-failed`; empty stdout → 0 lines (성공)
- [x] masking 적용 확인 (fake `sk-` 토큰 masked)
- [x] argv에 `--follow` 없음 (contract assert + fake reject)
- [x] UI 로그 뷰어 (limit 선택 + 새로고침)

### 6. 구조 유지
- [x] Phase 0~7 business logic 변경 0 (신규 파일 + `lib.rs` 등록 + `error.rs` +2 + fake handler + `ipc_name_contract` +4 외 기존 파일 로직 수정 0)
- [x] `ProcessRunner` 단일 spawn boundary 유지 (`Command::new` 유일 `runner.rs`)
- [x] 로그 파일 직접 filesystem 읽기 0 (CLI 경유만)
- [x] secret plain 노출 0 (S3/S8 — 로그·오류·테스트 출력)

### 7. 테스트
- [x] 신규 `diagnostics_contract` 전부 통과
- [x] 기존 Phase 0~7 regression test 전부 유지 (수치 변동은 lib 신규 unit test + ipc_name_contract 49 외 0)
- [x] real E2E read-only flow opt-in 게이트 유지, 기본 `cargo test`에서 NOT-RUN

### 8. 검증
- [x] `cargo fmt --check`, `cargo check --all-targets`, `cargo test --all-targets`, `cargo clippy --all-targets`
- [x] `pnpm typecheck`, `pnpm lint`, `pnpm test`

## Non-Goals

- `--follow` 로그 스트리밍, `channels logs`
- agent add/delete/bind/set-identity (read-only 표시만)
- gateway lifecycle (start/stop/restart/install/uninstall/call/probe)
- 실제 update 실행 (`openclaw update` run/repair/wizard)
- 심층 diagnostics: `doctor`, `status --all/--deep`, `triage`, `gateway diagnostics export`
- usage/quota (`status --usage`), sessions, transcripts, memory
- named profile (`--profile <name>`) — default profile만
- 로그 파일 직접 읽기 (filesystem adapter)
- Phase 9 (Integrated Chat), Phase 10 (Windows Release)

## 완료 후 수동 Smoke Test

자동 검증 완료 후 사용자가 직접 `pnpm exec tauri dev` 실행:

- Profile 영역 4개 섹션(agents/update/gateway/diagnostics)이 command-not-found 없이 로드되는지
- 로그 뷰어가 one-shot tail로 실제(또는 fake) 로그를 masked 상태로 표시하는지

smoke test에서 새로 드러나는 내부 기능 오류는 Phase 8 완료 여부와 분리하여 실제 오류 기준으로 후속 수정한다.
