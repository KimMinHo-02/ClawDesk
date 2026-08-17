# ClawDesk Product Contract (1.0)

Status: Phase 0 baseline. 이 계약의 변경은 Phase contract 업데이트를 통해만한다.

## 1. 제품 정의

- 제품명: **ClawDesk**
- OpenClaw를 관리하는 Windows 데스크톱 앱.
- 사용자가 OpenClaw 설치·환경·모델·채널·자동화를 **터미널 없이** 관리할 수 있게 한다.

## 2. 플랫폼과 스택

- 플랫폼: **Windows 10/11 x64 전용** (초기 버전)
- 스택: **Tauri 2 + React + TypeScript + Vite + Rust**
- 패키지 매니저: **pnpm**
- UI/사용자 설정 언어: **한국어 중심**
- OpenClaw 버전 정책: **latest stable** 기준

## 3. 핵심 UX 원칙

1. **terminal 미노출**: PowerShell/cmd가 UI에 노출되지 않는다.
2. 사용자는 PowerShell/cmd를 직접 사용할 필요가 없다.
3. React는 PowerShell/cmd/OpenClaw executable을 **직접 실행하지 않는다**.
4. 모든 OS/OpenClaw 작업은 **Rust application/adapter boundary**를 통과한다.
5. 프로세스 실행은 structured **executable + argv**만 허용 (shell string 조합 금지).

## 4. 1.0 기능 범위

### 4.1 모델 / Provider / API key

- 모델 및 provider 추가·편집·삭제 (UI 기준, 최신 stable OpenClaw 설정 형식)
- API key 등록: plaintext persistence 금지 (OS secret store 경유)
- 모델 capability에 따른 **reasoning/thinking effort** 설정 지원
  (모델이 지원하지 않으면 옵션 비활성화)

### 4.2 Skills

- OpenClaw skills 목록 조회, 활성화/비활성화
- skill 상태 변경은 Rust adapter 경유

### 4.3 Plugins

- OpenClaw plugins 목록 조회, 활성화/비활성화
- plugin 실행 상태 표시

### 4.4 Tools / Security

- Tool permission 관리 (허용/거부/확인)
- security profile 선택 (기본 profile + 사용자 프로필)

### 4.5 Channels

- **Discord**, **Telegram** 채널 연결/설정/상태 관리
- 채널 credentials는 secret store 경유

### 4.6 Automations

- automation(예약/트리거 작업) 목록·생성·수정·삭제 UI
- 실행은 OpenClaw 경유

### 4.7 Profile / Update / Diagnostics

- Account/profile 정보 표시
- OpenClaw **update 상태** (update 여부, 버전 차이)
- **API 상태** (gateway/연결 상태)
- Diagnostics: 환경 요약, 로그 조회 (mask 적용)

### 4.8 Integrated Chat

- 앱 내 채팅으로 OpenClaw와 대화
- 모델별 reasoning effort 표시/선택

## 5. Non-goals (1.0)

- macOS / Linux 지원
- terminal UI 제공
- OpenClaw 소스 수정/패치
- explicit real E2E phase 이전의 real OpenClaw install/update 삭제 조작 자동화
- multi-user / 원격 제어
- OpenClaw가 공식 지원하지 않는 비공식 extension

## 6. 성공 기준

- Windows 10/11 x64에서 설치 없이 `pnpm dev` 개발 실행 가능, Phase 10에서 NSIS installer 제공
- 모든 OS/OpenClaw 동작이 UI(한국어)에서 terminal 없이 수행 가능
- security invariants(`docs/security/SECURITY_INVARIANTS.md`) 전 항목 만족

## 7. 단계

구현은 `docs/phases/ROADMAP.md`의 Phase 0~10 순서로만 진행한다.
