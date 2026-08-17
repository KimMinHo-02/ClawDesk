---
description: read-only code review. uncommitted diff 또는 지정 파일을 계약 기준으로 검토하고 PASS/REVISE/BLOCKED를 보고.
agent: clawdesk-review
---

ClawDesk Review: $ARGUMENTS

(인자가 없으면 `git diff`로 uncommitted 변경분 전체를 review. 파일 경로 인자가 있으면 해당 파일만)

다음 순서로 진행하라:

1. 검토 대상 확인:
   - `git status`, `git diff` (read-only) 또는 지정 파일 읽기
   - 대상이 비어 있으면 "변경 없음" 보고 후 종료
2. 기준 문서 적용:
   - `docs/phases/PHASE_XX.md` (해당 phase scope/exit criteria)
   - `docs/security/SECURITY_INVARIANTS.md` (전 항목)
   - `docs/architecture/ARCHITECTURE.md` (layering, adapter boundary)
   - `AGENTS.md` (전역 계약)
3. 집중 체크리스트:
   - React가 process/executable을 직접 실행하지 않는지
   - 프로세스 실행이 structured executable+argv인지, shell string 조합이 없는지
   - secret/API key/token이 로그·에러·테스트 출력에 노출되지 않는지
   - repository 경계 이탈, git mutation, real OpenClaw mutation 시도 없는지
   - 테스트가 주장된 동작을 실제로 검증하는지 (test와 claim 일치)
4. 가능하면 검증을 직접 실행해 뒷받침한다: `cargo check/test/clippy`, `pnpm typecheck/lint/test`.
5. findings 보고: 각 항목에 severity (BLOCKER/MAJOR/MINOR), `file:line`, 근거, 수정 방향.
   구현은 수정하지 않는다 (edit deny).

마지막 줄은 반드시: `PASS` | `REVISE` | `BLOCKED`
