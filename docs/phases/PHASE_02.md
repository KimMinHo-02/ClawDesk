# Phase 2 — Installer (OpenClaw 설치 플로우)

Status: completed (2026-08-21)

## 목표

ClawDesk에서 OpenClaw CLI를 설치한다.

- OpenClaw이 미설치 상태이면 `openclaw@latest` stable을 npm 글로벌 설치한다.
- 이미 OpenClaw이 설치되어 있으면 update하지 않고 기존 설치 버전을 그대로 반환한다.
- UI + Rust adapter 기반으로 설치하며 terminal은 사용자에게 노출하지 않는다.
- GUI feature가 처음 들어가는 phase이며, `commands`(Tauri IPC) 레이어도 이 phase에서 만든다.
- 실제 설치(real mutation)는 explicit real E2E(opt-in) 레이어에서만 검증한다.

설계 근거 (2026-08-20 기준, 최신 stable OpenClaw `2026.7.1-2`, 공식 docs:

- `docs.openclaw.ai/install`
- `docs.openclaw.ai/install/installer`
- `docs.openclaw.ai/platforms/windows`

):

- 설치 방법: **`npm`을 구조화된 argv로 직접 호출**한다.
- 공식 PowerShell installer (`iwr | iex`)는 사용하지 않는다.
  - 비구조화 black box
  - PATH mutation
  - Node 자동 provision
  - ClawDesk S1 shell 금지 정책과 부합하지 않음
- Node 지원 범위:
  - Node 22: `22.22.3+`
  - Node 24: `24.15+`
  - Node 25: `25.9+`
  - Node 26: 지원
  - Node 23: 공식 unsupported
- npm 정책:
  - npm `<= 11.12`: `--allow-scripts` 없이 설치
  - npm `11.13 ~ 11.15`: 설치 차단, `unsupported-npm-version`
  - npm `>= 11.16`: `--allow-scripts=openclaw` 포함
  - npm 12+: `--allow-scripts=openclaw` 포함
- 설치 명령:
  - `npm install -g openclaw@latest`
- Windows에서 npm shim(`npm.cmd`)은 shell 없이 production spawn 대상으로 사용하지 않는다.
- npm 실행은 **해당 Node runtime의 `node.exe + npm-cli.js`** 구조화 argv로 수행한다.
- 설치 후 검증:
  - OpenClaw executable/package entry detect
  - `--version` parsing
  - Phase 1 기능 재사용
- OpenClaw package 실행 entry는 설치된 package의 `package.json`에서 `bin.openclaw`을 해석하고, package root 경계 검증 후 `node.exe + JS entry` 형태로 실행한다.
- Phase 2에서 기존 OpenClaw 설치를 update하지 않는다. Update 실행은 후속 phase 영역이다.

---

# Exit Criteria

## 1. 설치 플로우

Rust application service → port → adapter 경유.

### Node 사전조건

- [x] Node 존재 여부 확인
- [x] Node version parsing
- [x] 다음 지원 범위 확인
  - Node 22 `>= 22.22.3`
  - Node 24 `>= 24.15`
  - Node 25 `>= 25.9`
  - Node 26+
  - Node 23 reject
- [x] Node 부재 시 stable structured error:
  - `node-not-found`
- [x] Node unsupported 시 stable structured error:
  - `unsupported-node-version`
- [x] Node 사전조건 실패 시 npm process spawn 0회

### npm spawn entry 해석

- [x] PATH 또는 Phase 1 detection 기반으로 실제 `node.exe`를 해석
- [x] 해당 Node installation과 연결된 `npm-cli.js` 위치 해석
- [x] `npm.cmd`, `npm.ps1`, extension-less npm shim을 production process로 직접 spawn하지 않음
- [x] npm 관련 모든 process 실행은 `ProcessPort` / `ProcessRunner` 경유
- [x] shell 사용 0회

### npm version 정책

- [x] `node.exe npm-cli.js --version`으로 npm version 확인
- [x] npm version parsing
- [x] npm `<= 11.12`
  - 설치 허용
  - `--allow-scripts` 미포함
- [x] npm `11.13 ~ 11.15`
  - 설치 전 차단
  - stable error `unsupported-npm-version`
  - install spawn 0회
- [x] npm `>= 11.16`
  - `--allow-scripts=openclaw` 포함
- [x] npm 12+
  - `--allow-scripts=openclaw` 포함
- [x] ClawDesk가 npm 자체를 자동 upgrade하지 않음

### 설치

- [x] npm `<= 11.12`:

```text
node.exe npm-cli.js install -g openclaw@latest
```

- [x] npm `>= 11.16`:

```text
node.exe npm-cli.js install -g openclaw@latest --allow-scripts=openclaw
```

- [x] structured `executable + argv`
- [x] shell command string 조합 없음
- [x] 기존 `ProcessRunner` 사용
- [x] 설치 timeout: 15분
- [x] user-controlled version/channel 입력 없음
- [x] 설치 target은 항상 `openclaw@latest`

### 설치 후 OpenClaw spawn entry 해석

npm 글로벌 설치로 생성되는 `.cmd`, `.ps1`, extension-less shim을 shell 없이 직접 spawn하지 않는다.

- [x] npm global prefix 해석
- [x] global package root의 `node_modules/openclaw` 해석
- [x] 설치된 OpenClaw package의 `package.json` 읽기
- [x] `bin.openclaw` entry 해석
- [x] resolved JS entry가 OpenClaw package root 내부인지 검증
- [x] absolute/canonical path 기반 package boundary 검증
- [x] package root 외부로 탈출하는 entry는 reject
- [x] 정상 entry를:

```text
node.exe <resolved-openclaw-js-entry> ...
```

형태로 실행 가능 항목으로 사용

- [x] 현재 package가 `openclaw.mjs`를 제공하는 경우 해당 entry 정상 해석
- [x] `.cmd` / `.ps1` shim 직접 production spawn 없음

### 설치 후 검증

- [x] Phase 1의 detection/version 기능 재사용
- [x] OpenClaw detect
- [x] OpenClaw `--version` 실행 및 parsing
- [x] 설치 성공 시 설치된 version 반환
- [x] npm exit 0 이후 다음 중 하나라도 실패하면:
  - executable/package entry detect 실패
  - spawn entry 해석 실패
  - `--version` 실패
  - version parsing 실패

stable error:

```text
openclaw-install-verify-failed
```

### 멱등성

- [x] 설치 시작 전 OpenClaw detect
- [x] 이미 설치되어 있으면 npm version/install spawn 없이 기존 version 반환
- [x] 기존 설치 버전이 latest보다 오래되어도 Phase 2에서 update하지 않음
- [x] 기존 OpenClaw update는 Phase 2 Non-Goal

### mutation boundary

- [x] 기본 `cargo test`에서 실제 `npm install -g openclaw` 실행 0회
- [x] fake fixture 기반 테스트에서 실제 system mutation 0회
- [x] 실제 글로벌 install/uninstall은 dedicated real E2E에서만 실행
- [x] S6/S9 유지

### masking

- [x] npm stdout masking
- [x] npm stderr masking
- [x] process error masking
- [x] AppError masking
- [x] 기존 S3/S8 chokepoint 유지

---

## 2. 인터페이스

### OpenClawInstaller

`OpenClawInstaller` infrastructure adapter 추가.

책임:

- [x] node/npm 실행 entry 해석
- [x] npm version 확인
- [x] npm install spawn
- [x] OpenClaw package JS entry 해석
- [x] 모든 process 실행 `ProcessPort` 경유
- [x] production process spawn 단일 경계 유지

직접 `std::process::Command`, `tokio::process`, shell 등을 새 production spawn boundary로 만들지 않는다.

### InstallService

application layer `InstallService`.

오케스트레이션:

```text
기존 OpenClaw detect
→ 이미 설치됨: 기존 version 반환
→ 미설치:
   Node 사전조건
   → npm entry
   → npm version
   → npm version policy
   → npm install
   → OpenClaw package/spawn entry resolve
   → 설치 후 detect/version
   → 결과 반환
```

- [x] structured result
- [x] stable `AppError` mapping
- [x] infra implementation detail frontend 노출 금지

### commands 레이어

Phase 2에서 최초 Tauri commands layer 생성.

#### detect-environment

frontend:

```text
detect-environment
```

Rust:

```text
detect_environment
```

- [x] Phase 1 `EnvironmentService` 노출
- [x] 설치 전 현재 상태 표시
- [x] 설치 완료 후 환경 상태 재확인에 사용 가능

#### install-openclaw

frontend:

```text
install-openclaw
```

Rust:

```text
install_openclaw
```

- [x] `InstallService` 호출
- [x] 설치 결과 + 설치 후 detect/version 결과 반환

### IPC 계약

architecture §5 준수.

- [x] `src/lib/tauri/`에 frontend command name/type 정의 1곳
- [x] command 문자열 중복 정의 금지
- [x] 명시적 serde request/response 타입
- [x] `serde_json::Value` 중심 임의 계약 금지
- [x] stable `AppError` code 사용
- [x] frontend는 stable code를 기준으로 사용자 메시지 매핑

### 신규 stable error code

- [x] `unsupported-node-version`
- [x] `unsupported-npm-version`
- [x] `npm-not-found`
- [x] `openclaw-install-failed`
- [x] `openclaw-install-verify-failed`

기존 재사용:

- [x] `node-not-found`
- [x] `process-timeout`
- [x] `process-failed`

### async / UI blocking

- [x] `install_openclaw` Tauri command async
- [x] 장시간 blocking process 작업이 UI thread를 block하지 않음
- [x] 기존 async/process architecture를 활용
- [x] progress stream은 구현하지 않음
- [x] 완료 시 1회 invoke 결과 반환

### command safety

- [x] shell string 조합 0건
- [x] installer user input 0개
- [x] version/channel user input 없음
- [x] target 항상 `latest`
- [x] S1/S2/S10 유지

---

## 3. UI

Frontend는 한국어 중심.

위치:

```text
src/features/setup/
```

### 환경 상태

`detect-environment` IPC 사용.

다음 상태 표현:

- [x] OpenClaw 미설치
- [x] OpenClaw 설치됨 + version
- [x] Node 미설치
- [x] Node unsupported
- [x] npm 미발견
- [x] npm unsupported
- [x] 기타 structured error

### 설치

- [x] OpenClaw 설치 버튼
- [x] 설치 진행 중 상태
- [x] 진행 중 중복 클릭 방지
- [x] progress percentage/stream 없음
- [x] invoke 완료 후 결과 1회 처리

### 성공

- [x] 설치된 OpenClaw version 표시
- [x] 이미 설치된 경우 기존 version 표시

### 실패

stable error code별 한국어 메시지.

최소:

- [x] `node-not-found`
- [x] `unsupported-node-version`
- [x] `npm-not-found`
- [x] `unsupported-npm-version`
- [x] `openclaw-install-failed`
- [x] `openclaw-install-verify-failed`
- [x] `process-timeout`
- [x] fallback generic error

- [x] 재시도 UI 제공

### i18n

- [x] `install` namespace 생성
- [x] 한국어 문자열
- [x] 기존 i18n architecture 유지

### Tauri frontend wrapper

위치:

```text
src/lib/tauri/
```

- [x] command 이름 정의
- [x] request/response 타입
- [x] IPC invoke wrapper
- [x] command string/type single source 유지

### frontend test runner

- [x] 실제 frontend test runner 도입
- [x] `pnpm test` placeholder 제거
- [x] `pnpm test`가 실제 테스트 실행
- [x] IPC wrapper typing/logic 테스트
- [x] setup/install UI에서 핵심 상태 매핑 테스트

---

## 4. 테스트

기본 테스트는 fake fixture만 사용한다.

실제 npm global mutation은 real E2E 외부에서 금지.

### fake npm fixture

위치:

```text
tests/fixtures/npm/
```

fake-openclaw 패턴과 동일한 fixture 전략 사용.

지원 behavior:

- [x] `--version`
- [x] `install -g`
- [x] success
- [x] non-zero exit
- [x] timeout
- [x] argv capture/verification

### happy path

- [x] Node supported
- [x] npm 발견
- [x] npm version parsing
- [x] 정확한 executable 검증
- [x] 정확한 argv 검증

npm `<= 11.12`:

```text
node.exe npm-cli.js install -g openclaw@latest
```

검증:

- [x] `--allow-scripts=openclaw` 없음

npm `>= 11.16`:

```text
node.exe npm-cli.js install -g openclaw@latest --allow-scripts=openclaw
```

검증:

- [x] `--allow-scripts=openclaw` 있음

### npm unsupported range

예:

```text
11.13.0
11.14.x
11.15.x
```

- [x] `unsupported-npm-version`
- [x] install spawn 0회

### 설치 후 검증

fake OpenClaw 설치 후 상태 시뮬레이션.

- [x] fake package root
- [x] fake `package.json`
- [x] `bin.openclaw` entry
- [x] JS entry resolve
- [x] package root boundary
- [x] `node.exe + JS entry` 실행 형태
- [x] detect
- [x] version 반환

### spawn entry unit test

- [x] valid `bin.openclaw`
- [x] current `openclaw.mjs` 형태
- [x] package root 내부 canonical path
- [x] package root 밖으로 탈출하는 entry reject

### Node 사전조건 실패

Node 없음:

- [x] `node-not-found`
- [x] npm spawn 0회

unsupported Node 예:

```text
v23.0.0
```

- [x] `unsupported-node-version`
- [x] npm spawn 0회

지원 major이지만 minimum 미만 예:

```text
v22.22.2
v24.14.x
v25.8.x
```

- [x] `unsupported-node-version`
- [x] npm spawn 0회

### npm 실패

npm non-zero:

- [x] `openclaw-install-failed`
- [x] stderr masked

npm timeout:

- [x] `process-timeout`

### 설치 검증 실패

npm exit 0 이후 OpenClaw 검증 실패:

- [x] `openclaw-install-verify-failed`

### 멱등성

이미 OpenClaw 설치 상태:

- [x] 기존 version 반환
- [x] npm version spawn 0회
- [x] npm install spawn 0회

---

## 5. Real E2E

실제 system mutation을 수행하는 dedicated integration test.

target:

```text
--test real_e2e
```

### 3중 명시 조건

실제 mutation은 다음 조건을 모두 만족할 때만 허용한다.

1. dedicated test target 실행
2. Cargo feature `real-e2e`
3. environment variable:

```text
CLAWDESK_REAL_E2E=1
```

실행 예:

```text
CLAWDESK_REAL_E2E=1 cargo test --features real-e2e --test real_e2e
```

- [x] 기본 `cargo test`에서 real install 0회
- [x] feature 없이 real install 0회
- [x] env 없이 real install 0회
- [x] dedicated target이 아니면 real E2E 실행 없음
- [x] 조건 불만족 시 self-skip 또는 non-mutating NOT-RUN 처리
- [x] 종료 보고에서 실제 실행하지 않은 경우 `NOT-RUN` 명시

### real E2E 실행 내용

조건 충족 시에만:

- [x] 실제 Node/npm 사전조건 확인
- [x] 실제 `npm install -g openclaw@latest`
- [x] 실제 OpenClaw entry detect
- [x] 실제 `openclaw --version`
- [x] version parsing 확인

### cleanup ownership

테스트 실행 전 OpenClaw 상태를 기록한다.

#### 실행 전 이미 OpenClaw 설치됨

- [x] 기존 설치를 test-owned installation으로 취급하지 않음
- [x] uninstall하지 않음
- [x] 기존 사용자 설치를 삭제/변경하지 않음

#### 테스트가 직접 OpenClaw 설치함

- [x] test-owned installation으로 기록
- [x] 검증 후 cleanup uninstall 허용
- [x] verify 실패 시에도 test-owned installation에 한해서 cleanup 시도 가능

### cleanup 안전 규칙

- [x] E2E가 직접 생성하지 않은 기존 설치 uninstall 금지
- [x] test ownership 확인 없이 `npm uninstall -g openclaw` 실행 금지
- [x] 제품 기능으로 uninstall 구현하지 않음
- [x] cleanup은 real E2E test teardown에만 존재

---

# 6. 보안 / Architecture 불변식

Phase 1의 기존 security boundary를 유지한다.

### ProcessRunner

- [x] production process spawn 단일 경계
- [x] 신규 직접 spawn 없음
- [x] executable + argv
- [x] shell command string 없음

### Secret masking

- [x] stdout
- [x] stderr
- [x] process errors
- [x] AppError
- [x] frontend-visible error

기존 masking chokepoint 유지.

### Fail-closed

다음 상황에서 설치 진행 금지:

- [x] Node 없음
- [x] Node unsupported
- [x] npm entry 없음
- [x] npm version parse 실패
- [x] npm 11.13~11.15
- [x] invalid OpenClaw package entry
- [x] package boundary validation 실패

### user environment

- [x] Node 자동 설치 금지
- [x] npm 자동 upgrade 금지
- [x] PowerShell installer 금지
- [x] PATH 영구 mutation 금지
- [x] 다른 서비스/설정 임의 변경 금지

---

# 7. 검증 명령

Phase 2 종료 전 아래 명령을 실제 실행한다.

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

실제 완료 검증 결과 (2026-08-21): 위 명령 전부 통과.

- `cargo test`: lib 101 passed / 0 failed, `installer_contract` 10 passed / 0 failed,
  `openclaw_contract` 19 passed / 0 failed, `real_e2e` 1 passed (self-skip, non-mutating)
- `pnpm test`: 63 passed / 0 failed (3 test files)
- real E2E: NOT-RUN

real E2E는 opt-in이므로 기본 Phase 종료 검증에서 필수 실행하지 않는다.

실행하지 않은 경우:

```text
real E2E: NOT-RUN
```

이라고 명시한다.

실행한 경우에는 dedicated command와 실제 결과를 그대로 기록한다.

---

# Non-Goals

이 Phase에서 구현하지 않는다.

- **Node 자동 설치**
  - winget
  - Chocolatey
  - Scoop
  - portable Node 다운로드
  - Node 부재 시 structured error + 안내만 제공
- **npm 자동 upgrade**
  - unsupported npm 11.13~11.15에서는 structured error + 안내 only
- **공식 PowerShell installer 사용**
  - `iwr | iex`
  - PATH mutation
  - Node auto-provision
- **Onboarding / gateway 서비스**
  - `openclaw onboard`
  - `openclaw gateway install`
  - Windows scheduled task/daemon
  - `openclaw doctor --fix`
- **OpenClaw update 실행**
  - 기존 설치가 구버전이어도 Phase 2에서 update하지 않음
  - update status read-only 기능은 Phase 1 범위
  - update mutation은 후속 Phase
- **Uninstall 제품 기능**
  - real E2E가 직접 만든 설치의 test cleanup은 예외
- **API key / Model / Provider / Reasoning**
  - Phase 3
- **Skills**
- **Plugins**
- **Tools / Security profiles**
- **Discord / Telegram channels**
- **Automations / Cron**
- **Profile / Diagnostics**
- **Integrated Chat**
- **버전/채널 선택 UI**
  - 항상 `latest`
- **설치 progress stream**
- **기본 CI / cargo test에서 real OpenClaw 설치**
- **Windows Hub(OpenClaw Companion) 앱 설치/통합**
  - ClawDesk는 OpenClaw CLI를 npm으로 설치
- **Phase 3 이후 기능 선행 구현**

---

# Phase 종료 보고

`ClawDesk-build` 완료 보고는 다음을 포함한다.

1. 생성/변경 파일
2. 구현 내용
3. PHASE_02.md Exit Criteria 대응표
4. 실행한 검증 명령
5. 실제 검증 결과
   - passed
   - failed
   - ignored/skipped
6. 추가 dependency와 이유
7. 실제 OpenClaw 또는 시스템에 실행한 명령
8. 기존 ProcessRunner / security boundary 유지 확인
9. 미검증 사항
10. Phase 2 Non-Goals 미구현 확인
11. Phase 3 미시작 확인

real E2E를 실행하지 않았다면 반드시:

```text
real E2E: NOT-RUN
```

으로 기록한다.

실행하지 않은 검증을 PASS라고 쓰지 않는다.

테스트 수치는 실제 출력 그대로 보고한다.

Phase 2 완료 후 Phase 3으로 넘어가지 않는다.
