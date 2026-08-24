# Phase 4 — Skills / Plugins

Status: completed (2026-08-22)

## 목표

ClawDesk에서 OpenClaw의 **skills·plugins 목록 조회, 활성화/비활성화, plugin 실행
상태 표시**를 terminal 없이(한국어 UI) 관리한다.

- 모든 OpenClaw 작업은 Phase 1–3의 `ProcessRunner`(structured `executable + argv`)
  경유만 사용. shell string 조합 0 (S1).
- skill 상태 변경은 `openclaw config set skills.entries.<name>.enabled`
  (Phase 3 `OpenClawConfigPort`의 dry-run → write 2단계) 경유.
- plugin 상태 변경은 dedicated CLI `openclaw plugins enable/disable <id>` 경유.
- **install/update/uninstall/search/verify/workshop/registry 등 plugin·skill
  라이프사이클 mutation은 Phase 4에 없음** (제품 계약 1.0 §4.2/4.3은 목록 +
  활성화 관리 + 실행 상태 표시만 요구).

설계 근거 (2026-08-21 접속 기준, `docs.openclaw.ai` live docs, `doc-schema-version: 1`):

- `https://docs.openclaw.ai/tools/skills`
- `https://docs.openclaw.ai/cli/skills`
- `https://docs.openclaw.ai/tools/plugin`
- `https://docs.openclaw.ai/cli/plugins`

핵심 사실 (상기 docs 기준):

- **Skills**:
  - skill = `SKILL.md`(YAML frontmatter `name`/`description`)를 가진 디렉터리.
    로드 우선순위: workspace → project agent → personal agent → managed/state →
    bundled → extra dirs + plugin skills.
  - CLI: `openclaw skills list [--eligible] [--verbose] [--json] [--agent <id>]`,
    `openclaw skills info <name> [--json]`. `--json`이면 기계 판독 payload가
    stdout에 출력된다.
  - **`skills enable/disable` CLI 명령은 없다.** 활성화/비활성화는 config
    override `skills.entries.<key>.enabled`(boolean)이다: `false`는 bundled/
    설치 skill도 무효화한다. entry key는 기본 skill name(`metadata.openclaw
    .skillKey` 정의 시 그 값).
  - gating: `metadata.openclaw.requires`(bins/env/config) 등으로 load 시점에
    필터링 — skill은 enabled여도 ineligible할 수 있다.
  - **스냅샷**: skill 목록은 세션 시작 시 스냅샷. skills/config 변경은
    **다음 새 세션**부터 적용(watcher 감지 시 그 다음 turn).
- **Plugins**:
  - CLI: `openclaw plugins list [--enabled] [--verbose] [--json]`,
    `openclaw plugins enable <id>`, `openclaw plugins disable <id>`,
    `openclaw plugins inspect <id> [--runtime] [--json]`.
  - `plugins list`는 **cold read**(persisted local plugin registry + manifest
    fallback) — 이미 실행 중인 Gateway의 live runtime probe가 아니다.
    `--json`은 machine-readable inventory + registry diagnostics +
    package dependency install state(`dependencyStatus`) 포함.
  - `plugins enable/disable <id>`는 **config + cold registry 갱신**
    (install/update/uninstall과 달리 Gateway restart 불필요).
    Nix mode(`OPENCLAW_NIX_MODE=1`)에서는 enable/disable 모두 거부(nonzero exit).
  - **실행 상태**: `plugins inspect <id> --runtime --json`이 live runtime
    surface(registered hooks, tools, commands, services, gateway methods, HTTP
    routes)의 증명. plain `inspect`는 cold manifest/registry 확인만.
  - plugin id: npm 스타일(`@openclaw/<package>` scoped 포함). `plugins.list`
    row에서 id를 가져와 사용한다(사용자 free-text 입력이 아님).
- 재사용 (Phase 3):
  - `openclaw config set <path> '<json>' --strict-json` + `--dry-run --json`
    2단계 write (skills.entries 사용).
  - config path 동적 해석(`openclaw config file --json`), ProcessRunner timeout,
    masking chokepoint, `AppError` stable code.

미확인 항목 (구현 시 live CLI/docs로 확정, 추측 금지):

- `skills list --json` / `plugins list --json` / `plugins inspect --runtime
  --json`의 **완전한 row 스키마** — docs에 전체 미게재. 구현 시 live docs/
  실출력 기준 latest stable 형태를 fixture에 반영. parser는 필수 필드만 요구,
  선택 필드 부재 → `null`(fail-soft).
- `skills list --json` row가 entry key(`skillKey` override)를 노출하는지 —
  미노출 시 skill name을 entry key로 사용 (skillKey override 지원은 non-goal).
- `plugins enable/disable`의 non-interactive 동작(prompt 부재) — docs상
  prompt는 uninstall에만 명시. 구현 시 실출력 확인.

---

# Exit Criteria

## 1. Skills (Rust)

### 읽기

- [ ] skill 목록: `openclaw skills list --json` (read-only)
- [ ] row 파싱: 최소 필수 `name`(string), `enabled`(bool), `eligible`(bool)
  — docs 기준 flag 명칭이 다를 경우 live output으로 확정 (미확인 항목)
- [ ] 선택 필드(`description`, `source`/`origin` 등) 부재 → `null`, parse 실패
  전체(row drop 없음 — row는 name만 있으면 유지, 나머진 null)
- [ ] 전체 payload JSON 파싱 실패 → stable structured error (아래 error code)
- [ ] CLI 실패/미설치 시 stable structured error (Phase 1 error code 재사용)
- [ ] 모든 stdout/stderr masking pipeline 경유 (기존 chokepoint 유지)

### 활성화/비활성화 (config 경유)

- [ ] `skills.entries.<name>.enabled` leaf write:
  `openclaw config set skills.entries.<name>.enabled '<true|false>' --strict-json`
  (Phase 3 write flow 재사용: `--dry-run --json` 우선 → `ok: true` 확인 → 실write)
- [ ] write 전 `skills list --json`에서 해당 skill 존재 확인 (없는 skill →
  `skill-not-found` 계열 stable error, write 0회)
- [ ] `ok: false` → write 0회 + `openclaw-config-invalid` (Phase 3 재사용)
- [ ] toggle 후 재조회(list)로 상태 확인 — UI는 응답 후 list 갱신
- [ ] "변경은 새 세션부터 적용" UI 안내 문구 (i18n)

### SkillsService (application layer)

- [ ] `SkillsService`: `list_skills()`, `set_skill_enabled(name, enabled)`
  (list port + config port 조합)
- [ ] structured result + stable `AppError` mapping
- [ ] infra detail(CLI 출력 원문, path) frontend 노출 금지

## 2. Plugins (Rust)

### 읽기

- [ ] plugin 목록: `openclaw plugins list --json` (cold read, read-only)
- [ ] row 파싱: 최소 필수 `id`(string), `enabled`(bool)
  — 선택 필드(`name`, `format`, `origin`/`source`, `version`,
    `dependencyStatus`) 부재 → `null` (fail-soft, 위 skills와 동일 원칙)
- [ ] 전체 payload 파싱 실패 → stable structured error

### 활성화/비활성화 (dedicated CLI 경유)

- [ ] `openclaw plugins enable <id>` / `openclaw plugins disable <id>`
  (structured argv — id는 단일 argv 요소)
- [ ] nonzero exit → stable error + 재조회(list)로 실제 상태 표시
    (CLI가 state를 이미 바꿨을 수 있음 — UI는 optimistic 하지 않고
    재조회 결과를 표시)
- [ ] Nix mode 거부(nonzero) → 동일 error path (특별 처리 없음)

### 실행 상태 (runtime inspect)

- [ ] `openclaw plugins inspect <id> --runtime --json`
- [ ] row 파싱: `id` + 등록된 surface(`tools`, `hooks`, `services`,
  `cliCommands`, `gatewayMethods`, `routes` — 실제 명칭은 live output으로
  확정, 부재 → 빈 배열) + diagnostics(선택)
- [ ] runtime inspect timeout(아래 60초) → `process-timeout` (Phase 1 재사용)
- [ ] unknown id → nonzero exit → stable error

### PluginsService (application layer)

- [ ] `PluginsService`: `list_plugins()`, `set_plugin_enabled(id, enabled)`,
  `get_plugin_runtime(id)`
- [ ] structured result + stable `AppError` mapping
- [ ] infra detail frontend 노출 금지

### ports / adapters

- [ ] `domain/ports`에 `OpenClawSkillsPort`(list), `OpenClawPluginsPort`(list,
  enable, disable, runtime inspect) trait 추가 (Phase 3
  `OpenClawConfigPort` 패턴 — adapter는 port만 알 수 있음)
- [ ] `infrastructure/openclaw`에 `OpenClawSkillsAdapter`,
  `OpenClawPluginsAdapter` 구현 (기존 `ProcessPort`/`ProcessRunner` 경유,
  Phase 1 executable resolution 재사용)
- [ ] skill toggle은 신규 adapter 없이 Phase 3 `OpenClawConfigPort` 조합
  (`SkillsService`가 skills port + config port 조합)
- [ ] 모든 process 실행은 `ProcessPort`/`ProcessRunner` 경유 (spawn 단일 경계)
- [ ] `std::process::Command` 직접 사용 0, shell 사용 0회 (S1)

### commands 레이어 (Tauri IPC)

Phase 3의 `commands` 레이어 확장. frontend kebab-case ↔ Rust snake_case.

| frontend          | Rust                 | payload → result                            |
| ----------------- | -------------------- | ------------------------------------------- |
| `list-skills`     | `list_skills`        | → skill row[] (name, enabled, eligible, …)  |
| `set-skill-enabled` | `set_skill_enabled` | `{ skillName, enabled }`                    |
| `list-plugins`    | `list_plugins`       | → plugin row[] (id, enabled, …)             |
| `set-plugin-enabled` | `set_plugin_enabled` | `{ pluginId, enabled }`                     |
| `get-plugin-runtime` | `get_plugin_runtime` | `{ pluginId }` → runtime surface (tools, hooks, …) |

### IPC 계약

architecture §5 준수 (Phase 2/3와 동일 기준):

- [ ] `src/lib/tauri/`에 command name/type 1곳 정의, 중복 정의 금지
- [ ] 명시적 serde 타입만 (wire = camelCase, `serde_json::Value` 중심 임의
  계약 금지)
- [ ] `AppError` code 기반 frontend 메시지 매핑

### 신규 stable error code

- [ ] `skill-name-invalid` (입력 형식 검증 실패)
- [ ] `skill-not-found` (toggle 대상 skill이 목록에 없음)
- [ ] `plugin-id-invalid` (입력 형식 검증 실패)
- [ ] `openclaw-skills-read-failed` (skills list/info read/parse 실패)
- [ ] `openclaw-plugins-read-failed` (plugins list/inspect read/parse 실패)
- [ ] `openclaw-plugin-toggle-failed` (plugins enable/disable nonzero)

기존 재사용:

- [ ] `openclaw-not-found`, `process-failed`, `process-timeout`
- [ ] `openclaw-config-write-failed`, `openclaw-config-invalid` (skill toggle)

### command safety

- [ ] shell string 조합 0건 (S1)
- [ ] user input(skill name, plugin id)은 **사전 형식 검증 후** argv/config
  path에 사용 (S2)
  - skill name: `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$` (Phase 3 entry id 패턴
    재사용 — config path에 들어가므로 `/`, `:`, whitespace, `..` 불가)
  - plugin id: `^(@[A-Za-z0-9][A-Za-z0-9._-]*/)?[A-Za-z0-9][A-Za-z0-9._-]{0,255}$`
    (npm 스타일 — argv 요소로만 사용되나 config-free path 안전성 유지.
    whitespace/leading dot/traversal 불가)
- [ ] CLI 호출 timeout: list/read/toggle 30초, `plugins inspect --runtime`
  60초 (module load 때문에 긴 — ProcessRunner per-request timeout)
- [ ] JSON payload는 단일 argv 요소 (skills entries write 값 `true`/`false`)

## 3. UI

Frontend는 한국어 중심.

위치:

```text
src/features/skills/
src/features/plugins/
```

### Skills

- [ ] skill 목록 (name, description(선택), 상태: 활성화/비활성화, eligible
  badge — ineligible이면 사유 안내 문구만, gating detail 편집 없음)
- [ ] skill별 토글 (활성화/비활성화) — 진행 중 중복 submit 방지
- [ ] 토글 후 목록 재조회 (optimistic update 금지 — CLI/새 세션 반영 특성)
- [ ] "변경은 새 세션부터 적용됩니다" 안내 (i18n)
- [ ] backend error를 stable code 기반으로 한국어 표시

### Plugins

- [ ] plugin 목록 (id, name(선택), format, enabled, dependencyStatus(선택))
- [ ] plugin별 토글 (활성화/비활성화) — 진행 중 중복 submit 방지
- [ ] 토글 후 목록 재조회 (optimistic update 금지)
- [ ] **실행 상태 표시**: 선택한 plugin의 `get-plugin-runtime` 결과 —
  등록된 tools/hooks/services/CLI commands 수 + 이름(목록), diagnostics
  요약 (한국어 라벨)
- [ ] runtime inspect는 **on-demand**(선택 시/버튼) — 목록 로딩 시 자동 실행
  금지 (module load 비용)
- [ ] backend error를 stable code 기반으로 한국어 표시

### i18n

- [ ] `skills`, `plugins` namespace 생성 (한국어)
- [ ] 기존 i18n architecture 유지

### Tauri frontend wrapper

- [ ] `src/lib/tauri/`에 Phase 4 command wrapper 추가 (single source 유지)
- [ ] React에서 process/executable 직접 실행 0 (invoke만 — S10)

### frontend test

- [ ] skill/plugin 토글 상태 로직 (진행 중 중복 방지, 재조회 트리거) 테스트
- [ ] error code → 한국어 메시지 매핑 테스트
- [ ] runtime inspect on-demand 트리거 로직 테스트

## 4. 테스트

기본 테스트는 fake CLI fixture만 사용한다 (S5).

### fake CLI 확장 (`fixtures/fake-openclaw/`)

Phase 1–3 fake-openclaw(샌드박스 state 파일 + argv capture) 확장:

- [ ] `skills list --json` — 샌드박스 state `skills` 섹션 기반 row 출력
  (enabled/eligible flag 포함)
- [ ] `skills info <name> --json` — 단일 skill row (unknown name → nonzero)
- [ ] `plugins list --json` — 샌드박스 state `plugins` 섹션 기반 row 출력
  (id/enabled/format/dependencyStatus)
- [ ] `plugins enable <id>` / `plugins disable <id>` — state
  `plugins.entries.<id>.enabled` 갱신 (unknown id → nonzero, state 불변)
- [ ] `plugins inspect <id> [--runtime] --json` — state 기반 runtime surface
  출력 (unknown id → nonzero)
- [ ] `config set skills.entries.<name>.enabled ...` — 기존 config set
  핸들러 재사용 (skills.*는 protected path 아님)
- [ ] argv capture 유지 (기존 패턴)

### contract 테스트 (`tests/`)

happy path:

- [ ] skills list: exact argv(`skills list --json`), state → row[] 파싱
- [ ] skill toggle: `config set skills.entries.<name>.enabled` exact argv
  (dry-run + commit 2행), state 갱신, re-list 확인
- [ ] plugins list: exact argv(`plugins list --json`), state → row[] 파싱
- [ ] plugin enable/disable: exact argv(`plugins enable <id>` /
  `plugins disable <id>` 단일 행), state 갱신, re-list 확인
- [ ] plugin runtime inspect: exact argv(`plugins inspect <id> --runtime
  --json`), surface 파싱

실패/검증:

- [ ] unknown skill toggle → `skill-not-found`, config write 0회
- [ ] invalid skill name (`../evil`, `a/b`, whitespace) → `skill-name-invalid`,
  CLI 호출 0회
- [ ] invalid plugin id (whitespace, `..`) → `plugin-id-invalid`, CLI 호출 0회
- [ ] unknown plugin id enable/disable/inspect → nonzero →
  `openclaw-plugin-toggle-failed` / `openclaw-plugins-read-failed`
- [ ] malformed JSON output → `openclaw-skills-read-failed` /
  `openclaw-plugins-read-failed`
- [ ] missing executable → `openclaw-not-found` 재사용
- [ ] timeout → `process-timeout`
- [ ] masking pipeline 통과 확인 (stdout 캡처 assert)

### unit 테스트

- [ ] skill/plugin row parser: 필수/선택 필드 fail-soft (선택 필드 부재 →
  null, row drop 0)
- [ ] skill name / plugin id 검증기 (정상/Traversal/whitespace/reject case)
- [ ] service: config write 실패 → `openclaw-config-write-failed` 매핑,
  CLI nonzero → toggle-failed 매핑

## 5. Real E2E (Phase 4 확장, opt-in)

Phase 2/3의 3중 게이트 구조(`--test real_e2e` + `--features real-e2e` +
`CLAWDESK_REAL_E2E=1`)를 유지한다.

- [ ] 기본 `cargo test`에서 real skills/plugins mutation 0회
- [ ] 조건 불만족 시 self-skip
- [ ] 조건 충족 시에만:
  - [ ] `skills list --json` / `plugins list --json` 실출력 row 스키마
    baseline 검증 (미확인 항목 확정)
  - [ ] test-owned skills entry round-trip: `skills.entries.clawdesk-e2e-<ts>
    .enabled` set → list 확인 → unset (설정만 건드리고 제거 — 기존 skill
    toggle 금지)
  - [ ] `plugins inspect <id> --runtime --json` read-only 확인
    (설치된 bundled plugin 중 하나 — state 변경 0)
- [ ] 기존 user skill/plugin state 변경 0 (test-owned entry만 생성/제거)

---

# 6. 보안 / Architecture 불변식

### ProcessRunner

- [ ] production process spawn 단일 경계 유지
- [ ] 신규 직접 spawn 없음 (skills/plugins adapter는 모두 `ProcessPort` 경유)
- [ ] executable + argv, shell command string 0

### Secret handling (S3/S7/S8)

- [ ] `skills.entries.<key>.env` / `.apiKey` surface에 **접촉 0** —
  Phase 4는 `enabled` leaf만 write (secret injection은 다른 phase/계약 영역)
- [ ] CLI stdout/stderr masking chokepoint 재사용
- [ ] error serialization에 secret 0

### Fail-closed

- [ ] skill name / plugin id 검증 실패 → CLI 호출 0회
- [ ] skill toggle dry-run `ok: false` → write 0회
- [ ] plugins enable/disable nonzero → optimistic state 0 (재조회 표시)
- [ ] runtime inspect 실패 → "실행 상태 미확인" 표시 (loaded로 추정 0)

### user environment

- [ ] `openclaw.json` 직접 파일 편집 0 (CLI 경유만)
- [ ] skill/plugin **install/update/uninstall 0** (ClawHub/npm/git/local 모두)
- [ ] `plugins.allow` / `plugins.deny` / `plugins.enabled` / `plugins.slots` /
  `plugins.load.paths` 편집 0
- [ ] `skills.load.*`, `skills.entries.<key>.env/.apiKey/.config`,
  `agents.*.skills`(allowlist) 편집 0
- [ ] Gateway process start/stop/restart 없음
- [ ] ClawDesk 소유 경로(%APPDATA%\ClawDesk\) 외 파일 작성 0

### layering

- [ ] commands → application → domain → infrastructure 일방 의존 유지
- [ ] React는 invoke만 (process/executable 접근 0)

---

# 7. 검증 명령

Phase 4 종료 전 아래 명령을 실제 실행한다.

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

- **Skill/plugin install/update/uninstall** (ClawHub, npm, git, local,
  `skills update --all`, `plugins update --all`) — 제품 계약 1.0 범위 외
- **ClawHub search/verify**, **Skill Workshop**, **skill curator**
- **skill `env`/`apiKey`/`config` 관리** (secret injection surface —
  S7-compatible secret flow가 필요한 별도 계약)
- **`skills.load.*` (extraDirs, allowSymlinkTargets, watch) 편집**
- **agent allowlist(`agents.defaults.skills`, `agents.entries.*.skills`) 편집**
- **skillKey override 지원** (name = key 가정)
- **plugin `config` field 관리** (`plugins.entries.<id>.config`)
- **`plugins.allow`/`plugins.deny`/`plugins.enabled`/`plugins.slots`/
  `plugins.load.paths` 편집**
- **plugin dependency install/repair** (`doctor --fix`, npm work)
- **Gateway restart/lifecycle** (enable/disable은 restart 불필요)
- **Node-hosted skills / remote node**
- **ClawDesk 자체 config backup/rollback**
- **실제 OpenClaw mutation을 기본 테스트에서** (S5)
- **Phase 5 (Tools / Security) 시작**

---

# Phase 종료 보고

`ClawDesk-build` 완료 보고는 다음을 포함한다.

1. 생성/변경 파일
2. 구현 내용
3. PHASE_04.md Exit Criteria 대응표
4. 실행한 검증 명령
5. 실제 검증 결과 (passed / failed / ignored/skipped)
6. 추가 dependency와 이유
7. 실제 OpenClaw 또는 시스템에 실행한 명령
8. 기존 ProcessRunner / security boundary 유지 확인
9. 미검증 사항
10. Phase 4 Non-Goals 미구현 확인
11. Phase 5 미시작 확인

real E2E를 실행하지 않았다면 반드시:

```text
real E2E: NOT-RUN
```

으로 기록한다.

실행하지 않은 검증을 PASS라고 쓰지 않는다.

Phase 4 완료 후 Phase 5로 넘어가지 않는다.
