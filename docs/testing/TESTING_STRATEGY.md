# Testing Strategy

테스트는 5개 계층으로 나눈다. 상단으로 갈수록 실행 빈도가 낮고 환경 의존성이 높다.

## 1. 계층

| # | 계층 | 위치 | 도구 | 목적 |
| --- | --- | --- | --- | --- |
| 1 | Rust unit | `src-tauri/src/**` (`#[cfg(test)]`) | `cargo test --lib` | port/adapter 순수 로직, error 매핑, parsing |
| 2 | React unit | `src/**` (`*.test.tsx`) | frontend test runner (Phase 1+) | UI logic, IPC 래퍼 타이핑 |
| 3 | OpenClaw fake CLI / contract | `tests/fixtures/openclaw/` | rust test + fixture | CLI output 형식 계약: 정상/변형 출력 |
| 4 | Windows integration | `tests/contract/`, `tests/e2e/windows/` | rust/integration test | real ProcessRunner + fake CLI, OS detect (실제 Windows에서) |
| 5 | real Windows E2E | dedicated target (opt-in) | real OpenClaw | real binary와의 end-to-end |

## 2. Fake CLI 계약 (contract)

- `tests/fixtures/openclaw/`에 `openclaw` fake 명령(fixture)과 출력 fixture를 둔다.
- fake CLI는 real OpenClaw의 출력 형식(버전 문자열, status payload, JSON 구조)을
  latest stable 기준으로 모방한다.
- contract 테스트가 검증할 경우:
  - 정상 output parsing
  - **malformed output** (JSON 파싱 실패, 예상과 다른 형식)
  - **missing executable** (없는 경로)
  - **timeout** (응답 지연)
  - **non-zero exit code**
  - empty/partial stdout
- 각 case는 dedicated fixture + dedicated test로 1:1 대응.

## 3. Real E2E gating (opt-in)

- real OpenClaw를 쓰는 테스트는 **기본 무실행**이다.
- 실행 조건 (둘 다 만족):
  1. `CLAWDESK_REAL_E2E=1` environment 설정
  2. 전용 target 명시 실행 (예: `cargo test --test real_e2e`)
- real E2E는 real OpenClaw install/update/remove가 **유일하게** 허용되는 계층이다.
- CI 기본 파이프라인에서는 real E2E를 실행하지 않는다.

## 4. 금지

- unit/contract 테스트에서 real binary mutation (S5)
- 테스트 출력에 secret 노출 (S3)
- skip/ignore로 미검증을 PASS로 위장 (보고서에 NOT-RUN으로 명시)

## 5. Phase 0 범위

- Phase 0은 **fixture directory와 strategy 문서만** 만든다.
- real test code는 Phase 1(ProcessRunner/adapter)부터 작성한다.
- Phase 0에 test framework 신규 도입은 하지 않는다 (scaffold 최소 유지).
