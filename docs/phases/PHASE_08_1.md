# Phase 8.1 — Node.js Update (unsupported 버전 one-shot 업데이트)

Status: completed (2026-08-26)

## 목표

PRODUCT_CONTRACT §3 (terminal 미노출)에 따라, **Node.js가 지원 범위 밖 버전으로 detect된 경우** 터미널 없이(한국어 UI)
winget 경유 **one-shot Node.js 업데이트**를 제공하고, 성공 시 재-detect로 즉시 반영한다.

- 현재 동작(Phase 1/2): `node --version` probe(`system.rs:61-85`) → `NodeDetection::Found { version }`,
  지원 범위 판정 `node_version_supported`(`installer.rs:221-232`: **22.22.3+, 24.15+, 25.9+, 26+**, Node 23 제외).
  install flow는 unsupported 시 `unsupported-node-version` structured error + 0 npm spawn(`services/install.rs:82-84`),
  Setup UI는 안내 문구만 표시(`SetupFeature.tsx:135`).
- Phase 8.1: 이 dead-end를 **UI 버튼 → IPC → winget install(structured argv) → 재-detect** 흐름으로 대체한다.
- **범위 해제 선언**: PHASE_02.md Non-Goals의 "Node 자동 설치" 항목 중 **unsupported 버전 업데이트(winget) 케이스만**
  본 계약으로 명시적으로 해제한다. **Node 부재(NotFound) 케이스는 PHASE_02 계약 그대로** (structured error + 안내, 자동 설치 0).
- 모든 프로세스 실행은 `ProcessRunner` 단일 spawn boundary(structured `executable + argv`, timeout) 경유 (S1/S2).
- IPC naming은 Phase 7.5 계약 준수: frontend kebab-case + Rust `#[tauri::command(rename = "...")]`.
- real winget 실행(real OS mutation)은 **real app에서 사용자의 명시적 버튼 클릭**과 explicit real E2E(opt-in)에서만.
  기본 `cargo test`/`pnpm test`에서는 fake fixture만 (S5).

## 설계 근거

- winget: Windows 10 1809+/11 기본 패키지 매니저. `winget.exe + argv` structured spawn으로 S1/S2를 그대로 만족.
  앱이 직접 HTTP/설치 파일을 다루지 않음(네트워크는 winget 본인이 담당 — Phase 2 npm registry 연결과 동일 범주).
- `winget install`(upsert)을 `winget upgrade` 대신 사용: 기존 Node가 winget 패키지로 등록되어 있든
  (수동 설치 등) 없든 동일하게 설치/갱신되어 결정이 일관된다.
- 패키지 ID **`OpenJS.NodeJS.LTS`** pin: LTS 채널은 지원 범위(22.22.3+/24.15+/…)에 항상 들어옴.
  백스톱: 업데이트 후 **재-detect가 지원 범위에 들어오지 않으면 `node-update-failed`** (fail-closed, winget exit code만 신뢰하지 않음).
- **재-detect 경로 문제**: MSI 설치 후에도 실행 중인 앱 프로세스의 PATH env block은 stale(자식 spawn은 부모 env 상속).
  따라서 업데이트 후 detect는 후보를 **① 표준 MSI 경로 `C:\Program Files\nodejs\node.exe`(테스트 주입 가능) → ② PATH resolve** 순으로 probe.
  앱 재시작 없이 새 버전을 확인한다. (기존 detect 행위 — PATH probe — 는 변경 0)
- timeout: winget install **900s** (Phase 2 `INSTALL_TIMEOUT = 15 * 60`(`installer.rs:22`)과 동일 예산),
  probe류(`winget --version`, `node --version`) 10s (`adapter.rs:17` `VERSION_TIMEOUT` 패턴).

## 핵심 사실

### 트리거와 preconditions (service 순서, fail-closed)

1. `detect_node()` → `NotFound` ⇒ `node-not-found` (0 winget) — 8.1 범위 밖, Phase 2 유지
2. `Found { version }` + `node_version_supported(version) == true` ⇒ `node-update-not-needed` (0 winget)
3. `Found { version }` + unsupported ⇒ winget update 진행

### winget 호출 (adapter)

- detect: `winget --version` (10s) — `NotFound` ⇒ `winget-not-found`
- install exact argv (byte-match contract 대상, user input 0):
  ```
  ["install", "--id", "OpenJS.NodeJS.LTS", "--exact", "--silent",
   "--disable-interactivity", "--accept-source-agreements", "--accept-package-agreements"]
  ```
  (900s) — non-zero ⇒ `node-update-failed`(masked stderr), timeout ⇒ `process-timeout`
- install 후 재-detect: 후보 ①② 순 probe, `parse_node_version`(`system.rs:210`) 재사용 → `NodeDetection` 반환
- 성공 판정(service): 반환된 `NodeDetection`이 `Found` + `node_version_supported` 일 때만 성공.
  그렇지 않으면 `node-update-failed` (winget exit 0이라도)

### error code (신규 +3, 기존 컨벤션 준수)

- `winget-not-found` — winget executable 부재
- `node-update-failed` — winget non-zero / timeout 이후 재검증 실패 / 업데이트 후에도 unsupported
- `node-update-not-needed` — 이미 지원 버전 (값이 아닌 structured error: 불필요한 OS mutation 방지)

### IPC / wire

- `update-node` (kebab) → `update_node() -> Result<NodeDetection, AppError>`
- 반환 wire 타입은 **기존 `NodeDetection`**(`{status:"found",version}` / `{status:"not-found"}`) 재사용 — 신규 wire type 0
- IPC command 49 → **50**

## Scope (in-scope)

### Rust (`src-tauri/`)

1. `src/domain/ports/node_update.rs` (신규)
   - `NodeUpdatePort`: `update_node() -> Result<NodeDetection, AppError>`
     (winget detect → install → 재-detect를 한 번에 수행하는 adapter 단위 use case)
2. `src/infrastructure/windows/node_update.rs` (신규)
   - `NodeUpdateAdapter` (ProcessPort):
     - production wiring: winget = PATH 이름 `winget`, MSI 후보 경로 = `C:\Program Files\nodejs\node.exe`
     - 테스트 wiring: fake binary 경로 + MSI 후보 경로 주입 (`WindowsSystemAdapter::with_node_executable` 패턴과 동일 정신)
     - winget missing ⇒ `winget-not-found`; timeout/exit mapping 위 표와 동일
3. `src/application/services/node_update.rs` (신규)
   - `NodeUpdateService`: 위 preconditions 1-3 순서 검증, `node_version_supported` 재사용,
     결과 검증(Find + supported), `production()` wiring(`WindowsSystemAdapter` + `NodeUpdateAdapter`, 단일 `ProcessRunner`)
4. `src/commands/node_update.rs` (신규) — IPC command 1개:
   - `update-node` → `NodeDetection` (Phase 8 `run_blocking` 패턴 재사용)
5. `src/error.rs` — stable code +3 (위 목록)
6. `src/lib.rs` — command 1개 `generate_handler!` 등록 (49 → 50)
7. mod 등록: `domain/ports`, `infrastructure/windows`, `application/services`, `commands` (+1 each, re-export)
8. `tests/ipc_name_contract.rs` — `IPC_CONTRACT` +1 (49 → **50**), `registered_names()` +1
9. `fixtures/fake-winget/main.rs` (신규 bin `clawdesk-fake-winget`) — fake-openclaw/fake-npm 패턴 1:1:
   - `--version` → winget 버전 문자열 (probe 대상)
   - 위 exact argv → exit 0 + state file 갱신(`node.version` → supported LTS, 예: `24.15.0`)
   - behavior: `fail`(exit 1 + stderr, fake `sk-` 토큰 1줄 포함 — masking 검증용), `sleep`(timeout 시나리오)
   - unknown/missing flag(특히 `--id` 부재) ⇒ exit 2 (non-goal guard, Phase 4~8 패턴)
   - `CLAWDESK_FAKE_STATE` sandbox env (기존 fixture 동일)
10. `fixtures/fake-node/main.rs` (신규 bin `clawdesk-fake-node`):
    - `--version` → state file의 `node.version` 출력(`v` prefix 포함), state 없으면 기본 `18.19.0`(unsupported)
    - 다른 argv ⇒ exit 2
11. `tests/node_update_contract.rs` (신규) — contract test (Phase 4~8 패턴 1:1, real `ProcessRunner` + fake binary):
    - preconditions: unsupported → 진행 / supported → `node-update-not-needed` + **0 winget** (capture 부재 assert) /
      not-found → `node-not-found` + 0 winget
    - winget install exact argv byte-match (capture)
    - success: fake-winget exit 0 → MSI 후보 경로(주입) 경유 재-detect → `Found` + supported → `NodeDetection` 반환
    - success이지만 재-detect가 여전히 unsupported(state 미변경 시나리오) ⇒ `node-update-failed`
    - non-zero ⇒ `node-update-failed` (masked), sleep ⇒ `process-timeout`
    - winget executable 부재 ⇒ `winget-not-found`
    - fake-winget unknown flag reject (exit 2)
    - masking: fake `sk-` 토큰이 masked 상태로만 노출 (`logs_fake_token_is_masked_end_to_end` 패턴)
12. `tests/real_e2e.rs` — `real_node_update_baseline` +1 (opt-in S9 gate, 기본 NOT-RUN):
    - **read-only 전용**: node detect + support 판정 + `winget --version` 가용성 probe — real winget install **0**

### Frontend (`src/`)

1. `src/features/setup/SetupFeature.tsx` — Node `unsupported` 표시 영역(`:135`)에 **"Node.js 업데이트" 버튼** 추가:
   - 클릭 → `updateNode()` → 로딩("Node.js 업데이트 중입니다...") → 성공 시 반환된 `NodeDetection`을
     report에 반영(node 줄 → supported + 새 버전, OpenClaw 설치 흐름 진행 가능) / 실패 시 한국어 에러 + 안내
   - `unsupported-node-version` install 에러 영역에서도 동일 버튼 노출
2. `src/features/setup/setupState.ts` (+ `.test.ts`) — node update 상태 로직 + 신규 error code → 한국어 메시지 매핑
3. `src/lib/tauri/index.ts` — COMMANDS +1 (`updateNode: "update-node"`), wrapper +1 (반환: 기존 `NodeDetection` wire type);
   `index.test.ts` +1 (kebab 이름 계약)
4. `src/i18n/ko/index.ts` — `setup` namespace 확장: 업데이트 버튼/진행/완료 문자열 + errors 3개

### Docs

- 이 파일(계약) — 완료 시 `Status: completed (YYYY-MM-DD)` + exit criteria `[x]`
- `ROADMAP.md` Phase 8.1 상태 갱신

## Exit Criteria

### 1. IPC command (1개)

- [x] `update-node` kebab `rename` 속성으로 등록 (Phase 7.5 계약), `ipc_name_contract` **50/50** 통과
- [x] frontend COMMANDS/wrapper/test 1쌍이 Rust side와 1:1 일치

### 2. Preconditions (fail-closed)

- [x] supported 버전 ⇒ `node-update-not-needed` + **0 winget** (contract assert)
- [x] not-found ⇒ `node-not-found` + **0 winget** (contract assert)
- [x] unsupported 버전만 winget update 진행

### 3. winget 실행

- [x] exact argv byte-match (contract test, user input 0)
- [x] winget non-zero ⇒ `node-update-failed`, timeout(900s) ⇒ `process-timeout`
- [x] winget executable 부재 ⇒ `winget-not-found`

### 4. 재-detect 검증

- [x] MSI 후보 경로 우선 → PATH fallback probe, `parse_node_version` 재사용
- [x] 재-detect가 `Found` + supported 일 때만 성공, 아니면 `node-update-failed` (winget exit 0 무관)
- [x] 반환 타입 = 기존 `NodeDetection` wire (신규 wire type 0)

### 5. UI

- [x] Setup: unsupported 버전 표시 + "Node.js 업데이트" 버튼 (로딩/성공/실패 상태, 한국어)
- [x] 성공 시 node 줄이 supported + 새 버전으로 갱신되고 OpenClaw 설치 흐름 진행 가능

### 6. 구조 유지

- [x] Phase 0~8 business logic 변경 0 (신규 파일 + `lib.rs` 등록 + `error.rs` +3 + `ipc_name_contract` +1 + mod/Cargo.toml 등록 + `real_e2e.rs` baseline 1개 외 기존 파일 로직 수정 0)
- [x] `ProcessRunner` 단일 spawn boundary 유지 (`Command::new` 유일 `runner.rs`)
- [x] shell string 0, user input shell 보간 0 (S1/S2)
- [x] winget 출력 masking pipeline 경유 확인 (fake `sk-` 토큰 masked — S3/S8)

### 7. 테스트

- [x] 신규 `node_update_contract` 전부 통과 (10/10)
- [x] 기존 Phase 0~8 regression test 전부 유지 (수치 변동은 lib 신규 unit test 14개 + ipc_name_contract 50 + 신규 기능 frontend 테스트 외 0)
- [x] real E2E read-only baseline opt-in 게이트 유지, 기본 `cargo test`에서 NOT-RUN (real winget install 0)

### 8. 검증

- [x] `cargo fmt --check`, `cargo check --all-targets`, `cargo test --all-targets`, `cargo clippy --all-targets`
- [x] `pnpm typecheck`, `pnpm lint`, `pnpm test`

## Non-Goals

- Node **부재(NotFound)** 자동 설치 — PHASE_02 계약 유지 (structured error + 안내 only)
- Chocolatey / Scoop / portable Node 다운로드 / 공식 PowerShell installer (`iwr | iex`)
- PATH mutation 자동화, 앱 자동 재시작
- npm 자동 upgrade (PHASE_02 유지: unsupported npm 11.13~11.15 = error + 안내)
- Node 버전 선택 UI (`OpenJS.NodeJS.LTS` pin 고정), Node uninstall/rollback
- Windows Update 및 기타 OS 패키지 운영

## 완료 후 수동 Smoke Test

자동 검증 완료 후 사용자가 직접 `pnpm exec tauri dev` 실행:

- **unsupported Node 머신**: Setup에서 unsupported 버전 표시 → "Node.js 업데이트" 클릭 → winget 실행 →
  node 줄이 supported + 새 버전으로 갱신 → OpenClaw 설치 흐름 진행
- **supported Node 머신**: 업데이트 시도 → `node-update-not-needed` 한국어 안내 (OS mutation 0)
- real winget 실행은 이 수동 smoke와 explicit real E2E(미포함 — read-only baseline만)에서만 발생

smoke test에서 새로 드러나는 내부 기능 오류는 Phase 8.1 완료 여부와 분리하여 실제 오류 기준으로 후속 수정한다.
