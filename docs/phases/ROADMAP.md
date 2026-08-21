# ClawDesk Roadmap

Phase는 선형으로 진행한다. 이전 phase exit criteria 미충족 시 다음 phase 시작 금지.
각 phase의 상세 계약은 `PHASE_XX.md`에 있다.

| Phase | 이름                                   | 요약                                                                                                                                | 상태        |
| ----- | -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| 0     | Product / Harness                      | 제품 계약, security invariants, 아키텍처, OpenCode harness(agent/command), 최소 scaffold                                            | completed   |
| 1     | Windows Environment + OpenClaw Adapter | Windows 버전/아키텍처 detect, Node detect, OpenClaw executable/version/gateway/update status, ProcessRunner, fake CLI contract test | completed   |
| 2     | Installer                              | OpenClaw 설치 플로우 (UI + adapter). real mutation은 explicit E2E에서만 검증                                                        | completed   |
| 3     | Model / Provider / API / Reasoning     | 모델·provider·API key 관리, capability 기반 reasoning/thinking effort                                                               | completed   |
| 3.1   | Phase 3 Review Revision                | F1-F4 수정: API key life-cycle 순서(DPAPI 우선) + orphan cleanup, fake protected path 모방, 계약 문서 개정, argv capture contract test | completed   |
| 4     | Skills / Plugins                       | skills·plugins 목록과 활성화 관리                                                                                                   | not started |
| 5     | Tools / Security                       | tool permission, security profile                                                                                                   | not started |
| 6     | Channels                               | Discord/Telegram 채널 연결·설정                                                                                                     | not started |
| 7     | Automations                            | automation 관리 (생성/수정/삭제/활성화)                                                                                             | not started |
| 8     | Profile / Update / Diagnostics         | profile, OpenClaw update 상태, API 상태, diagnostics                                                                                | not started |
| 9     | Integrated Chat                        | 앱 내 chat (모델별 reasoning effort)                                                                                                | not started |
| 10    | Windows Release                        | NSIS installer, 코드사인, 릴리스 파이프라인                                                                                         | not started |

## 규칙

- Phase 0 exit criteria 충족 전 Phase 1 구현 시작 금지.
- phase 범위 확장/축소는 해당 `PHASE_XX.md` 수정 후 진행.
- real OpenClaw mutation은 Phase 2+ 에서도 explicit real E2E layer(opt-in)에서만.
