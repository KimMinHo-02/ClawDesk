# Phase 3 — Model / Provider / API / Reasoning

Status: completed (2026-08-21, review revision: PHASE_03_1.md)

## 목표

ClawDesk에서 OpenClaw의 **모델·provider 추가·편집·삭제, API key 등록, capability 기반
reasoning/thinking effort 설정**을 terminal 없이(한국어 UI) 관리한다.

- 모든 OpenClaw config mutation은 `openclaw config` CLI를 **structured argv**로 호출하는
  경유만 사용. ClawDesk가 `openclaw.json` 파일을 직접 편집하지 않는다.
- API key는 **ClawDesk OS secret store(DPAPI)에만 plaintext 저장**. OpenClaw에는
  exec SecretRef(resolver binary)로만 공급한다.
- reasoning effort는 모델 capability(`reasoning` flag, `compat.supportedReasoningEfforts`)
  기반 UI 옵션 + `agents.defaults.thinkingDefault` 저장.

설계 근거 (2026-08-21 접속 기준, `docs.openclaw.ai` live docs, `doc-schema-version: 1`):

- `https://docs.openclaw.ai/gateway/configuration`
- `https://docs.openclaw.ai/gateway/configuration-reference`
- `https://docs.openclaw.ai/gateway/config-tools`
- `https://docs.openclaw.ai/gateway/config-agents`
- `https://docs.openclaw.ai/gateway/secrets`
- `https://docs.openclaw.ai/reference/secretref-credential-surface`
- `https://docs.openclaw.ai/tools/thinking`
- `https://docs.openclaw.ai/cli/config`
- `https://docs.openclaw.ai/cli/models`
- `https://docs.openclaw.ai/cli/secrets`
- `https://docs.openclaw.ai/help/environment`

핵심 사실 (상기 docs 기준):

- config 파일: `~/.openclaw/openclaw.json` (JSON5). ClawDesk는 파일 경로 자체를
  hardcode하지 않고 `openclaw config file --json`(`{"path": "..."}`)으로 항상 동적 해석한다.
- provider: `models.providers.<id>` map. entry 필드: `baseUrl`, `apiKey`,
  `api`(request adapter), `auth`, `maxTokens`, `timeoutSeconds`, `models[]`.
- model entry 필드: `id`(필수), `name`, `reasoning`(bool, 기본 `false`),
  `input`(기본 `["text"]`), `cost`(기본 0), `contextWindow`, `contextTokens`,
  `maxTokens`(기본 8192), `compat`(`supportsReasoningEffort`,
  `supportedReasoningEfforts`, `reasoningEffortMap`, `thinkingFormat`).
- `models.providers`, `models.providers.<id>`, `models.providers.<id>.models`는
  **protected path**: 기존 entry를 제거하는 replacement는 `--replace` 명시,
  추가는 `--merge`. `config set`은 commit 전 전체 config를 schema 검증하고,
  실패 시 활성 config 불변 + `<path>.rejected.<timestamp>` 저장.
- 안전한 비인터랙티브 write: `openclaw config set <path> '<json>' --strict-json
  [--merge|--replace]` + 쓰기 전 `--dry-run --json`(structured `ok`/`errors[]`).
- 삭제: `openclaw config unset <path>`(없는 target → exit 1, 파일 불변).
- 기본 모델: `openclaw models set <provider/model>` → `agents.defaults.model.primary`
  (unknown ref은 nonzero exit, config 불변).
- 모델/기본값 읽기: `openclaw models list --json`(read-only, `models.json` rewrite 없음),
  `openclaw config get <path> --json`(**redacted snapshot** — secret 절대 미출력).
- API key 소비: OpenClaw에 OS secret store(DPAPI 등) 연동은 **공식 docs에 없음**.
  - config `apiKey` plaintext → S7 위반, 금지.
  - `~/.openclaw/.env` → 평문 파일, 금지.
  - `openclaw secrets store` → 평문 SQLite("not encrypted at rest"), 금지.
  - `openclaw models auth paste-api-key` → 평문 auth SQLite, 금지.
  - **exec SecretRef**(`{ source: "exec", provider: "<alias>", id: "<key>" }`)가
    유일한 S7-compatible 경로: OpenClaw가 선언된 absolute binary를 shell 없이 실행해
    키를 on-demand 조회.
- exec provider declaration: `secrets.providers.<alias>`(`source: "exec"`,
  `command`(absolute, regular file, symlink 불가), `args`, `timeoutMs`(기본 5000),
  `passEnv`, `trustedDirs`, `jsonOnly`(기본 true) 등). alias 패턴
  `^[a-z][a-z0-9_-]{0,63}$`. Windows에서 ACL 검증 불가 시 **fail-closed**.
- exec protocol v1: stdin `{"protocolVersion":1,"provider":"<alias>","ids":[...]}` →
  stdout `{"protocolVersion":1,"values":{...}}`(+선택 `errors` map).
- SecretRef 지원 surface에 `models.providers.*.apiKey` 명시 등재.
- thinking level 표준 사다리: `off | minimal | low | medium | high | xhigh | adaptive
  | max | ultra`. 글로벌 기본값 config: `agents.defaults.thinkingDefault`.
  resolution order: inline directive → session override → per-agent default →
  `agents.defaults.thinkingDefault` → fallback(reasoning 모델: `medium` 또는 가장 가까운
  non-`off`, non-reasoning: `off`). 저장된 unsupported level은 OpenClaw가 자동 remap.
- gateway hot reload: `models`, `agents` 카테고리는 **restart 불필요**(hot-apply).
  Phase 3가 변경하는 모든 path는 이 카테고리에 속한다.
- `openclaw config schema`가 필드 ground truth("Code truth beats this page").

미확인 항목 (구현 시 live schema로 확정, 추측 금지):

- Windows `command` 경로 형식(반슬래시 허용 여부) — docs 미확정.
  구현 시 `openclaw config schema` 실출력 + real E2E로 검증.
- `models list --json`의 완전한 row 스키마 — docs에 전체 미게재.
  구현 시 fixture는 live docs/실출력 기준 latest stable 형태를 모방.

---

# Exit Criteria

## 1. Model / Provider CRUD (Rust, service → port → adapter)

### config path 해석

- [ ] `openclaw config file --json`으로 활성 config path 동적 해석
- [ ] `~/.openclaw/openclaw.json` hardcode 없음
- [ ] path 미해석 시 stable structured error

### 읽기

- [ ] provider 목록: `openclaw config get models.providers --json` (redacted)
- [ ] 모델 목록: `openclaw models list --json` (read-only)
- [ ] reasoning 기본값: `openclaw config get agents.defaults.thinkingDefault --json`
- [ ] 기본 모델: `openclaw config get agents.defaults.model --json`
- [ ] CLI 실패/미설치 시 stable structured error (Phase 1 error code 재사용)
- [ ] 모든 stdout/stderr masking pipeline 경유 (기존 chokepoint 유지)

### provider 추가

- [ ] 입력 검증 (id, baseUrl, api, model entry들 — §2 command safety 참조)
- [ ] `--dry-run --json` 우선 실행, `ok: true` 확인
- [ ] `openclaw config set models.providers.<id> '<json>' --strict-json --merge`
- [ ] JSON payload는 **단일 argv 요소** (shell string 조합 0)
- [ ] `ok: false` 시 쓰기 0회 + stable error
- [ ] API key는 provider JSON에 **포함하지 않음** (SecretRef는 key 등록 시에만 — §1 API key)

### provider 수정 (update)

- [ ] 필드별 subpath write (Phase 3.1에서 공식화): UI payload(UI 편집 가능
      전체 필드)를 `models.providers.<id>.baseUrl` / `.api`(Plain)와
      `models.providers.<id>.models`(models[] 전체 배열, `--replace`)에 필드별로
      쓰고, provider entry 자체는 재작성하지 않는다.
      근거: redacted read에는 실제 apiKey가 없으므로 whole-entry `--replace`는
      등록된 SecretRef를 삭제한다(S7 충돌). 계약 의도(기존 모델·필드 보존,
      다른 provider 무영향)는 더 강하게 충족한다(UI 편집 불가 필드
      `auth`/`maxTokens`/`timeoutSeconds` 포함 보존).
- [ ] write마다 `--dry-run --json` 우선
- [ ] 다른 provider entry에는 영향 없음 (path는 `<id>` 범위)

### 모델 추가/수정/삭제

- [ ] 모델 추가/수정: provider 수정과 동일한 `--replace` 경로 (models[] 전체)
- [ ] 모델 삭제: `models.providers.<id>.models`에서 해당 entry 제거한 배열을
      `--strict-json --replace`로 쓰기 (read-modify-write)
- [ ] 마지막 모델 삭제 시 빈 배열 허용
- [ ] 삭제 target 존재 확인 후 쓰기 (없는 target → error, 쓰기 0회)

### provider 삭제

- [ ] `openclaw config unset models.providers.<id>`
- [ ] 삭제 전 `config get`으로 존재 확인
- [ ] 함께 등록된 API key도 삭제 (DPAPI + config SecretRef 제거 — §1 API key)

### 기본 모델

- [ ] `openclaw models set <provider/model>`
- [ ] model ref 형식 `provider/model` (user input 2개 조합, 각각 사전 검증)
- [ ] unknown ref → nonzero exit → stable error, config 불변 확인

### API key (secret store + exec provider)

ClawDesk가 관리하는 key lifecycle:

```text
set-provider-api-key:
  1. DPAPI에 key 저장 (ClawDesk SecretStore, key id = providers/<providerId>/apiKey)
  2. secrets.providers.clawdesk exec provider declaration 보장 (idempotent,
     resolver binary 경로 변경 시 갱신)
  3. models.providers.<id>.apiKey =
     { source: "exec", provider: "clawdesk", id: "providers/<providerId>/apiKey" }
     (--ref-provider clawdesk --ref-source exec --ref-id ... 또는 --strict-json SecretRef)
  4. dry-run → write 순서 (다른 write와 동일)

delete-provider-api-key:
  1. models.providers.<id>.apiKey unset (provider JSON에서 apiKey 제거 후 --replace)
  2. DPAPI key 삭제
  3. 더 이상 key가 등록된 provider가 없으면 secrets.providers.clawdesk declaration 제거
```

- [ ] key plaintext는 config 파일·`.env`·OpenClaw store 어디에도 기록 0
- [ ] `secrets store` / `models auth paste-api-key` / `apiKey` plaintext 사용 0
- [ ] exec provider alias: `clawdesk` (패턴 `^[a-z][a-z0-9_-]{0,63}$` 준수)
- [ ] resolver `command`: absolute 경로, regular file, symlink 아님
- [ ] Windows ACL fail-closed 상황을 stable error로 매핑 (실패 시 key가 사용 가능한
      것처럼 UI에 표현하지 않음)
- [ ] `--allow-exec`는 제품 flow에서 **사용하지 않음** (exec reference resolvability
      체크는 dry-run에서 기본 skip — schema 검증만 수행)
- [ ] key 등록 상태 조회는 ClawDesk SecretStore index 기준 (redacted config read 보조)
- [ ] key 값은 argv에도 기록 0: `set-provider-api-key` IPC argument는 SecretStore
      write와 resolver reference 설정에만 사용되고, 로그/에러/UI에 mask

### reasoning / thinking effort

- [ ] thinking level enum: `off | minimal | low | medium | high | xhigh | adaptive | max | ultra`
- [ ] `set`: `openclaw config set agents.defaults.thinkingDefault '<level>'`
      (dry-run → write, enum 검증 실패 시 stable error, 쓰기 0회)
- [ ] `get`: `config get agents.defaults.thinkingDefault --json` (미설정 → null)
- [ ] 모델별 capability: model entry `reasoning` flag + `compat.supportsReasoningEffort`
      / `compat.supportedReasoningEfforts` 해석
- [ ] capability 필드 부재 → `reasoning: false`로 처리 (fail-closed)
- [ ] reasoning 미지원 모델 → effort 옵션 UI disabled
- [ ] `supportedReasoningEfforts` 존재 → UI 옵션을 해당 집합(기본값 포함)으로 제한
- [ ] `supportedReasoningEfforts` 부재 + reasoning 지원 → 표준 사다리 전체 노출
      (OpenClaw가 unsupported level을 자동 remap하는 공식 동작에 의존)
- [ ] Phase 3에서 per-model effort 저장하지 않음 (글로벌 default만 — Phase 9 영역)

### 멱등성 / write safety

- [ ] 모든 write는 `--dry-run --json` → `ok` 확인 → 실write 2단계
- [ ] `ok: false` → stable error + config 불변 (retry 가능)
- [ ] 동일 상태 재등록 (exec provider declaration, SecretRef) → 의미 변경 0
- [ ] CLI가 atomic write + rejected payload side-file을 보장하는 구조를 재사용
      (ClawDesk 자체 backup/rollback 구현 없음)

### mutation boundary

- [ ] 기본 `cargo test`에서 real OpenClaw config 변경 0회
- [ ] fake CLI fixture 기반 테스트만 (real binary detect 외 mutation 금지, S5)
- [ ] real config CRUD는 dedicated real E2E(opt-in)에서만

---

## 2. 인터페이스

### OpenClawConfigPort / OpenClawConfigAdapter

- [ ] `domain/ports`에 `OpenClawConfigPort` trait 추가
- [ ] `infrastructure/openclaw`에 `OpenClawConfigAdapter` 구현
- [ ] 모든 process 실행은 기존 `ProcessPort`/`ProcessRunner` 경유 (spawn 단일 경계 유지)
- [ ] Phase 1의 OpenClaw executable resolution 재사용
- [ ] `std::process::Command` 직접 사용, shell 사용 0회

### SecretStorePort / SecretStore

ARCHITECTURE §3의 기존 port 정의 기반:

- [ ] `set(key_id, value)`, `get(key_id) -> Option<value>`, `delete(key_id)`,
      `contains(key_id)`, `list_key_ids() -> Vec<key_id>`
- [ ] `infrastructure/secrets`에 `SecretStore` 구현:
  - 값: **DPAPI**로 암호화한 blob 파일
  - index: 비secret JSON (key id, 저장/수정 시각만 — 값 0 포함)
  - 위치: `%APPDATA%\ClawDesk\secrets\` (S4 명시 경로)
- [ ] DPAPI 실패 → stable structured error (fail-closed)
- [ ] DPAPI 계층은 trait 뒤에 숨겨 unit test에서 fake 교체 가능

### clawdesk-secret-resolver (신규 bin)

- [ ] `src-tauri` workspace에 별도 bin target
- [ ] exec protocol v1 구현:
  - stdin: `{"protocolVersion":1,"provider":"clawdesk","ids":[...]}`
  - stdout: `{"protocolVersion":1,"values":{...}}` (+부존재 id는 `errors` map,
    `code: "NOT_FOUND"`)
- [ ] DPAPI lookup은 ClawDesk SecretStore와 동일한 저장소(index + blob) 사용
- [ ] shell 사용 0, argv/환경변수에서 secret 수신 0 (stdin JSON만)
- [ ] stdout/stderr/error에 secret 값 포함 0 (에러는 code만)
- [ ] protocol version 불일치/JSON parse 실패 → nonzero exit, secret 미출력

### ModelService / ApiKeyService (application layer)

- [ ] `ModelService`: list/get/save/delete provider·model, default model,
      reasoning default get/set
- [ ] `ApiKeyService`: set/delete/list provider API key
      (SecretStore + config write orchestration)
- [ ] structured result + stable `AppError` mapping
- [ ] infra detail(path, CLI 출력 원문 등) frontend 노출 금지

### commands 레이어 (Tauri IPC)

Phase 2의 `commands` 레이어 확장. frontend kebab-case ↔ Rust snake_case.

| frontend | Rust | payload → result |
| --- | --- | --- |
| `list-providers` | `list_providers` | → provider 요약[] (id, baseUrl, api, model 수, apiKeyRegistered) |
| `get-provider` | `get_provider` | `{ providerId }` → 전체 provider (redacted) + models[] |
| `save-provider` | `save_provider` | `{ provider }` → upsert (신규=merge, 기존=replace) |
| `delete-provider` | `delete_provider` | `{ providerId }` |
| `list-models` | `list_models` | → 모델 row[] (provider, id, name, reasoning, context 등) |
| `get-default-model` | `get_default_model` | → `modelRef \| null` |
| `set-default-model` | `set_default_model` | `{ modelRef }` |
| `get-reasoning-default` | `get_reasoning_default` | → level \| null |
| `set-reasoning-default` | `set_reasoning_default` | `{ level }` |
| `set-provider-api-key` | `set_provider_api_key` | `{ providerId, apiKey }` |
| `delete-provider-api-key` | `delete_provider_api_key` | `{ providerId }` |
| `list-api-keys` | `list_api_keys` | → `{ providerId, registered }`[] |

### IPC 계약

architecture §5 준수 (Phase 2와 동일 기준):

- [ ] `src/lib/tauri/`에 command name/type 1곳 정의, 중복 정의 금지
- [ ] 명시적 serde 타입만 (`serde_json::Value` 중심 임의 계약 금지)
- [ ] `AppError` code 기반 frontend 메시지 매핑
- [ ] `apiKey` 필드는 응답에 **절대 포함하지 않음** (registered bool만)

### 신규 stable error code

- [ ] `provider-id-invalid`
- [ ] `model-id-invalid`
- [ ] `thinking-level-invalid`
- [ ] `openclaw-config-read-failed`
- [ ] `openclaw-config-write-failed`
- [ ] `openclaw-config-invalid` (dry-run/validate `ok: false`)
- [ ] `secret-store-unavailable` (DPAPI 읽기/쓰기 실패)
- [ ] `secret-ref-registration-failed` (exec provider declaration/SecretRef write 실패)

기존 재사용:

- [ ] `openclaw-not-found`, `process-failed`, `process-timeout`

### command safety

- [ ] shell string 조합 0건 (S1)
- [ ] user input(provider id, model id, baseUrl, model ref)은
      **사전 형식 검증 후** argv/config path에 사용 (S2)
  - provider id / model id: `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`
    (`/`, `:`, whitespace, `..` 불가 — config path traversal 차단)
  - baseUrl: `http://` 또는 `https://` 절대 URL
  - thinking level: enum 검증
- [ ] JSON payload는 단일 argv 요소로 전달 (내부 문자열 보간 없음)
- [ ] CLI 호출 timeout: 30초 (Phase 1 ProcessRunner timeout 재사용)

---

## 3. UI

Frontend는 한국어 중심.

위치:

```text
src/features/models/
```

### Provider / 모델 목록

- [ ] provider 목록 (id, baseUrl, api, 모델 수, API key 등록 상태)
- [ ] provider 상세 (모델 목록, 모델별 reasoning badge)
- [ ] 전체 모델 목록 (`list-models`)
- [ ] 기본 모델 표시 + 변경 select

### Provider / 모델 편집

- [ ] provider form: id (신규만 편집 가능), baseUrl, api type select
- [ ] 모델 form: id (필수), name, reasoning checkbox, input modalities
      (text/image), contextWindow, maxTokens,
      supportsReasoningEffort checkbox, supportedReasoningEfforts multi-select
      (reasoning + supportsReasoningEffort일 때만 노출)
- [ ] 모델 추가/수정/삭제 (provider 편집 내)
- [ ] 입력 검증 (frontend) + Rust 재검증 (backend) — backend error를
      stable code 기반으로 한국어 표시
- [ ] 진행 중 중복 submit 방지

### API key

- [ ] password input (register/change) — 입력 즉시 mask, 저장 후 재표시 0
- [ ] 등록 상태 표시 (registered / unregistered) — `list-api-keys` 기준
- [ ] 삭제 (확인 다이얼로그)
- [ ] key 값을 UI 어디에라도 재노출하지 않음 (S3)

### Reasoning effort

- [ ] 글로벌 thinking default selector (9단계, 한국어 라벨)
- [ ] 현재 기본 모델이 reasoning 미지원 → 전체 disabled + 안내 문구
- [ ] `supportedReasoningEfforts` 존재 시 옵션 제한
- [ ] 현재 값 표시 (`get-reasoning-default`, null → "기본값 없음" 안내)

### i18n

- [ ] `models` namespace 생성 (한국어)
- [ ] 기존 i18n architecture 유지

### Tauri frontend wrapper

- [ ] `src/lib/tauri/`에 Phase 3 command wrapper 추가 (single source 유지)

### frontend test

- [ ] reasoning 옵션 enable/disable 상태 로직 테스트
- [ ] supportedReasoningEfforts 제한 로직 테스트
- [ ] error code → 한국어 메시지 매핑 테스트
- [ ] provider/model form 검증 로직 테스트

---

## 4. 테스트

기본 테스트는 fake CLI fixture만 사용한다 (S5).

### fake CLI 확장 (`tests/fixtures/openclaw/`)

Phase 1/2 fake-openclaw에 config 상태 시뮬레이션 추가:

- [ ] 샌드박스 config state 파일 (테스트 임시 디렉터리) 읽기/쓰기
- [ ] `config file --json`
- [ ] `config get <path> --json` (secret은 redacted 출력)
- [ ] `config set <path> <json> --strict-json [--merge|--replace]`
      (+ `--dry-run --json`: state 변경 없이 `ok`/`errors[]` 반환)
- [ ] `config unset <path>` (없는 target → exit 1, state 불변)
- [ ] `models list --json` (config state 기반 row)
- [ ] `models set <provider/model>` (unknown ref → nonzero, state 불변)
- [ ] protected path 규칙 모방: `--replace` 없는 entry 제거 → 거부
- [ ] argv capture (Phase 2 fake-npm 패턴)

### contract 테스트 (`tests/`)

happy path:

- [ ] provider 추가: exact argv 검증 (`--strict-json`, `--merge`, 단일 argv JSON),
      state 갱신 확인
- [ ] provider 수정: `--replace`, 다른 provider 보존 확인
- [ ] 모델 추가/수정/삭제: models[] read-modify-write, 배열 불변성 확인
- [ ] provider 삭제: `unset` + API key 연동 삭제 확인
- [ ] 기본 모델 설정: exact argv (`models set <ref>`)
- [ ] thinking default set/get

API key:

- [ ] key 등록 시 argv에 key 값 0 포함 (capture 검증)
- [ ] `secrets.providers.clawdesk` declaration write (idempotent — 2회 호출 시
      2회째 의미 write 0)
- [ ] `models.providers.<id>.apiKey` = SecretRef object (state 검증, plaintext 0)
- [ ] DPAPI fake에 key 저장 확인, index에 값 0 포함
- [ ] key 삭제: SecretRef 제거 + DPAPI fake delete
- [ ] 마지막 key 삭제 시 declaration 제거
- [ ] resolver protocol: stdin/stdout round-trip, NOT_FOUND errors map,
      protocol version mismatch → nonzero, stdout에 secret 0 (로그 캡처 검증)

실패/검증:

- [ ] dry-run `ok: false` → 실write 0 (state 불변) + `openclaw-config-invalid`
- [ ] invalid id (`../evil`, `a/b`, `a:b`, whitespace) → `provider-id-invalid` /
      `model-id-invalid`, CLI 호출 0회
- [ ] invalid thinking level → `thinking-level-invalid`, CLI 호출 0회
- [ ] unknown model ref → stable error, state 불변
- [ ] config read 실패(missing openclaw) → `openclaw-not-found` 재사용
- [ ] timeout → `process-timeout`
- [ ] DPAPI fake 실패 → `secret-store-unavailable`, config write 0회

masking:

- [ ] config get redacted output + masking pipeline 통과 확인
- [ ] error message에 key 값 0 포함 (not-contains assert)
- [ ] AppError serialization에 secret 0 포함

### unit 테스트

- [ ] SecretStore (DPAPI fake): set/get/delete/contains/list, index 정합성,
      값 불변성 (index에 값 없음)
- [ ] capability 해석: `reasoning` 부재 → false, `supportedReasoningEfforts` 제한
- [ ] thinking level enum parse/serialize
- [ ] id/baseUrl/model ref 검증기

---

## 5. Real E2E (Phase 3 확장, opt-in)

Phase 2의 3중 게이트 구조(`--test real_e2e` + `--features real-e2e` +
`CLAWDESK_REAL_E2E=1`)를 유지한다.

- [ ] 기본 `cargo test`에서 real config mutation 0회
- [ ] 조건 불만족 시 self-skip
- [ ] 조건 충족 시에만:
  - [ ] `openclaw config schema` 실출력으로 필드 baseline 검증
        (특히 Windows `command` 경로 형식 — §설계 근거 미확인 항목)
  - [ ] `list-providers` → `save-provider` → `delete-provider` round-trip
        (test-owned provider id: `clawdesk-e2e-<timestamp>`)
  - [ ] test-owned provider cleanup (실행 전 상태 기록, test가 만든 항목만 제거)
- [ ] 기존 user provider/model/key는 절대 변경/삭제 0 (ownership 확인)

---

# 6. 보안 / Architecture 불변식

### ProcessRunner

- [ ] production process spawn 단일 경계 유지
- [ ] 신규 직접 spawn 없음 (resolver bin은 OpenClaw가 실행하는 대상 —
      ClawDesk spawn 아님. stdin/stdout protocol만 사용)
- [ ] executable + argv, shell command string 0

### Secret handling (S3/S7/S8)

- [ ] key plaintext 저장 0: DPAPI blob만
- [ ] config/`.env`/OpenClaw store/로그/UI/테스트 출력에 key plaintext 0
- [ ] `config get` redacted snapshot + 기존 masking chokepoint 재사용
- [ ] error serialization에 secret 0 (mask pipeline 경유)
- [ ] resolver 출력에 secret 0 (value map만, 에러는 code만)

### Fail-closed

- [ ] id/baseUrl/level 검증 실패 → CLI 호출 0회
- [ ] dry-run 실패 → write 0회
- [ ] DPAPI 실패 → key 등록/공급 0
- [ ] resolver 부재/실패 → OpenClaw가 key를 못 얻음 (OpenClaw 측 unresolved) —
      ClawDesk는 "key 사용 가능"으로 UI 표현 0
- [ ] capability 필드 부재 → reasoning 미지원 처리

### user environment

- [ ] `openclaw.json` 직접 파일 편집 0 (CLI 경유만)
- [ ] `~/.openclaw/.env` 작성 0
- [ ] OpenClaw store(`secrets store`) 사용 0
- [ ] gateway process start/stop 없음
- [ ] PATH/시스템 설정 변경 없음
- [ ] ClawDesk 소유 경로(%APPDATA%\ClawDesk\) 외 파일 작성 0

### layering

- [ ] commands → application → domain → infrastructure 일방 의존 유지
- [ ] React는 invoke만 (process/executable 접근 0)

---

# 7. 검증 명령

Phase 3 종료 전 아래 명령을 실제 실행한다.

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

- **Channel credentials** (Discord/Telegram — Phase 6)
- **OAuth flow** (`auth: oauth`, `models auth login`) — API key만
- **Gateway lifecycle** (start/stop/install, onboarding, daemon)
- **OpenClaw update 실행** (Phase 8/10)
- **Uninstall**
- **Per-model effort 저장/세션별 effort** (Phase 9 Integrated Chat 영역)
- **`cost` 필드 관리** (기본 0 유지)
- **`agents.entries`(per-agent) 편집** — `agents.defaults`만
- **`$include`, named profile(`OPENCLAW_PROFILE`) 지원**
- **`models refresh` catalog 관리**
- **ClawDesk 자체 config backup/rollback** (OpenClaw atomic write/rejected payload
      구조 신뢰)
- **`--allow-exec`를 쓰는 검증 flow**
- **실제 OpenClaw mutation을 기본 테스트에서** (S5)

---

# Phase 종료 보고

`ClawDesk-build` 완료 보고는 다음을 포함한다.

1. 생성/변경 파일
2. 구현 내용
3. PHASE_03.md Exit Criteria 대응표
4. 실행한 검증 명령
5. 실제 검증 결과 (passed / failed / ignored/skipped)
6. 추가 dependency와 이유 (예: DPAPI binding)
7. 실제 OpenClaw 또는 시스템에 실행한 명령
8. 기존 ProcessRunner / security boundary 유지 확인
9. 미검증 사항
10. Phase 3 Non-Goals 미구현 확인
11. Phase 4 미시작 확인

real E2E를 실행하지 않았다면 반드시:

```text
real E2E: NOT-RUN
```

으로 기록한다.

실행하지 않은 검증을 PASS라고 쓰지 않는다.

Phase 3 완료 후 Phase 4로 넘어가지 않는다.
