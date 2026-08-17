---
description: phase 검증 수행. exit criteria별 PASS/FAIL/NOT-RUN을 evidence 기반으로 보고.
agent: clawdesk-review
---

ClawDesk Phase 검증: $ARGUMENTS

(인자가 없으면 `docs/phases/ROADMAP.md`에서 현재 진행 phase를 사용. "all"이면 ROADMAP 전체 phase 파일 확인)

다음 순서로 진행하라:

1. 해당 `docs/phases/PHASE_XX.md`의 exit criteria를 항목별로 나열한다.
2. 각 항목을 검증한다:
   - 코드 존재 확인은 `file:line` 인용으로
   - 동작 확인은 검증 명령 실행으로
     (allowlist: `pnpm typecheck/lint/test`, `cargo check/test/clippy`, `cargo fmt --check`)
   - read-only git(`status/diff/log/show`)로 changed files 확인 가능
3. 각 exit criteria 항목별 결과를 표로 정리한다:
   - 항목 | 상태(PASS / FAIL / NOT-RUN) | 근거(file:line 또는 명령 출력)
4. NOT-RUN은 반드시 이유를 붙인다 (예: "미실행: cargo 미설치").
   NOT-RUN을 PASS로 처리하지 않는다.

최종 판정: 모든 항목 PASS일 때만 `PASS`, FAIL 또는 BLOCKER가 있으면 `REVISE`,
환경 문제로 검증이 불가능하면 `BLOCKED` + 사유.
