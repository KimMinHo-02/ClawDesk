# ClawDesk Architecture (최소)

상세 설계는 phase별로 보완한다. 이 문서는 Phase 0 합의된 최소 아키텍처다.

## 1. 레이어 (일방 의존)

```
React (src/)
  → Tauri IPC (invoke, @tauri-apps/api)
  → Rust Commands (src-tauri/src/commands/)
  → Application Service (src-tauri/src/application/services/)
  → Domain Port (src-tauri/src/domain/ports/)
  → Infrastructure Adapter (src-tauri/src/infrastructure/)
```

- 의존 방향은 위부터 아래로만. adapter가 command를 알면 안 된다.
- **React는 process/executable을 직접 실행하지 않는다.**
  frontend에서 `child_process`, PowerShell, cmd, openclaw.exe 호출은 금지다.
  frontend의 유일한 실행 경로는 Tauri IPC다.

## 2. 디렉터리 매핑

```
src/
  app/            애플리케이션 조립 (router, provider)
  components/     공유 UI 컴포넌트
  features/       기능 단위 (setup, home, models, skills, plugins, tools,
                  automations, channels, profile, settings, logs)
  i18n/ko/        한국어 문자열
  lib/tauri/      Tauri IPC 래퍼 (command 명 + 타입)
  types/          공유 타입

src-tauri/src/
  lib.rs          Tauri builder 조립
  main.rs         binary entry
  commands/       Tauri IPC command (입력 검증, AppError 매핑)
  application/
    services/     use case 오케스트레이션 (port 조합)
  domain/
    models/       도메인 타입 (Environment, ClawStatus, ...)
    ports/        adapter 인터페이스 (trait)
  infrastructure/
    openclaw/     OpenClawAdapter
    windows/      WindowsSystemAdapter
    process/      ProcessRunner (spawn)
    secrets/      SecretStore
    filesystem/   filesystem adapter
```

## 3. 핵심 adapter (port/adapter)

| Port (trait) | Adapter | 책임 |
| --- | --- | --- |
| `OpenClawPort` | `OpenClawAdapter` | OpenClaw executable detect, version, gateway status, update status |
| `WindowsSystemPort` | `WindowsSystemAdapter` | Windows version/build, architecture(x64), Node detect |
| `ProcessPort` | `ProcessRunner` | structured spawn, stdout/stderr capture, timeout, exit code |
| `SecretStorePort` | `SecretStore` | API key/credential 저장·읽기 (OS secret store 경유) |

- adapter는 테스트ability를 위해 **trait + fake 구현**(테스트 fixture)을 가진다.
- `ProcessRunner`가 유일한 process spawn 지점이다. 다른 경로로 spawn 금지.

## 4. 프로세스 실행 계약

- spawn 형태: `(executable: PathBuf, argv: Vec<String>)` **のみ**. `shell: true` 금지.
- user input(model명, 채널 ID, 파일경로 등)을 command line 문자열에 보간하지 않는다.
- 모든 실행은 timeout을 가진다. timeout/실패는 structured error로 반환한다.
- stdout/stderr는 masking을 거친 후 UI/로그로 전달한다.

## 5. IPC 계약

- command 이름: kebab-case (frontend) ↔ snake_case (rust), `src/lib/tauri/`에 1곳만 정의.
- argument/return은 serde로 직렬화되는 명시적 타입만 (loose object 금지).
- 오류는 stable error code를 가진 `AppError`로 통일 (raw panic message 금지).

## 6. Phase 0 범위

- 위 구조의 **scaffold만**: 빈 레이어 디렉터리, 최소 Tauri/React 실행 코드.
- adapter 비즈니스 로직(Phase 1+), IPC command 본문은 Phase 계약이 시작될 때만 작성.
- Phase 0에 placeholder business logic을 만들지 않는다.
