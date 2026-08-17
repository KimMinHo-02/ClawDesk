# AGENTS.md — ClawDesk 전역 개발 계약

이 파일은 ClawDesk repository에서 일하는 모든 agent(OpenCode 포함)와 개발자의 전역 계약입니다.
다른 문서와 충돌할 경우 `AGENTS.md`와 `docs/security/SECURITY_INVARIANTS.md`가 우선합니다.

## 1. 제품 기본

- 제품명: ClawDesk
- 플랫폼: 초기 버전 Windows 10/11 x64 전용
- 스택: Tauri 2 + React + TypeScript + Vite + Rust, 패키지 매니저 pnpm
- UI/사용자 설정: 한국어 중심
- OpenClaw: latest stable 기준

## 2. 프로세스 실행 계약 (최상위 우선순위)

- 사용자는 PowerShell/cmd를 직접 사용할 필요가 없어야 함.
- React(전端)는 PowerShell/cmd/OpenClaw executable을 직접 실행하지 않음.
- 모든 OS/OpenClaw 작업은 Rust application/adapter boundary를 통과해야 함.
- 프로세스 실행은 structured `executable + argv`만 허용.
- shell command string 조합(shell: true, 문자열 연결 명령) 금지.
- explicit E2E Phase 전까지 real OpenClaw install/update/remove 금지.
- 실제 API key 사용 금지, 외부 서비스 연결 금지.

## 3. 보안

- secret/API key/token을 로그, 에러, UI, 테스트 출력에 출력하지 않음 (masking 필수).
- repository 외부 수정 금지 (ClawDesk repository 내부에서만 파일 작성/수정).
- API key plaintext persistence 금지.

## 4. 범위 규율

- Phase contract(`docs/phases/`) 밖의 기능 구현 금지.
- Phase 0 종료 전 Phase 1 구현 시작 금지.
- real OpenClaw mutation은 explicit real E2E layer(opt-in)에서만.

## 5. Git

- git mutation 금지: `add / commit / push / reset / restore / clean / stash / rebase / merge`(및 `checkout`, `switch`, `update-ref`, `gc` 포함).
- read-only git(`status`, `diff`, `log`, `show`)만 허용.

## 6. 문서

- 제품 계약: `docs/product/PRODUCT_CONTRACT.md`
- 아키텍처: `docs/architecture/ARCHITECTURE.md`
- 보안 불변식: `docs/security/SECURITY_INVARIANTS.md`
- 테스트 전략: `docs/testing/TESTING_STRATEGY.md`
- 로드맵/페이즈 계약: `docs/phases/`

## 7. 검증과 보고

- 변경 후에는 allowlist 내 focused verification을 실행한다
  (`pnpm typecheck` / `pnpm lint` / `pnpm test`, `cargo check` / `cargo test` / `cargo clippy` / `cargo fmt`).
- 실행하지 못했을 경우 실패로 속이지 않고 "미실행 + 이유"로 보고한다.
- 완료 보고 형식: 생성/변경 파일, 구현 내용, 검증 명령, 검증 결과, 추가 dependency, 미검증 사항.
