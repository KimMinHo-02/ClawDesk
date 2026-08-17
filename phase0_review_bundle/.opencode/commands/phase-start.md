---
description: phase 시작 절차. Phase contract를 불러와 범위/non-goals를 확정하고 구현 계획을 세운다. 구현은 하지 않는다.
agent: clawdesk-build
---

ClawDesk Phase 시작: $ARGUMENTS

(인자가 없으면 docs/phases/ROADMAP.md에서 다음 미시작 phase를 사용. 예: /phase-start 1, /phase-start 01)

다음 순서로 진행하라:

1. `docs/phases/ROADMAP.md`와 해당 `docs/phases/PHASE_XX.md`를 읽는다.
2. 계약 문서 확인: `AGENTS.md`, `docs/product/PRODUCT_CONTRACT.md`,
   `docs/architecture/ARCHITECTURE.md`, `docs/security/SECURITY_INVARIANTS.md`,
   `docs/testing/TESTING_STRATEGY.md`.
3. phase start 보고를 작성하라:
   - phase 목표 1문장
   - in-scope 항목 (phase contract에서)
   - non-goals (이번 phase에서 금지한 것)
   - 예상 수정/생성 파일 목록
   - 검증 계획 (실행할 검증 명령 목록, allowlist 기준)
   - open questions / blocker
4. **이 단계에서는 어떤 구현도 하지 않는다.** `edit`을 사용하지 않는다.
5. 구현은 사용자가 `/phase-implement`를 호출하는 시점에 시작한다.
