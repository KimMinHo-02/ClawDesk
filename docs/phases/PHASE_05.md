# Phase 5 — Tools / Security

Status: not started

## 목표

ClawDesk에서 OpenClaw의 **tool permission policy(허용/거부/확인), security profile 선택, security audit(read-only) 표시**를 terminal 없이(한국어 UI) 관리한다.

- 제품 계약 §4.4 "허용/거부/확인" 대응: `tools.allow`(허용, +`tools.profile`), `tools.deny`(거부), `tools.exec.mode`(확인 — human approval 게이트).
- 모든 config mutation은 Phase 3 `OpenClawConfigPort`(`openclaw config` CLI, structured argv, dry-run → commit) 경유만 사용. ClawDesk가 `openclaw.json` 파일을 직접 편집하지 않는다.
- 도구 정책 read/write 대상은 글로벌 `tools.*` path(`tools.profile`, `tools.allow`, `tools.deny`, `tools.exec.mode`)만.
- security profile = 위 도구 정책의 이름 붙은 프리셋(기본 builtin + 사용자 프로필). 저장소는 ClawDesk 소유 경로이며, profile apply는 OpenClaw config write.
- security audit는 `openclaw security audit --json`(cold, read-only, credential 없음)만. `--deep`/`--fix`는 non-goal.
- 모든 프로세스 실행은 Phase 1 `ProcessRunner`(structured `executable + argv`) 경유만. shell string 조합 0 (S1).

설계 근거 (2026-08-22 접속 기준, `docs.openclaw.ai` live docs, `doc-schema-version: 1`):

- `https://docs.openclaw.ai/gateway/config-tools`
- `https://docs.openclaw.ai/gateway/security`
- `https://docs.openclaw.ai/gateway/security/audit-checks`
- `https://docs.openclaw.ai/cli/security`
- `https://docs.openclaw.ai/tools/exec`
- `https://docs.openclaw.ai/cli`
- `https://docs.openclaw.ai/cli/config` (Phase 3 재사용)

핵심 사실 (상기 docs 기준):

- **Tool profile** (`tools.profile`): `minimal`(session_status only) | `coding`(group:fs/runtime/web/sessions/memory + goals/media) | `messaging`(messaging/session tools) | `full`(무제한 — unset과 동일). Local onboarding은 신규 local config에 unset 시 `coding` 기본 설정.
- **Tool allow/deny** (`tools.allow` / `tools.deny`): string 배열, **deny wins**, case-insensitive, `*` wildcard 지원. entry는 tool id(`web_search`, `session_status`, ...), group ref(`group:fs`, `group:runtime`, ...), wildcard pattern(`image*`, `outlook__*`). `write`와 `apply_patch`는 별개 tool id(`write` deny가 `apply_patch`를 막지 않음). 같은 scope에서 `allow`와 `alsoAllow` 동시 설정은 config validation이 거부.
- **Exec policy** (`tools.exec.mode`, canonical persisted policy knob):

  | mode | security | ask | 동작 |
  | --- | --- | --- | --- |
  | `deny` | deny | off | exec 거부 |
  | `allowlist` | allowlist | off | allowlist/safe-bin만 실행 |
  | `ask` | allowlist | on-miss | allowlist 밖은 **human 확인** |
  | `auto` | allowlist | on-miss | auto reviewer 후 human 확인 |
  | `full` | full | off | approval 없음 (unset 기본 — trusted-operator default) |

- **security audit** (`openclaw security audit`):
  - plain audit = cold config/filesystem/read-only path (live Gateway probe 없음, plugin runtime load 없음).
  - `--deep` = live probe + plugin code scan (gateway credential 필요 — non-goal).
  - `--fix` = mutation (group policy flip, 파일 권한/ACL tighten — non-goal).
  - `--json`: 성공 시 stdout에 single JSON document; 실패 시 nonzero exit + `{"ok":false,"error":{"type":"cli_error","message":...}}` envelope (Phase 1 JSON failure envelope 관례).
  - findings는 structured `checkId`(예: `tools.exec.security_full_configured`, `fs.config.perms_world_readable`) + severity(`critical`/`warn`/`info`).
  - suppression된 findings는 `suppressedFindings`로 보고됨 (display-only).
- 재사용 (Phase 3): `openclaw config get <path> --json`(redacted snapshot), `config set <path> '<json>' --strict-json [--replace]` + `--dry-run --json` 2단계 write, config path 동적 해석, ProcessRunner timeout, masking chokepoint, `AppError` stable code.

미확인 항목 (구현 시 live CLI/docs로 확정, 추측 금지):

- `security audit --json`의 findings item 정확 필드 스키마(title/detail field 명칭, summary shape) — docs에 전체 미게재. parser는 `checkId`만 필수, 나머지는 fail-soft → `null`.
- `tools` section unset일 때 `config get tools --json` 출력 shape (null vs object).
- `tools` section이 없는 config에서 `tools.allow` 등 write 시 parent path 자동 생성 동작.
- Windows에서 cold audit(파일/ACL 스캔) 소요 시간 — timeout(60s) 여유 검증.

---

# Exit Criteria

## 1. Tool Policy (Rust)

### 읽기

- [ ] 현재 도구 정책: `openclaw config get tools --json` (read-only, redacted)
- [ ] `ToolPolicy` 파싱: `profile`(string, unset → null), `allow[]`, `deny[]`,
  `exec.mode`(string, unset → null), `elevated.enabled`(read-only),
  `fs.workspaceOnly`(read-only)
- [ ] field 부재 → `null`/빈 배열 (fail-soft, drop 없음)
- [ ] 전체 payload JSON 파싱 실패 → stable structured error (`openclaw-config-read-failed` 재사용)
- [ ] CLI 실패/미설치 시 stable structured error (Phase 1 error code 재사용)
- [ ] 모든 stdout/stderr masking pipeline 경유 (기존 chokepoint 유지)

### 쓰기 (허용/거부/확인)

- [ ] `tools.profile` write: `openclaw config set tools.profile '<enum>' --strict-json`
  (dry-run → commit, Phase 3 flow 재사용)
  - enum: `minimal | coding | messaging | full` — 검증 실패 → `tool-profile-invalid`, CLI 호출 0회
- [ ] `tools.allow` / `tools.deny` write: `openclaw config set tools.allow '<json-array>' --strict-json --replace`
  (배열 전체 replace, dry-run → commit)
  - entry **사전 검증** (S2): non-empty, ≤128 chars, character set `[A-Za-z0-9_:.*-]`,
    whitespace/`/`/`..` 불가; `group:` prefix는 `group:[A-Za-z0-9-]{1,32}`
  - 검증 실패 → `tool-entry-invalid`, CLI 호출 0회
- [ ] `tools.exec.mode` write: `openclaw config set tools.exec.mode '<enum>' --strict-json`
  - enum: `deny | allowlist | ask | auto | full` — 검증 실패 → `exec-mode-invalid`, CLI 호출 0회
- [ ] JSON payload(배열)는 단일 argv 요소 (shell string 조합 0)
- [ ] `ok: false`(dry-run/validate reject) → write 0회 + `openclaw-config-invalid` (Phase 3 재사용)
  - 예: 사용자가 수동으로 `alsoAllow`를 설정해 둔 상태에서 `allow` write 시 validation reject —
    stable error로 원문 표면화 (silent retry 금지)
- [ ] write 후 재조회(read)로 상태 확인 — UI는 응답 후 list 갱신 (optimistic update 금지)

### ToolPolicyService (application layer)

- [ ] `ToolPolicyService`: `get_tool_policy()`, `set_tool_profile(p)`, `set_tool_allow(entries)`,
  `set_tool_deny(entries)`, `set_exec_mode(m)`
- [ ] Phase 3 `OpenClawConfigPort` + `OpenClawPort`(executable detection) 조합
- [ ] structured result + stable `AppError` mapping
- [ ] infra detail(CLI 출력 원문, path) frontend 노출 금지

## 2. Security Profile (Rust)

### 정의 / 저장

- [ ] security profile = named preset: `{ id, name, baseProfile, allow[], deny[], execMode }`
  - `baseProfile`: `minimal | coding | messaging | full`
  - `execMode`: `deny | allowlist | ask | auto | full`
- [ ] **builtin profile 2개 (read-only, 파일에 저장되지 않음)**:
  - `default` ("기본"): baseProfile `coding`, allow `[]`, deny `[]`, execMode `full`
    (OpenClaw onboarding default 반영 — 신규 local config `tools.profile: "coding"`,
    exec default no-approval)
  - `hardened` ("보안 강화"): baseProfile `messaging`, allow `[]`,
    deny `["group:automation","group:runtime","group:fs","sessions_spawn","sessions_send"]`,
    execMode `deny`
    (docs Hardened baseline 반영 — baseline의 `exec.security:"deny"`는 mode `deny`에 대응)
- [ ] **사용자 profile**은 `%APPDATA%\ClawDesk\security-profiles.json`에 저장
  (ClawDesk 소유 경로, S4)
  - file shape: `{ "version": 1, "profiles": [...] }` — secret 0 (tool policy만)
  - file 부재 → 빈 list (fresh); corrupt → `security-profile-store-failed` (fail-closed, 재작성 금지)
  - atomic write (temp + rename)
- [ ] id 검증: `^[a-z][a-z0-9_-]{0,63}$`; builtin 또는 기존 user profile id와 충돌 →
  `security-profile-conflict`
- [ ] name 검증: 1–50 chars, control character 불가 (display-only)
- [ ] store 실패 → `security-profile-store-failed`, config write 0회

### apply

- [ ] `apply-security-profile`: profile의 4 field를 config에 순서대로 write
  (`tools.profile` → `tools.allow` → `tools.deny` → `tools.exec.mode`, 각 dry-run → commit)
  - 4 field 무조건 전체 write (idempotent, 결정적)
  - 첫 실패 시 중단 (partial continue 금지) — UI는 재조회로 실제 상태 표시
- [ ] **적용 상태 판정**: 현재 정책 read와 profile 4 field 비교
  - `tools.profile` unset ≡ `full` (docs: full = unset과 동일)
  - `tools.exec.mode` unset ≡ `full` (host policy default no-approval)
  - 일치 → "profile 적용 중", 불일치 → "custom"
- [ ] builtin apply도 user profile과 동일 path (special-case 0)

### SecurityProfileService (application layer)

- [ ] `SecurityProfileService`: `list_security_profiles()`, `save_security_profile(p)` (upsert),
  `delete_security_profile(id)`, `apply_security_profile(id)`
- [ ] structured result + stable `AppError` mapping
- [ ] builtin profile은 store에 없음 (delete/edit 불가 — builtin id에 대한 delete/edit 시도 →
  `security-profile-not-found`)

## 3. Security Audit (Rust, read-only)

- [ ] `openclaw security audit --json` (cold audit only)
  - `--deep`, `--fix`, `--token`, `--password` 사용 **0회** (argv capture assert)
  - credential은 argv/환경변수/UI 어디에도 입력되지 않음 (S2/S3)
- [ ] findings 파싱: `{ checkId, severity, title, detail }`
  - `checkId` 필수 (부재 row drop), `severity`/`title`/`detail` fail-soft → `null`
  - severity 값: `critical | warn | info` (unknown 값 → raw string 유지, UI "미확인" 매핑)
- [ ] `summary` object: schema assert 없음 (informational display only)
- [ ] `suppressedFindings`: count display only (detail 미노출)
- [ ] nonzero exit → `openclaw-security-audit-failed` (Phase 1 JSON failure envelope parse 재사용)
- [ ] parse 실패/timeout → stable error (`process-timeout` 재사용)
- [ ] masking pipeline: detail/title 전체 masking 경유 (config 유도 detail에 sensitive value 포함 가능)
- [ ] audit 실패 → "감사 실패" 표시 ("clean 상태" 추정 0 — fail-closed)

## 4. 인터페이스

### ports / adapters

- [ ] `domain/ports`에 `OpenClawSecurityPort`(run_security_audit) trait 추가 +
  `infrastructure/openclaw`에 `OpenClawSecurityAdapter` 구현
- [ ] `domain/ports`에 `SecurityProfileStorePort`(list/get/save/delete) + `infrastructure`
  file store 구현 (unit test에서 temp dir으로 교체 가능)
- [ ] tool policy: 신규 port 없음 — `ToolPolicyService`가 Phase 3 `OpenClawConfigPort` 조합
- [ ] 모든 process 실행은 `ProcessPort`/`ProcessRunner` 경유 (spawn 단일 경계 유지)
- [ ] `std::process::Command` 직접 사용 0, shell 사용 0회 (S1)

### commands 레이어 (Tauri IPC)

Phase 4의 `commands` 레이어 확장. frontend kebab-case ↔ Rust snake_case.

| frontend                 | Rust                    | payload → result                                             |
| ------------------------ | ----------------------- | ------------------------------------------------------------ |
| `get-tool-policy`        | `get_tool_policy`       | → ToolPolicy (profile, allow, deny, execMode, elevatedEnabled, fsWorkspaceOnly) |
| `set-tool-profile`       | `set_tool_profile`      | `{ profile }`                                                |
| `set-tool-allow`         | `set_tool_allow`        | `{ entries: string[] }`                                      |
| `set-tool-deny`          | `set_tool_deny`         | `{ entries: string[] }`                                      |
| `set-exec-mode`          | `set_exec_mode`         | `{ mode }`                                                   |
| `list-security-profiles` | `list_security_profiles`| → `{ builtins, users, currentApplied: id \| null, policyReadFailed: bool }` |
| `save-security-profile`  | `save_security_profile` | `{ profile }` → upsert                                       |
| `delete-security-profile`| `delete_security_profile` | `{ profileId }`                                            |
| `apply-security-profile` | `apply_security_profile`| `{ profileId }`                                              |
| `run-security-audit`     | `run_security_audit`    | → `{ summary, findings[], suppressedCount }`                 |

### IPC 계약

architecture §5 준수 (Phase 2–4와 동일 기준):

- [ ] `src/lib/tauri/`에 command name/type 1곳 정의, 중복 정의 금지
- [ ] 명시적 serde 타입만 (wire = camelCase, `serde_json::Value` 중심 임의 계약 금지)
- [ ] `AppError` code 기반 frontend 메시지 매핑

### 신규 stable error code

- [ ] `tool-profile-invalid` (profile enum 검증 실패)
- [ ] `tool-entry-invalid` (allow/deny entry 형식 검증 실패)
- [ ] `exec-mode-invalid` (exec mode enum 검증 실패)
- [ ] `security-profile-id-invalid`
- [ ] `security-profile-name-invalid`
- [ ] `security-profile-not-found` (builtin id에 대한 delete/edit 포함)
- [ ] `security-profile-conflict` (id collision)
- [ ] `security-profile-store-failed` (ClawDesk store read/write 실패)
- [ ] `openclaw-security-audit-failed` (audit 실행/parse 실패)

기존 재사용:

- [ ] `openclaw-config-read-failed`, `openclaw-config-write-failed`, `openclaw-config-invalid` (tool policy write)
- [ ] `openclaw-not-found`, `process-failed`, `process-timeout`

### command safety

- [ ] shell string 조합 0건 (S1)
- [ ] user input(profile, entries, execMode, profile id/name)은 **사전 형식 검증 후**
  argv/config path에 사용 (S2)
- [ ] CLI 호출 timeout: config read/write 30초 (Phase 3 재사용), security audit 60초
- [ ] JSON payload는 단일 argv 요소

## 5. UI

Frontend는 한국어 중심.

위치:

```text
src/features/tools-security/
```

### Tool Policy

- [ ] 현재 정책 표시: profile(한국어 라벨), exec mode(한국어 라벨),
  elevated/workspaceOnly(read-only badge)
- [ ] profile selector (4단계, 한국어 라벨: 최소 / 코딩 / 메신저 / 전체(제한 없음))
- [ ] allow list editor (add/remove, chip 표시) + deny list editor (동일)
  - `group:*`/wildcard 지원 안내, "deny 우선" 안내 문구
  - entry 검증 (frontend) + Rust 재검증 (backend) — backend error를 stable code 기반으로 한국어 표시
- [ ] exec mode selector (5단계: 거부 / 허용 목록만 / 확인 요청 / 자동 검토 후 확인 / 전체 허용)
- [ ] 진행 중 중복 submit 방지 + 변경 후 재조회 (optimistic update 금지)

### Security Profile

- [ ] profile 목록: builtin 2개 + user profiles, 적용중 badge,
  "custom (profile 미일치)" 표시
- [ ] 생성 (현재 정책에서 / builtin에서), 수정 (user만), 삭제 (user만, 확인 다이얼로그)
- [ ] apply — apply 후 tool policy section 재조회
- [ ] backend error를 stable code 기반으로 한국어 표시

### Security Audit

- [ ] "보안 감사 실행" button (중복 실행 방지 guard)
- [ ] 결과: severity badge (심각 / 경고 / 참고 / unknown raw), checkId, 한국어 카테고리
  (known prefix map: fs.* 파일 권한, gateway.* 게이트웨이 노출, tools.* 도구 정책,
  plugins.* 플러그인, skills.* 스킬, channels.* 채널, sandbox.* 샌드박스,
  browser.* 브라우저, hooks.* 훅, security.* 보안)
- [ ] detail 표시 (masked)
- [ ] suppressed count 표시
- [ ] audit 실패 → "감사 실패" 메시지 (clean 상태 추정 0)

### i18n

- [ ] `toolsSecurity` namespace 생성 (한국어)
- [ ] 기존 i18n architecture 유지

### Tauri frontend wrapper

- [ ] `src/lib/tauri/`에 Phase 5 command wrapper 추가 (single source 유지)
- [ ] React에서 process/executable 직접 실행 0 (invoke만 — S10)

### frontend test

- [ ] tool policy edit state 로직 (중복 save 방지, 재조회 트리거, entry add/remove/검증)
- [ ] security profile select/apply state 로직
- [ ] audit run state 로직 (진행 중, fail-closed)
- [ ] error code → 한국어 메시지 매핑 테스트

## 6. 테스트

기본 테스트는 fake CLI fixture만 사용한다 (S5).

### fake CLI 확장 (`fixtures/fake-openclaw/`)

- [ ] `security audit --json` handler: state 기반 findings + summary 출력
  - findings: state 유도(예: `tools.exec.mode`가 `"full"`/unset →
    `tools.exec.security_full_configured` warn) + state `securityAudit.findings`
    section passthrough
- [ ] behavior override 재사용 (malformed/not-json/cli-error/sleep/fail) — audit 대상
- [ ] `config get/set tools.*` — 기존 config 핸들러 재사용 (tools.*는 protected path 아님)
- [ ] argv capture 유지 (기존 패턴)

### contract 테스트 (`tests/`)

happy path:

- [ ] get-tool-policy: exact argv (`config get tools --json`), state → ToolPolicy 파싱
- [ ] set-tool-profile: exact argv (dry-run + commit 2행), state 갱신, re-read 확인
- [ ] set-tool-allow/deny: exact argv (`--strict-json --replace`, JSON 배열 단일 argv 요소),
  state 갱신
- [ ] set-exec-mode: exact argv, state 갱신
- [ ] security profile apply: 4 field write sequence (profile → allow → deny → execMode,
  각 2행), state 갱신
- [ ] run-security-audit: exact argv (`security audit --json`), findings 파싱, suppressedCount

실패/검증:

- [ ] invalid profile enum → `tool-profile-invalid`, CLI 호출 0회
- [ ] invalid entry (`../evil`, `a/b`, `a b`, empty, >128, `group:` 단독) →
  `tool-entry-invalid`, CLI 호출 0회
- [ ] invalid exec mode → `exec-mode-invalid`, CLI 호출 0회
- [ ] invalid profile id/name → `security-profile-id-invalid` /
  `security-profile-name-invalid`, CLI 호출 0회
- [ ] unknown profile id apply/delete → `security-profile-not-found`, CLI 호출 0회
- [ ] id collision (builtin/기존) → `security-profile-conflict`
- [ ] dry-run `ok: false` → 실write 0 (state 불변) + `openclaw-config-invalid`
- [ ] audit: nonzero → `openclaw-security-audit-failed`, malformed → 동일 code,
  timeout → `process-timeout`
- [ ] missing executable → `openclaw-not-found` 재사용
- [ ] **audit argv에 `--deep`/`--fix`/`--token`/`--password`가 절대 포함되지 않음**
  (capture assert)

### unit 테스트

- [ ] ToolPolicy parser: field 부재 → null, nested exec.mode, unknown profile string
  raw 유지 (fail-soft)
- [ ] profile slug / name / entry 검증기 (정상/traversal/whitespace/reject case)
- [ ] profile store (temp dir): save/overwrite/delete/list, corrupt → error, atomic write
- [ ] 적용 상태 판정: unset≡full normalization, partial mismatch → custom
- [ ] audit findings parser: checkId 부재 drop, severity/title/detail fail-soft

## 7. Real E2E (Phase 5 확장, opt-in)

Phase 2–4의 3중 게이트 구조(`--test real_e2e` + `--features real-e2e` +
`CLAWDESK_REAL_E2E=1`)를 유지한다.

- [ ] 기본 `cargo test`에서 real config mutation 0회
- [ ] 조건 불만족 시 self-skip
- [ ] 조건 충족 시에만:
  - [ ] `security audit --json` read-only 실행, findings row 스키마 baseline 검증
    (미확인 항목 확정)
  - [ ] `tools.profile` test-owned round-trip: 현재 값 기록(possible unset) →
    `messaging` set → read-back 확인 → 원복(기존 unset이면 unset) → 복원 확인
- [ ] round-trip 외 기존 user tool policy/profile 변경 0 (복원 보장)

---

# 8. 보안 / Architecture 불변식

### ProcessRunner

- [ ] production process spawn 단일 경계 유지
- [ ] 신규 직접 spawn 없음 (tools/security adapter는 모두 `ProcessPort` 경유)
- [ ] executable + argv, shell command string 0

### Secret handling (S3/S7/S8)

- [ ] credential 처리 0: gateway token/password를 받지 않고, 저장하지 않음
  (`--token`/`--password` 사용 불가)
- [ ] profile store file에 secret 0 (tool policy만)
- [ ] CLI stdout/stderr/error masking chokepoint 재사용 (audit detail에 config 유도
  sensitive value 포함 가능)
- [ ] error serialization에 secret 0 (mask pipeline 경유)

### Fail-closed

- [ ] 검증 실패 (profile/entry/mode/id/name) → CLI 호출 0회
- [ ] dry-run 실패 → write 0회
- [ ] profile store corrupt → 재작성 0, stable error
- [ ] audit 실패 → "감사 실패" 표시 (clean 상태 추정 0)
- [ ] apply partial failure → 재조회로 실제 상태 표시 (optimistic 0)

### user environment

- [ ] `openclaw.json` 직접 파일 편집 0 (CLI 경유만)
- [ ] `security audit --fix`/`--deep` 사용 0
- [ ] gateway process start/stop 없음
- [ ] ClawDesk 소유 경로(%APPDATA%\ClawDesk\) 외 파일 작성 0
  (OpenClaw config는 CLI가 쓰는 것만 — ClawDesk 직접 작성 아님)

### layering

- [ ] commands → application → domain → infrastructure 일방 의존 유지
- [ ] React는 invoke만 (process/executable 접근 0)

---

# 9. 검증 명령

Phase 5 종료 전 아래 명령을 실제 실행한다.

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

- **`security audit --deep`** (live Gateway probe, plugin code scan — gateway credential 필요)
- **`security audit --fix`** (channel policy flip, 파일 권한/ACL tighten mutation)
- **credential 처리** (`--token`/`--password`, gateway auth token/password 관리)
- **`tools.byProvider`** (provider별 tool policy)
- **`tools.toolsBySender`** (sender-scope tool policy — 채널 영역, Phase 6)
- **`tools.elevated.allowFrom`** (sender별 elevated allowlist — 채널 영역)
  - `tools.elevated.enabled`, `tools.fs.workspaceOnly`는 read-only 표시만
- **`approvals`/`exec-policy` CLI** (exec approval 세밀 allowlist — binary path 단위)
- **sandbox 설정** (`agents.defaults.sandbox`, Docker/Podman — Windows 초기 버전 non-goal)
- **`tools.github`** (OAuth identity — secret/OAuth flow)
- **`tools.web`** (web search API key — secret 영역)
- **`tools.loopDetection`**, `tools.codeMode`, `tools.media`, `tools.agentToAgent`,
  `tools.sessions*`
- **`security.installPolicy` 편집**, **`security.audit.suppressions` 편집**
  (audit 표시만)
- **per-agent tool policy** (`agents.entries.*.tools`)
- **DM/group policy** (`dmPolicy`, `allowFrom` — Phase 6)
- **plugin allowlist** (`plugins.allow`)
- **gateway auth/bind/token 관리** (Phase 8 diagnostics 영역)
- **실제 OpenClaw mutation을 기본 테스트에서** (S5)
- **Phase 6 (Channels) 시작**

---

# Phase 종료 보고

`ClawDesk-build` 완료 보고는 다음을 포함한다.

1. 생성/변경 파일
2. 구현 내용
3. PHASE_05.md Exit Criteria 대응표
4. 실행한 검증 명령
5. 실제 검증 결과 (passed / failed / ignored/skipped)
6. 추가 dependency와 이유
7. 실제 OpenClaw 또는 시스템에 실행한 명령
8. 기존 ProcessRunner / security boundary 유지 확인
9. 미검증 사항
10. Phase 5 Non-Goals 미구현 확인
11. Phase 6 미시작 확인

real E2E를 실행하지 않았다면 반드시:

```text
real E2E: NOT-RUN
```

으로 기록한다.

실행하지 않은 검증을 PASS라고 쓰지 않는다.

Phase 5 완료 후 Phase 6로 넘어가지 않는다.
