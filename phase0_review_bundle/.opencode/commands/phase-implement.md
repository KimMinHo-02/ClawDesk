---
description: 현재 phase 계약 범위 내 구현 실행. focused verification과 완료 보고까지 수행.
agent: clawdesk-build
---

ClawDesk Phase 구현: $ARGUMENTS

(인자가 없으면 현재 진행 phase를 `docs/phases/ROADMAP.md`에서 판단. 예: /phase-implement 1, /phase-implement "Phase 1")

다음 순서로 진행하라:

1. `docs/phases/PHASE_XX.md`를 다시 읽고 in-scope / non-goals를 재확인한다.
   `/phase-start` 보고가 있다면 그 범위에서 벗어나지 않는다.
2. non-goals 확인: phase contract가 금지하는 것 (예: real OpenClaw install/update,
   API key 설정, GUI 기능) 은 절대 구현하지 않는다.
3. `AGENTS.md` 규칙을 지키며 구현한다:
   - structured `executable + argv`만, shell string 조합 금지
   - React에서 process 직접 실행 금지, Tauri IPC만
   - secret masking, repository 경계 내부만 수정
   - git mutation 금지
   - placeholder business logic 금지
4. 범위 초과가 필요하다고 판단되면 구현을 멈추고 `BLOCKED` + 사유를 보고하라.
5. focused verification 실행 (allowlist):
   - frontend: `pnpm typecheck`, `pnpm lint`, `pnpm test`
   - rust: `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt --check`
6. `clawdesk-build` agent 완료 보고 형식(7개 항목)으로 최종 보고한다.
