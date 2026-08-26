# Phase 7 — Automations

Status: completed (2026-08-25)

## 목표

ClawDesk에서 **automation job의 목록/상세/생성/수정/활성화(enable/disable)/삭제**를 terminal 없이(한국어 UI) 관리한다.

- 제품 계약 §4.6: "automation(예약/트리거 작업) 목록·생성·수정·삭제 UI", "실행은 OpenClaw 경유".
- Automations는 **OpenClaw 내장 스케줄러**다. 스케줄 발화는 **Gateway process** 실행 중일 때만 일어난다. ClawDesk는 job을 직접 실행하지 않으며, 수동 실행(`automations run`)은 non-goal이다.
- job 정의/실행 상태/히스토리는 OpenClaw **공유 SQLite state DB**에 영속화된다. ClawDesk는 read-only consumer만 한다 (직접 편집 0).
- 모든 automation 작업은 `openclaw automations` CLI(structured argv, timeout 30초) 경유만. `openclaw cron` alias 사용 0.
- payload 노출은 2종: **리마인더**(`--system-event` + main session + `--wake`)와 **예약 작업**(`--message` + isolated session). command/script payload는 unattended host code execution surface로 GUI non-goal이다.
- 모든 프로세스 실행은 Phase 1 `ProcessRunner`(structured `executable + argv`) 경유만. shell string 조합 0 (S1).

설계 근거 (2026-08-25 접속 기준, `docs.openclaw.ai` live docs):

- `https://docs.openclaw.ai/automation` (automation mechanism overview)
- `https://docs.openclaw.ai/automation/cron-jobs` (Automations 상세)
- `https://docs.openclaw.ai/cli/cron` (`openclaw automations` CLI reference)
- 재사용: Phase 1 `ProcessRunner`, Phase 5/6 adapter/contract pattern (exact argv, fail-soft, argv capture)

핵심 사실 (상기 docs 기준):

- **Automations** = OpenClaw 내장 스케줄러. Gateway process 내부에서 실행(Gateway 실행 중일 때만 schedule 발화), job 정의/실행 상태/히스토리는 **공유 SQLite state DB**에 영속.
- **CLI**: `openclaw automations`(primary). `openclaw cron`은 alias — **ClawDesk는 `automations`만 사용**.
- **명령 surface**:
  - `list [--all] [--json]` — list만 기본 human-readable, **`--json` 필수**. `--all`은 disabled job 포함
  - `get <job-id> [--json]` — 기본 job JSON
  - `show <job-id>` — human display
  - `enable <job-id>` / `disable <job-id>` — 기본 JSON
  - `edit <job-id> <flags>` — 기본 JSON
  - `add` / `create <flags>` — 기본 JSON (`--json` 명시 옵션 허용)
  - `remove <job-id>` — 기본 JSON (alias `rm`/`delete`)
  - `run` — 강제 실행, `runs` — 실행 히스토리 (둘 다 non-goal)
  - `--json`은 add/create·status·enable·disable·remove·run·edit·get·runs가 explicit machine-output용으로 허용
- **schedule 5종**:
  - `at` (`--at`): ISO 8601 또는 relative(`20m`). timezone 없는 datetime은 UTC 처리, `--tz <iana>`로 offset-less 해석 가능
  - `every` (`--every`): fixed interval (`10m`/`1h`/`1d`)
  - `cron` (`--cron`): 5/6-field cron expression + 선택 `--tz` (croner — dom/dow 비wildcard 동시 지정 시 OR semantics)
  - `on-exit`, `stream` (non-goal)
- **payload 4종** (+system heartbeat):
  - `--system-event <text>` — main session enqueue, model call 없음
  - `--message <text>` — model-backed agent turn
  - `--command <shell>` / `--command-argv <json>` — host shell/process, model 없음 (**operator-admin surface**)
  - `--script <file|->` — headless code mode, owning agent의 **full tool policy 포함 exec** (docs "unattended code execution" 경고)
- **`--session`**: `main | isolated | current | session:<id>`. CLI(무 session context) agent-turn job은 **isolated fallback**.
- main session job: system event enqueue + 선택 `--wake now|next-heartbeat`.
- isolated job: announce delivery가 **default** (`--no-deliver`로 내부 유지). **다중 채널 host에서는 `--channel` 명시 필요 가능**.
- **job 편집**: schedule flag는 add/edit 공통. `--model/--fallbacks/--thinking`, `--webhook`, delivery routing(`--channel/--to/--thread-id/--account`), `--failure-alert*`, `--pacing-*`, `--light-context`, `--display-name`, `--agent`, `--trigger-script`, stream flag 등 광범위 옵션 존재 (ClawDesk non-goal).
- **실패 안전장치**: 연속 실패 10회 시 auto-disable(`state.autoDisabled.reason`), `list --all`로 disabled 확인, `enable`으로 복구.
- CLI `list --json` job row는 top-level `status` field 포함(`disabled/running/ok/error/skipped/idle`).
- 모든 mutation은 `operator.admin` 필요(로컬 CLI 기본 충족).

미확인 항목 (구현 시 live CLI/docs로 확정, 추측 금지):

1. `automations list --all --json` exact row 스키마 (id field 명: `id`/`jobId`, schedule/payload/state 필드 구조)
2. `automations get <id> --json` exact job 정의 스키마
3. `add`/`create` 결과 JSON의 job id field 명 (`ok:true` envelope 여부)
4. `edit`의 schedule kind 전환 acceptance (at→cron 등) 및 payload text flag(`--system-event`/`--message`) edit acceptance
5. job id 형식/길이 (validation pattern 확정)
6. unknown job id / invalid schedule 실패 시 envelope 형식
7. `--wake` 미지정 main+system-event job의 기본 wake 동작
8. `--at`에 explicit offset ISO(Z 접미) acceptance (docs는 UTC 처리 명시, Z 형식 live 확인)
9. `add`의 `--json` 명시 flag와 기본 JSON 출력 동등성

---

# Exit Criteria

## 1. 검증 (Rust, S2)

- [ ] `validate_automation_id` (IPC로 들어오는 job id — list 결과에서 가져온 id지만 IPC surface 검증):
  - `^[A-Za-z0-9._:-]{1,64}$`
  - 위반 → `automation-id-invalid`, CLI 호출 0회
- [ ] `validate_automation_name`: trim 후 non-empty, ≤128자, control character 0건
  - 위반 → `automation-name-invalid`, CLI 0회
- [ ] `validate_schedule` {kind, value, tz}:
  - `kind` ∈ {at, every, cron}
  - `at`: explicit UTC ISO 8601만 (**Z 또는 ±offset**, 예: `2027-02-01T16:00:00Z`) —
    offset-less datetime은 reject (timezone 해석을 OpenClaw에 위임하지 않음)
  - `every`: `^[1-9][0-9]*[mhd]$` (UI가 value+unit(분/시간/일)을 이 형식으로 변환)
  - `cron`: 5/6 whitespace field, 각 field `^[\d*,/-]+$`
    (의미 검증은 OpenClaw CLI 위임 — fail 시 stable error)
  - `tz`: **cron 전용 선택** IANA `^[A-Za-z0-9+/_-]{1,64}$`
    - at/every + tz → reject
  - 위반 → `automation-schedule-invalid`, CLI 0회
- [ ] `validate_automation_payload` {kind, text}:
  - text trim 후 non-empty, ≤8000자 — 위반 → `automation-payload-invalid`, CLI 0회
  - payload kind ↔ session **고정 페어링**: reminder → `main`/`--system-event`,
    task → `isolated`/`--message` — IPC wire에 session field 없음(서버가 고정)
- [ ] 모든 user text(name/text/tz/cron)는 **단일 argv 요소**로만 전달 —
  shell string 조합 0건 (S1/S2)

## 2. 목록 / 상세 (Rust, read-only)

- [ ] `get-automations`:
  - exact argv: `automations list --all --json` (timeout 30초) — disabled job 포함
  - row parse fail-soft: `id` 필수(부재 row drop), name/enabled/status/nextRunAt/schedule/payload
    unknown field raw 유지 (schema assert 없음 — 미확인 항목 1)
  - `status` top-level field: unknown 값 raw 유지 (UI "미확인" 매핑)
- [ ] `get-automation` {jobId}:
  - id 사전 검증(fail-closed) → exact argv: `automations get <jobId> --json` (timeout 30초)
  - detail: name, schedule (kind/value/tz), payload (kind/text), enabled, status
  - fail-soft: unknown field/status raw 유지 (미확인 항목 2)
- [ ] job row/detail의 payload text는 user content이므로 노출 가능(plaintext 0 대상 아님),
  다만 secret 형식 값이 들어가면 masking chokepoint가 일반 masking 적용 (S8)
- [ ] CLI 실패/parse 실패 → `openclaw-automations-failed` (§5)
  - missing executable → `openclaw-not-found` (재사용)
  - timeout → `process-timeout` (재사용)
- [ ] `show` (human display), `run`, `runs` — 0회 (non-goal)

## 3. 생성 (Rust)

- [ ] `create-automation` {name, scheduleKind, scheduleValue, scheduleTz, payloadKind, text, wake}:
  - 사전 검증 전부 통과 (§1) — 위반 시 CLI 0회
  - exact argv (user text는 단일 argv 요소, 공백/따옴표 포함 byte-match):
    - reminder:
      `automations add --name <name> --at <utc-iso> | --every <dur> | --cron <expr> [--tz <iana>] --session main --system-event <text> --wake <now|next-heartbeat> --json`
    - task:
      `automations add --name <name> --at <utc-iso> | --every <dur> | --cron <expr> [--tz <iana>] --session isolated --message <text> --json`
  - 일회성 (`at`)은 UI local datetime → **UTC ISO 8601 (Z 접미)** 변환해 전송
  - `wake`: reminder 전용, `now|next-heartbeat` (기본 `now`) — reminder argv는 `--wake` 항상 emit(기본 `now`), task argv `--wake` 0회
  - 결과 JSON에서 job id 추출 (fail-soft — 부재/파싱 불가 → `openclaw-automations-failed`,
    field 명은 미확인 항목 3)
- [ ] `--model`/`--webhook`/`--command`/`--command-argv`/`--script`/`--trigger-script`/
  `--channel`/`--to`/`--thread-id`/`--account`/`--agent` 등 non-goal flag는
  **절대 emit하지 않음** (argv capture assert)

## 4. 수정 / 활성화 / 삭제 (Rust)

- [ ] `update-automation` {jobId, name, scheduleKind, scheduleValue, scheduleTz, payloadKind, text, wake}:
  - id 검증 + 전체 필드 사전 검증 (fail-closed, CLI 0회)
  - exact argv:
    `automations edit <jobId> --name <name> --at <utc-iso> | --every <dur> | --cron <expr> [--tz <iana>] --system-event <text> [--wake <now|next-heartbeat>] | --message <text> --json`
  - **payload kind는 현재 job과 동일** (kind 변경 = 삭제+재생성, UI 안내)
  - reminder는 `--wake` 기본 `now` emit, task는 0회
  - edit의 schedule flag acceptance는 add 공통 (schedule kind 전환, payload text flag
    acceptance는 미확인 항목 4)
- [ ] `set-automation-enabled` {jobId, enabled}:
  - id 검증(fail-closed) → `automations enable <jobId> --json` | `automations disable <jobId> --json` (30초)
- [ ] `delete-automation` {jobId}:
  - id 검증(fail-closed) → `automations remove <jobId> --json` (30초) — alias `rm`/`delete` 사용 0
- [ ] 전부: id 검증 fail-closed(CLI 0회), nonzero → envelope parse → stable code
  (`openclaw-automations-failed`, §5)

## 5. 인터페이스

### ports / adapters

- [ ] `domain/ports`에 `OpenClawAutomationsPort` trait + `infrastructure/openclaw`
  `OpenClawAutomationsAdapter` 구현 (Phase 5/6 adapter pattern):
  - `list_automations`, `get_automation`, `add_automation`, `edit_automation`,
    `set_automation_enabled`, `remove_automation`
  - timeout 30초, exact argv, fail-soft parse
- [ ] 모든 process 실행은 `ProcessPort`/`ProcessRunner` 경유 (spawn 단일 경계 유지)
- [ ] `std::process::Command` 직접 사용 0, shell 사용 0회 (S1)

### commands 레이어 (Tauri IPC)

Phase 4–6의 `commands` 레이어 확장. frontend kebab-case ↔ Rust snake_case.

| frontend                | Rust                    | payload → result                                                                 |
| ----------------------- | ----------------------- | -------------------------------------------------------------------------------- |
| `get-automations`       | `get_automations`       | → `{jobs[]}`                                                                     |
| `get-automation`        | `get_automation`        | `{jobId}` → job detail                                                           |
| `create-automation`     | `create_automation`     | `{name, scheduleKind, scheduleValue, scheduleTz, payloadKind, text, wake?}` → `{jobId}` |
| `update-automation`     | `update_automation`     | `{jobId, name, scheduleKind, scheduleValue, scheduleTz, payloadKind, text, wake?}` |
| `set-automation-enabled`| `set_automation_enabled`| `{jobId, enabled}`                                                               |
| `delete-automation`     | `delete_automation`     | `{jobId}`                                                                        |

### IPC 계약

architecture §5 준수 (Phase 2–6와 동일 기준):

- [ ] `src/lib/tauri/`에 command name/type 1곳 정의, 중복 정의 금지
- [ ] 명시적 serde 타입만 (wire = camelCase, `serde_json::Value` 중심 임의 계약 금지)
- [ ] `AppError` code 기반 frontend 메시지 매핑

### 신규 stable error code

- [ ] `automation-id-invalid`
- [ ] `automation-name-invalid`
- [ ] `automation-schedule-invalid`
- [ ] `automation-payload-invalid`
- [ ] `openclaw-automations-failed` (automations list/get/add/edit/enable/disable/remove 실행/parse 실패)
- [ ] 전부 `AppError::new`/`invalid_input` 경유 (masking chokepoint, S8)

### 기존 재사용

- [ ] `openclaw-not-found`, `process-failed`, `process-timeout`

### command safety

- [ ] shell string 조합 0건 (S1)
- [ ] user input(jobId/name/text/tz/cron/wake)은 **사전 형식 검증 후**
  argv에 사용 (S2)
- [ ] CLI 호출 timeout: 전부 30초
- [ ] user text는 단일 argv 요소

## 6. UI

Frontend는 한국어 중심.

위치:

```text
src/features/automations/
```

(Phase 6 channels 구조 템플릿)

### Job 목록 / 상세

- [ ] job 카드/목록: name, schedule 인간 가독 표시, 타입(리마인더/예약 작업),
  enabled badge, status (fail-soft — unknown raw 값은 "미확인")
  - 일회성 (`at`) → local datetime 표시
  - 간격 (`every`) → "10분마다" 류 (분/시간/일)
  - `cron` → raw expression + tz 표시
- [ ] 상세: schedule (kind/value/tz), payload text, enabled, status

### 생성 / 수정 폼

- [ ] name 입력 (frontend 검증 + Rust 재검증)
- [ ] schedule kind 3택 (일회성 / 반복 간격 / cron):
  - 일회성: `datetime-local` 입력 → UTC ISO 8601 (Z 접미) 변환해 전송
  - 간격: 숫자 + 단위 선택(분/시간/일) → `^[1-9][0-9]*[mhd]$` 형식
  - cron: expression text 입력 (5/6-field 형식 검증) + 선택 IANA timezone 입력
    (기본 host timezone 안내)
- [ ] job type 2택 (리마인더 / 예약 작업):
  - 리마인더: 내용 입력 + wake 선택(지금 / 다음 하트비트, 기본 지금)
  - 예약 작업: 내용 입력 (wake 선택 없음)
- [ ] payload text: non-empty, ≤8000 frontend 검증 + Rust 재검증
- [ ] 수정: payload kind 변경 불가 — 안내 "타입을 바꾸려면 삭제 후 새로 만듭니다"
- [ ] enable/disable — disable(무효화) 시 확인 다이얼로그
- [ ] 삭제 — 확인 다이얼로그
- [ ] optimistic update 0 — mutation 완료(성공/실패) 후 재조회
- [ ] 진행 중 중복 submit guard

### i18n

- [ ] `automations` namespace 생성 (한국어)
  - 신규 error code 5개 + 재사용 code(`openclaw-not-found`, `process-failed`, `process-timeout`) 매핑 포함
- [ ] 기존 i18n architecture 유지

### Tauri frontend wrapper

- [ ] `src/lib/tauri/`에 Phase 7 command wrapper 6개 추가 (single source 유지)
- [ ] React에서 process/executable 직접 실행 0 (invoke만 — S10)

### frontend test

- [ ] schedule 검증 (3종 정상/reject, cron field 형식, at+tz reject)
- [ ] form state (중복 guard, 재조회 counter)
- [ ] error code → 한국어 메시지 매핑 테스트

## 7. 테스트

기본 테스트는 fake CLI fixture만 사용한다 (S5).

### fake CLI 확장 (`fixtures/fake-openclaw/`)

- [ ] `automations list --all --json` handler:
  state `automations.jobs`(array) passthrough, `{"ok":true,"jobs":[...]}` 출력
- [ ] `automations get <id> --json` handler:
  state 조회, 부재 시 nonzero + `cli_error` envelope
- [ ] `automations add <flags>` handler:
  - flag 파싱 (`--name`/`--at`/`--every`/`--cron`/`--tz`/`--session`/`--system-event`/`--message`/`--wake`)
  - state에 job 추가 (id `job-<n>` 생성), `{"ok":true,"id":...}` 출력
  - **non-goal flag(`--command`/`--script`/`--webhook`/`--model`/`--channel`/`--to`)가 argv에 있으면 reject**
    (exit 2 또는 envelope — 회귀 차단)
- [ ] `automations edit <id> <flags>` handler: 동일 flag set으로 state 갱신
- [ ] `automations enable <id>` / `disable <id>` / `remove <id>` handler: state 갱신
- [ ] `automations run` / `runs` — **handler 없음** (unsupported exit 2 — non-goal 회귀 차단)
- [ ] behavior_override + argv capture 기존 패턴 재사용

### contract 테스트 (`tests/automations_contract.rs`, Phase 5/6 구조)

happy path:

- [ ] get-automations: exact argv (`automations list --all --json`), row parse (`id` 필수, fail-soft)
- [ ] get-automation: exact argv, detail parse
- [ ] create (reminder): exact argv (`--session main --system-event <text> --wake now` 포함,
  text 단일 argv byte-match — 공백/따옴표 포함)
- [ ] create (task): exact argv (`--session isolated --message <text>`, `--wake` 0회)
- [ ] update: exact argv, state 갱신
- [ ] enable/disable: exact argv + state 갱신
- [ ] delete: exact argv + state 갱신 (job 제거)
- [ ] **전 flow argv capture에 non-goal flag(`--command`/`--script`/`--webhook`/`--model`/`--channel`/`--to`/`--agent`) 0회** (assert)

실패/검증:

- [ ] invalid job id → `automation-id-invalid`, CLI 0회
- [ ] invalid name (빈/과대/control char) → `automation-name-invalid`, CLI 0회
- [ ] schedule reject (at offset-less, every `0m`/`1x`/빈, cron 4-field/불법 char, at+tz, every+tz)
  → `automation-schedule-invalid`, CLI 0회
- [ ] payload reject (빈 text, >8000자, task+wake) → `automation-payload-invalid`, CLI 0회
- [ ] unknown job id get/edit/remove → nonzero → `openclaw-automations-failed`
- [ ] missing executable → `openclaw-not-found` (재사용)
- [ ] timeout → `process-timeout` (재사용)
- [ ] malformed JSON → `openclaw-automations-failed`
- [ ] Unicode (한국어 name/text, 공백/따옴표/따옴표 포함) text가 단일 argv 요소로 byte-match 전달

### unit 테스트

- [ ] 검증기 6종 (id, name, schedule 3종, payload — 정상/reject case)
- [ ] job row/detail parser fail-soft (`id` 필수 drop, unknown field raw 유지)
- [ ] service flow (검증 fail-closed 0-CLI, exact argv 순서)

## 8. Real E2E (Phase 7 확장, opt-in)

Phase 2–6의 3중 게이트 구조(`--test real_e2e` + `--features real-e2e` +
`CLAWDESK_REAL_E2E=1`)를 유지한다.

- [ ] 기본 `cargo test`에서 real mutation 0회
- [ ] 조건 불만족 시 self-skip
- [ ] 조건 충족 시에만:
  - [ ] `automations list --all --json` read-only 실행, 출력 스키마 baseline 보고
    (보고만, assert 0 — 미확인 항목 1/2 확정)
  - [ ] test-owned job round-trip:
    **far-future `--at` (2099-01-01T00:00:00Z) 리마인더 job 생성 → get read-back 확인 → remove → get 부재 확인**
    — 발화 불가능한 inert job(수초 존재), user job 0 건드림
  - [ ] `automations run` 실행 **절대 0회** (강제 실행 = real agent turn 발화 — 금지)
- [ ] `automations` CLI 미지원(구 버전) → skip + NOT-RUN 보고
- [ ] round-trip 외 user job/policy 상태 변경 0

## 9. 보안 / Architecture 불변식

### ProcessRunner

- [ ] production process spawn 단일 경계 유지
- [ ] 신규 직접 spawn 없음 (automations adapter는 `ProcessPort` 경유)
- [ ] executable + argv, shell command string 0

### non-goal flag 차단

- [ ] non-goal flag emit 0 (`--command`/`--command-argv`/`--script`/`--trigger-script`/
  `--webhook`/`--channel`/`--to`/`--thread-id`/`--account`/`--model`/`--fallbacks`/`--thinking`/
  `--agent`) — argv capture assert로 고정

### 실행 위임

- [ ] `automations run`/`runs` CLI 0 (ClawDesk는 job을 실행하지 않음 — Gateway 스케줄러 위임)

### Secret handling (S3/S8)

- [ ] user text(payload text, name)는 masking chokepoint 경유 (일반 secret 패턴 masking)
- [ ] error serialization에 secret 0 (mask pipeline 경유)

### Fail-closed

- [ ] 검증 실패 전부 → CLI 0회
- [ ] nonzero → stable code (envelope parse)
- [ ] create 결과 job id 추출 실패 → `openclaw-automations-failed` (silent success 0)

### user environment

- [ ] `openclaw.json` 직접 파일 편집 0 (automations는 CLI만 — config 경로 없음)
- [ ] SQLite state DB 직접 편집 0
- [ ] gateway process start/stop/restart 0 (Phase 8 영역)
- [ ] repository 경계 밖 파일 작성 0 (본 phase 신규 file store 없음)

### layering

- [ ] commands → application → domain → infrastructure 일방 의존 유지
- [ ] React는 invoke만 (process/executable 접근 0)

## 10. 검증 명령

Phase 7 종료 전 아래 명령을 실제 실행한다.

```text
cargo check
cargo test
cargo clippy
cargo fmt --check
pnpm typecheck
pnpm test
pnpm lint
```

모두 통과해야 한다.

real E2E는 opt-in이므로 기본 Phase 종료 검증에서 필수 실행하지 않는다.

실행하지 않은 경우:

```text
real E2E: NOT-RUN
```

이라고 명시한다.

---

# Non-Goals

이 Phase에서 구현하지 않는다.

- **command payload** (`--command`/`--command-argv`), **script payload** (`--script`),
  **condition trigger** (`--trigger-script`), **stream/on-exit schedule**
  — unattended host code execution surface, GUI 노출 부적합
- **delivery routing 설정** (`--channel`/`--to`/`--thread-id`/`--account`/`--webhook`)
  — isolated job은 CLI default(announce delivery) 위임. 다중 채널 라우팅 모호 시
  stable error 표면화, 설정 UI 없음
- **per-job model/fallback/thinking override** (`--model`/`--fallbacks`/`--thinking`/`--clear-*`)
- **`current`/`session:<id>` custom session**
- **수동 실행** (`automations run`), **실행 히스토리** (`automations runs`)
  — Phase 8 diagnostics 영역
- **failureAlert tuning** (`--failure-alert*`), **pacing** (`--pacing-*`),
  **`--light-context`**, **`--display-name`**, **multi-agent** (`--agent`), **scratch**, **hooks**,
  **heartbeat 설정**
- **Task Flow, standing orders, Gmail PubSub** (기타 automation mechanism)
- **webhook endpoint 설정**, **`doctor --fix`**
- **gateway start/stop/restart/lifecycle** (Phase 8 diagnostics 영역)
- **실제 OpenClaw mutation을 기본 테스트에서** (S5)
- **Phase 8 (Profile / Update / Diagnostics) 시작**

---

# Phase 종료 보고

`ClawDesk-build` 완료 보고는 다음을 포함한다.

1. 생성/변경 파일
2. 구현 내용
3. PHASE_07.md Exit Criteria 대응표
4. 실행한 검증 명령
5. 실제 검증 결과 (passed / failed / ignored/skipped)
6. 추가 dependency와 이유
7. 실제 OpenClaw 또는 시스템에 실행한 명령
8. 기존 ProcessRunner / security boundary 유지 확인
9. 미검증 사항
10. Phase 7 Non-Goals 미구현 확인
11. Phase 8 미시작 확인

real E2E를 실행하지 않았다면 반드시:

```text
real E2E: NOT-RUN
```

으로 기록한다.

실행하지 않은 검증을 PASS라고 쓰지 않는다.

Phase 7 완료 후 Phase 8로 넘어가지 않는다.
