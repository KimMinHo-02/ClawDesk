# Security Invariants

ClawDesk 전역 보안 불변식. 어느 phase, 어느 구현도 이 목록을 위반하면 안 된다.
위험하거나 불확실한 경우 **거절(reject)한다**, 우회하지 않는다.

## S1. executable + argv only

모든 프로세스 실행은 `(executable, argv[])` structured form이다.
`Command::new(shell)` + argument string, `shell: true`, command line 문자열 조합은 금지.

## S2. user input shell interpolation 금지

사용자 입력(모델명, provider ID, 채널 설정, 파일경로, automation 이름 등)을
shell 문자열에 연결/보간해 실행하지 않는다. 입력은 argv 요소로서만 전달한다.

## S3. secret masking

API key, token, password, channel credential은
로그, error message, UI, 테스트 출력 어디에도 plain으로 노출하지 않는다.
mask 형식 통일 (예: `sk-...****`).

## S4. repository boundary

ClawDesk repository 경계 밖 파일 작성/수정/삭제 금지.
adapter가 OS 설정을 건드리는 경우(예: OpenClaw config)는 phase 계약으로 명시된 경로에만 허용.

## S5. unit/contract test에서 real installation 금지

unit/contract/integration 테스트는 **fake CLI fixture**만 사용.
real OpenClaw binary에 대한 detect 외 mutation 시도 금지.

## S6. destructive commands 금지

- `rm`/`rmdir`/`Remove-Item`류 파일 삭제
- git mutation: `add/commit/push/reset/restore/clean/stash/rebase/merge`
- real OpenClaw install/update/remove (explicit real E2E layer 제외)

위 동작은 CI/agent/테스트 어디에서도 금지다.

## S7. API key plaintext persistence 금지

API key/credential은 OS secret store(DPAPI 등) 경유로만 저장.
설정 파일, JSON, 로그, environment dump에 plaintext 기록 금지.

## S8. stdout/stderr/error masking

OpenClaw/외부 process 출력은 UI/로그 전달 전에 mask pipeline을 거친다.
error serialization에도 secret 포함 금지.

## S9. explicit real E2E gating

real OpenClaw mutation이 가능한 real E2E는 **명시적 opt-in**으로만 실행:
- environment: `CLAWDESK_REAL_E2E=1`
- 그리고 전용 test target (`--test real_e2e` 류) 으로만 진입
- 기본 CI/`cargo test`/`pnpm test`에서는 절대 실행되지 않는다.

## S10. hidden PowerShell 제한

UI에 terminal을 노출하지 않는(hidden) 컨셉이지만,
향후 어떤 형태로든 PowerShell/cmd를 spawn해야 하는 요구가 생기면
**Rust `ProcessRunner` 경유만** 허용한다.
React/WebView/전端 스크립트에서 shell 접근은 영구 금지.
