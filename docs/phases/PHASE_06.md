# Phase 6 — Channels (Discord / Telegram)

Status: not started

## 목표

ClawDesk에서 **Discord와 Telegram 채널의 연결(enable), 토큰(credential) 관리, 접근 정책(DM/그룹), 상태 표시, 최초 연결(pairing)**을 terminal 없이(한국어 UI) 관리한다.

- 제품 계약 §4.5: "Discord, Telegram 채널 연결/설정/상태 관리", "채널 credentials는 secret store 경유".
- 채널 토큰(Discord bot token, Telegram bot token)은 **ClawDesk DPAPI secret store(Phase 3 `SecretStorePort`) 경유로만** 저장하고 exec SecretRef로 공급한다. `openclaw.json` plaintext 금지 (S7).
- 모든 config mutation은 Phase 3 `OpenClawConfigPort`(`openclaw config` CLI, structured argv, dry-run → commit) 경유만. ClawDesk가 `openclaw.json` 파일을 직접 편집하지 않는다.
- Discord는 **official plugin**: `openclaw plugins install @openclaw/discord` (idempotent). Telegram은 **bundled plugin** (core install 포함, install 불필요).
- 채널 런타임은 **Gateway process**가 소유한다. ClawDesk는 gateway를 시작/정지/재시작하지 않는다 (Phase 8 영역). gateway 실행 중이면 config write를 watch하여 영향받는 채널을 자동 restart한다. 미실행이면 상태는 config 기준 표시.
- 두 채널의 기본 DM 정책은 `pairing`이므로 **pairing code approve**(`openclaw pairing` CLI)가 최초 연결 플로우의 일부다.

설계 근거 (2026-08-24 접속 기준, `docs.openclaw.ai` live docs):

- `https://docs.openclaw.ai/channels`
- `https://docs.openclaw.ai/channels/discord`
- `https://docs.openclaw.ai/channels/telegram`
- `https://docs.openclaw.ai/cli/channels`
- `https://docs.openclaw.ai/cli/pairing`
- `https://docs.openclaw.ai/gateway/secrets`
- `https://docs.openclaw.ai/reference/secretref-credential-surface`
- 재사용: `https://docs.openclaw.ai/cli/config` (Phase 3), plugins CLI (Phase 4)

핵심 사실 (상기 docs 기준):

- **Discord** (official plugin):
  - install: `openclaw plugins install @openclaw/discord` (install 후 Gateway restart 필요 — gateway 실행 중이면 config watch로 자동 reload)
  - config: `channels.discord.{enabled, token, dmPolicy, allowFrom, groupPolicy, applicationId, ...}`
  - 토큰: `channels.discord.token` — SecretRef 지원 명시 (credential surface). env fallback `DISCORD_BOT_TOKEN`(default account 전용)은 ClawDesk가 gateway process env를 관리하지 못하므로 **SecretRef만 사용**.
  - `dmPolicy`: `pairing`(default) | `allowlist` | `open` | `disabled`; `allowFrom`이 canonical DM allowlist (Discord user ID 또는 `*`)
  - `groupPolicy`: `open` | `allowlist` | `disabled` (guild allowlist 편집은 본 phase non-goal)
  - config validation: `dmPolicy=allowlist`는 allowFrom 1개 이상 요구, `open`은 `allowFrom`에 `*` 포함 요구
- **Telegram** (bundled plugin, install 불필요):
  - config: `channels.telegram.{enabled, botToken, dmPolicy, allowFrom, groupPolicy, groups, ...}`
  - 토큰: `channels.telegram.botToken` — SecretRef 지원 명시 (credential surface). env fallback `TELEGRAM_BOT_TOKEN`(default account 전용) — **SecretRef만 사용**
  - `dmPolicy`: `pairing`(default) | `allowlist` | `open` | `disabled`; `allowFrom`은 numeric user ID (1–32 자리, `telegram:`/`tg:` prefix는 OpenClaw가 normalize) 또는 `*`
  - `groupPolicy`: `open` | `allowlist`(default) | `disabled` (groups map 편집은 non-goal)
  - config validation: `dmPolicy=allowlist` + 빈 allowFrom → reject
- **SecretRef (Phase 3 패턴 재사용)**:
  - `channels.discord.token` / `channels.telegram.botToken` 둘 다 SecretRef credential surface에 명시 등재
  - Phase 3 `clawdesk` exec provider(`secrets.providers.clawdesk`, `clawdesk-secret-resolver` binary) 재사용 — ref shape `{source:"exec", provider:"clawdesk", id:"<key-id>"}`
  - DPAPI-first lifecycle, idempotent declaration, orphan cleanup (PHASE_03/PHASE_03_1 관례)
- **CLI**:
  - `openclaw channels list --all --json` — 설정된 계정 + `installed`/`configured`/`enabled` tag
  - `openclaw channels status [--channel <name>] [--json]` — 채널별 runtime 상태. **gateway 불능 시 config-only summary로 fallback** (live 상태와 구분 표시 필요)
  - `openclaw pairing list <channel> [--json]` — pending pairing requests
  - `openclaw pairing approve <channel> <code>` — 코드 승인 (최초 1회 승인 시 `commands.ownerAllowFrom` bootstrap — one-time owner bootstrap)
  - `channels status --probe` (live probe), `capabilities`/`resolve`/`logs`/`dead-letters` — non-goal

미확인 항목 (구현 시 live CLI/docs로 확정, 추측 금지):

- `channels list --all --json` 출력 스키마 (account row shape, tag field 명칭)
- `channels status --json` 출력 스키마 (runtime state field, gateway 불능 config-only fallback의 구분 신호)
- `pairing list --json` 출력 스키마 (code field 명칭, request row shape)
- `plugins install <npm-id>` exact argv shape (positional id, 추가 flag)와 already-installed일 때 exit code/출력
- `plugins install`이 `plugins enable`을 포함하는지 (별도 enable 필요 여부)
- pairing code 정확 형식 (길이/character set)
- gateway의 channel config 변경 auto-reload 시점/동작 (UI 안내 문구 기준)

---

# Exit Criteria

## 1. Channel Token (Rust, S7)

- [ ] 신규 key-id helper: `channels/discord/token`, `channels/telegram/botToken`
  (exec id pattern, 기존 `is_valid_key_id` 통과)
- [ ] 신규 SecretRef helper: `{source:"exec", provider:"clawdesk", id:"channels/<channel>/<field>"}`
  (기존 `CLAWDESK_SECRET_ALIAS` 재사용)
- [ ] `set-channel-token` lifecycle (Phase 3 `ApiKeyService` 순서):
  1. channel/token 검증 (channel ∈ {discord, telegram}, token trim 후 non-empty)
  2. 현재 토큰 field read → **ClawDesk ref가 아닌 값(plaintext string, provider≠clawdesk ref)이 이미 존재** →
     `secret-ref-registration-failed` ("OpenClaw에서 먼저 변경") — DPAPI 포함 write 0회
  3. **DPAPI set first** (실패 시 config write 0)
  4. `secrets.providers.clawdesk` declaration 보장 (idempotent — Phase 3과 공유, 일치 시 write 0)
  5. `channels.discord.token` / `channels.telegram.botToken` SecretRef write
     (dry-run → commit, `WriteMode::Plain`)
- [ ] `delete-channel-token`:
  - clawdesk ref 존재 → ref unset → DPAPI delete
  - ref 부재 + store entry 존재 (orphan) → store-only cleanup
  - 둘 다 없음 → `channel-token-not-found`
  - declaration cleanup: `secrets.providers.clawdesk`는 **provider surface(`models.providers.*.apiKey`)과
    channel surface(두 채널 token path) 모두에 clawdesk-managed ref가 없으면만** unset (남아 있으면 declaration write 0)
- [ ] `list-channel-tokens`: store index (non-secret) → 채널별 `{channel, registered}`
- [ ] 토큰 값은 config/argv/log/error/UI 어디에도 0 (S3/S7/S8) — argv capture assert
- [ ] secret store 불가 → `secret-store-unavailable` (Phase 3 재사용), config write 0

## 2. Channel Connect / Enable (Rust)

- [ ] `connect-channel`:
  - Discord:
    1. 토큰 등록 확인 (clawdesk ref 존재) — 없으면 `channel-token-not-found` (CLI 0회)
    2. plugin install 확인 (Phase 4 `plugins list` — `@openclaw/discord` 존재?) —
       없으면 `openclaw plugins install @openclaw/discord` (단일 argv, timeout 300초) →
       exit 0 후 `plugins list` 재확인 (없으면 `openclaw-plugin-install-failed`)
    3. `channels.discord.enabled=true` write (dry-run → commit)
  - Telegram:
    1. 토큰 등록 확인 → `channels.telegram.enabled=true` write (install 단계 없음)
  - 위 순서 고정, **첫 실패 시 중단** (partial continue 금지), 단계별 stable error
- [ ] `set-channel-enabled` (disable 포함): `channels.<channel>.enabled` scalar write
  (false로 disable — 토큰/정책 보존)
- [ ] plugin install **idempotent** (already installed 시 exit 0 + `plugins list` post-check)
- [ ] `openclaw plugins update/remove`, Discord 외 plugin install — 0회 (non-goal)
- [ ] gateway start/stop/restart — 0회 (ClawDesk는 gateway를 관리하지 않음)

## 3. Access Policy (Rust)

- [ ] `set-dm-access` {channel, dmPolicy, allowFrom}:
  - 사전 검증 (fail-closed, CLI 0회):
    - `dmPolicy` ∈ {pairing, allowlist, open, disabled}
    - allowFrom entry: `*` 또는 numeric ID `[0-9]{1,32}` (Discord/Telegram 공통 —
      ClawDesk는 numeric만 수락, prefix 형식은 reject)
    - cross-rule: `allowlist` → entry 1개 이상, `open` → `*` 포함 — 위반 시 `dm-access-inconsistent`
  - 2 path 순서 write: `channels.<channel>.dmPolicy` (scalar) →
    `channels.<channel>.allowFrom` (array `--replace`) — 첫 실패 시 중단
- [ ] `set-group-policy` {channel, groupPolicy}:
  - enum: `open` | `allowlist` | `disabled` — 단일 path write
- [ ] policy read: `config get channels.discord --json` / `config get channels.telegram --json`
  (fail-soft — section 부재 → `enabled` null, allowFrom 빈 배열, `tokenState: "absent"`)
  - 응답: `{enabled, tokenState, dmPolicy, allowFrom, groupPolicy}`
  - `tokenState`: `managed` (clawdesk ref) | `external` (string/타 provider ref) | `absent`
    — **shape 기반 분류만, 토큰 값 자체는 응답에 0** (redacted snapshot)
- [ ] dry-run `ok: false` → write 0회 + `openclaw-config-invalid` (재사용)
  - 예: Telegram `allowlist` + 빈 allowFrom config validation reject

## 4. Status / Pairing (Rust)

- [ ] `get-channels`:
  - `openclaw channels list --all --json` + `openclaw channels status --json` (둘 다 read-only, 30초)
  - `discord`/`telegram` row로 필터, 부재 row → `installed:false, configured:false` (fail-soft)
  - 통합 row: `{id, installed, configured, enabled, runtimeState, gatewayReachable}`
  - `runtimeState`: raw string 유지 (unknown 값 raw, fail-soft)
  - `gatewayReachable: false` (gateway 불능 → config-only fallback 감지) →
    UI "연결됨" 추정 0 (설정 기준 상태 명시)
- [ ] `list-pairing-requests` {channel}: `openclaw pairing list <channel> --json` (30초)
  - request row: `code` 필수 (부재 row drop), sender/나머지 field fail-soft
- [ ] `approve-pairing` {channel, code}: `openclaw pairing approve <channel> <code>` (30초)
  - code 검증 (S2): non-empty, 4–64 chars, `[A-Za-z0-9_-]`, 단일 argv 요소 —
    위반 시 `pairing-code-invalid` (CLI 0회)
  - owner bootstrap: UI 안내만 (승인 성공 시 "최초 승인 시 command owner가 설정될 수 있음" —
    추가 CLI 호출 0)
- [ ] channels list/status/pairing 실패/parse 실패 → stable error (아래 §5 코드),
    timeout → `process-timeout` 재사용
- [ ] `--probe`, `capabilities`, `resolve`, `logs`, `dead-letters` — 0회 (non-goal)

## 5. 인터페이스

### ports / adapters

- [ ] `domain/ports`에 `OpenClawChannelsPort` (list_channels, channel_status,
      pairing_list, pairing_approve) trait + `infrastructure/openclaw`
      `OpenClawChannelsAdapter` 구현
- [ ] `domain/ports`에 `OpenClawPluginInstallPort` (install_plugin) trait +
      `OpenClawPluginInstallAdapter` 구현 (timeout 300초)
- [ ] token: 신규 port 없음 — `ChannelTokenService`가 Phase 3 `OpenClawConfigPort`
      + `SecretStorePort` 조합 (Phase 3 `ApiKeyService` 패턴 재사용)
- [ ] 모든 process 실행은 `ProcessPort`/`ProcessRunner` 경유 (spawn 단일 경계 유지)
- [ ] `std::process::Command` 직접 사용 0, shell 사용 0회 (S1)

### commands 레이어 (Tauri IPC)

Phase 4–5의 `commands` 레이어 확장. frontend kebab-case ↔ Rust snake_case.

| frontend                | Rust                  | payload → result                                              |
| ----------------------- | --------------------- | ------------------------------------------------------------- |
| `get-channels`          | `get_channels`        | → `{gatewayReachable, channels[]}`                            |
| `get-channel-config`    | `get_channel_config`  | `{channel}` → `{enabled, tokenState, dmPolicy, allowFrom, groupPolicy}` |
| `set-channel-token`     | `set_channel_token`   | `{channel, token}`                                            |
| `delete-channel-token`  | `delete_channel_token`| `{channel}`                                                   |
| `connect-channel`       | `connect_channel`     | `{channel}`                                                   |
| `set-channel-enabled`   | `set_channel_enabled` | `{channel, enabled}`                                          |
| `set-dm-access`         | `set_dm_access`       | `{channel, dmPolicy, allowFrom}`                              |
| `set-group-policy`      | `set_group_policy`    | `{channel, groupPolicy}`                                      |
| `list-pairing-requests` | `list_pairing_requests` | `{channel}` → `{requests[]}`                               |
| `approve-pairing`       | `approve_pairing`     | `{channel, code}`                                             |

### IPC 계약

architecture §5 준수 (Phase 2–5와 동일 기준):

- [ ] `src/lib/tauri/`에 command name/type 1곳 정의, 중복 정의 금지
- [ ] 명시적 serde 타입만 (wire = camelCase, `serde_json::Value` 중심 임의 계약 금지)
- [ ] `AppError` code 기반 frontend 메시지 매핑

### 신규 stable error code

- [ ] `channel-id-invalid` (channel ∈ {discord, telegram} 검증 실패)
- [ ] `channel-token-invalid` (빈 토큰)
- [ ] `channel-token-not-found` (delete 대상 없음 / connect 전제 토큰 미등록)
- [ ] `dm-policy-invalid`
- [ ] `group-policy-invalid`
- [ ] `allow-from-entry-invalid`
- [ ] `dm-access-inconsistent` (allowlist+빈 배열, open+`*` 부재)
- [ ] `pairing-code-invalid`
- [ ] `openclaw-channels-failed` (channels list/status 실행/parse 실패)
- [ ] `openclaw-pairing-failed` (pairing list/approve 실행/parse 실패)
- [ ] `openclaw-plugin-install-failed`

기존 재사용:

- [ ] `openclaw-config-read-failed`, `openclaw-config-write-failed`, `openclaw-config-invalid`
- [ ] `openclaw-not-found`, `process-failed`, `process-timeout`
- [ ] `secret-ref-registration-failed`, `secret-store-unavailable`

### command safety

- [ ] shell string 조합 0건 (S1)
- [ ] user input(token, allowFrom, code, channel)은 **사전 형식 검증 후**
      argv/config path에 사용 (S2) — **토큰은 argv에 0회** (SecretRef 경유만)
- [ ] CLI 호출 timeout: config/channels/pairing 30초, plugins install 300초
- [ ] JSON payload(allowFrom 배열, SecretRef object)는 단일 argv 요소

## 6. UI

Frontend는 한국어 중심.

위치:

```text
src/features/channels/
```

### Channel 카드 / 상태

- [ ] Discord/Telegram 카드: installed/configured/enabled badge,
      runtime state (한국어 라벨 — unknown raw 값은 "미확인"),
      token 상태 (등록됨/외부 관리/미등록)
- [ ] `gatewayReachable=false` → "Gateway 실행 중 아님 — 설정 기준 상태" 안내
      (연결됨 추정 0)
- [ ] config 변경 안내: "Gateway 실행 중이면 변경이 자동 반영됩니다."

### 연결 / 토큰

- [ ] 토큰 입력 (password field, mask) → set-channel-token
- [ ] 연결 버튼 (토큰 미등록 시 disabled 또는 실행 시 `channel-token-not-found` 안내)
      → connect-channel (Discord: plugin install 진행 상태 표시)
- [ ] 비활성화 버튼 (set-channel-enabled false, 확인 다이얼로그)
- [ ] 토큰 삭제 (확인 다이얼로그) — enabled 유지, tokenState=미등록 표시

### 접근 정책

- [ ] dmPolicy selector (4단계, 한국어: 페어링 / 허용 목록 / 공개 / 비활성)
- [ ] allowFrom editor (add/remove, chip 표시)
  - entry 검증 (frontend) + Rust 재검증 (backend) — `*` 또는 numeric ID
  - "허용 목록: 1개 이상 입력", "공개: `*` 포함" 안내 문구
- [ ] groupPolicy selector (3단계, 한국어: 공개 / 허용 목록 / 비활성)
- [ ] 진행 중 중복 submit 방지 + 변경 후 재조회 (optimistic update 금지)

### Pairing

- [ ] pending pairing request 목록 (code, sender fail-soft 표시)
- [ ] code 입력 + approve (중복 실행 guard, 성공 시 목록+상태 재조회)
- [ ] owner bootstrap 안내 (승인 전 정적 안내: "최초 승인 시 command owner가 설정될 수 있습니다.")

### i18n

- [ ] `channels` namespace 생성 (한국어)
- [ ] 기존 i18n architecture 유지

### Tauri frontend wrapper

- [ ] `src/lib/tauri/`에 Phase 6 command wrapper 추가 (single source 유지)
- [ ] React에서 process/executable 직접 실행 0 (invoke만 — S10)

### frontend test

- [ ] access policy edit state 로직 (cross-rule 검증, 중복 guard, 재조회 트리거)
- [ ] token flow state 로직 (입력 mask, connect 전제 guard)
- [ ] pairing approve state 로직 (진행 중 guard, 실패 시 fail-closed)
- [ ] error code → 한국어 메시지 매핑 테스트

## 7. 테스트

기본 테스트는 fake CLI fixture만 사용한다 (S5).

### fake CLI 확장 (`fixtures/fake-openclaw/`)

- [ ] `channels list --all --json`, `channels status --json` handler:
  state 기반 (installed/configured/enabled + runtime state,
  `channelsStatus.gatewayReachable` state로 live/config-only 분기)
- [ ] `pairing list <channel> --json`, `pairing approve <channel> <code>` handler:
  state `pairing.<channel>.requests` 기반 (approve 시 해당 code row 제거)
- [ ] `plugins install <npm-id>` handler:
  state `plugins.installed[]`에 추가, behavior override 재사용
- [ ] `config get/set channels.*` — 기존 config 핸들러 재사용
  (channels.*는 protected path 아님)
- [ ] argv capture 유지 (기존 패턴)

### contract 테스트 (`tests/`)

happy path:

- [ ] get-channels: exact argv (`channels list --all --json` + `channels status --json`),
  state → 통합 row 파싱, gatewayReachable 분기
- [ ] connect (discord): `plugins list` → (미설치 시) `plugins install @openclaw/discord`
  exact argv → `channels.discord.enabled` write exact argv (dry-run+commit), state 갱신
- [ ] connect (telegram): 토큰 확인 → `channels.telegram.enabled` write exact argv
- [ ] set-dm-access: 2 path 순서 (dmPolicy → allowFrom) exact argv, state 갱신
- [ ] set-group-policy: exact argv, state 갱신
- [ ] set-channel-token: DPAPI first, declaration idempotent (2회 실행 시 2회 write 0),
  ref write exact argv (body = SecretRef object, state 검증 — plaintext 0)
- [ ] pairing list/approve: exact argv, state 갱신

실패/검증:

- [ ] invalid channel id → `channel-id-invalid`, CLI 0회
- [ ] empty token → `channel-token-invalid`, CLI 0회
- [ ] 기존 external 토큰 (state에 plaintext/타 ref) → `secret-ref-registration-failed`,
  DPAPI 0, CLI 0회
- [ ] 토큰 미등록 connect → `channel-token-not-found`, CLI 0회
- [ ] invalid dmPolicy/groupPolicy/allowFrom entry → 각 stable code, CLI 0회
- [ ] dm-access cross-rule 위반 (allowlist+빈, open+무`*`) → `dm-access-inconsistent`, CLI 0회
- [ ] invalid pairing code → `pairing-code-invalid`, CLI 0회
- [ ] 토큰 없음 delete → `channel-token-not-found`
- [ ] dry-run `ok: false` → 실write 0 (state 불변) + `openclaw-config-invalid`
- [ ] missing executable → `openclaw-not-found` 재사용
- [ ] channels status/pairing nonzero/malformed → `openclaw-channels-failed` /
  `openclaw-pairing-failed`, timeout → `process-timeout`
- [ ] **전 flow argv capture에 토큰 값이 0회 포함** (masking/leak assert)

### unit 테스트

- [ ] channel config parser: field 부재 → null/빈 배열, tokenState 3분류 (managed/external/absent)
- [ ] channels list/status row, pairing row parser fail-soft (code 필수 drop)
- [ ] 검증기: channel, token, dmPolicy, groupPolicy, allowFrom entry, pairing code (정상/reject case)
- [ ] ChannelTokenService (temp store): set lifecycle, delete, orphan cleanup,
  declaration cleanup (provider+channel dual surface — Phase 3 provider key 잔존 시 declaration 유지)
- [ ] connect flow (fake port): 단계 순서, 첫 실패 중단, install idempotent (2회 실행 시 install 1회)

## 8. Real E2E (Phase 6 확장, opt-in)

Phase 2–5의 3중 게이트 구조(`--test real_e2e` + `--features real-e2e` +
`CLAWDESK_REAL_E2E=1`)를 유지한다.

- [ ] 기본 `cargo test`에서 real config mutation 0회
- [ ] 조건 불만족 시 self-skip
- [ ] 조건 충족 시에만:
  - [ ] `channels list --all --json`, `channels status --json` read-only 실행,
    출력 스키마 baseline 보고 (보고만, assert 0 — 미확인 항목 확정)
  - [ ] test-owned token round-trip: **해당 채널 토큰 미설정 + ref 부재일 때만**
    (original 기록 → `clawdesk-e2e-<timestamp>` fake token set → read-back ref 존재 확인 →
    unset → store delete → 부재 확인). user 토큰 존재 시 skip + NOT-RUN 보고
- [ ] round-trip 외 기존 user token/policy/pairing 상태 변경 0 (복원 보장)
- [ ] `plugins install` real 실행 0 (round-trip 대상 아님 — Discord install은 non-goal 실행)

## 9. 보안 / Architecture 불변식

### ProcessRunner

- [ ] production process spawn 단일 경계 유지
- [ ] 신규 직접 spawn 없음 (channels/install adapter는 모두 `ProcessPort` 경유)
- [ ] executable + argv, shell command string 0

### Secret handling (S3/S7/S8)

- [ ] 채널 토큰은 DPAPI 경유로만 저장 (config/JSON/log에 plaintext 0)
- [ ] 채널 토큰 argv 0회 (SecretRef 경유만)
- [ ] 채널 토큰 UI plaintext 0 (mask 표시만)
- [ ] CLI stdout/stderr/error masking chokepoint 재사용
- [ ] error serialization에 secret 0 (mask pipeline 경유)

### Fail-closed

- [ ] 검증 실패 (channel/token/dmPolicy/groupPolicy/allowFrom/code) → CLI 0회
- [ ] dry-run 실패 → write 0회
- [ ] 토큰 미등록 connect → error (partial connect 0)
- [ ] gateway 불능 → config 기준 표시 (연결됨 추정 0)
- [ ] plugin install 실패 → enabled write 0회
- [ ] pairing code 무효 → CLI 0회

### user environment

- [ ] `openclaw.json` 직접 파일 편집 0 (CLI 경유만)
- [ ] gateway process start/stop/restart 0
- [ ] ClawDesk 소유 경로(%APPDATA%\ClawDesk\) 외 파일 작성 0 —
  본 phase 신규 file store 없음
- [ ] `openclaw plugins update/remove` 0

### layering

- [ ] commands → application → domain → infrastructure 일방 의존 유지
- [ ] React는 invoke만 (process/executable 접근 0)

## 10. 검증 명령

Phase 6 종료 전 아래 명령을 실제 실행한다.

```text
cargo check
cargo test
cargo clippy
cargo fmt --check
pnpm typecheck
pnpm test
pnpm lint
```

모두 통과해야 한다.

real E2E는 opt-in이므로 기본 Phase 종료 검증에서 필수 실행하지 않는다.

실행하지 않은 경우:

```text
real E2E: NOT-RUN
```

이라고 명시한다.

---

# Non-Goals

이 Phase에서 구현하지 않는다.

- **기타 채널** (Slack, WhatsApp, Signal, iMessage 등 — official/external plugin 30종+)
- **multi-account** (`channels.<channel>.accounts.*`)
- **Discord guild 설정** (`guilds` map, per-guild `users`/`roles`/`channels`,
  `requireMention`, access groups)
- **Telegram group 설정** (`groups` map, `groupAllowFrom`, per-group entry, `topics`)
- **DM/group sender-scope tool policy** (`tools.toolsBySender`,
  `tools.elevated.allowFrom`, `channels.telegram.direct.*.tools`) — Phase 5에서 이월된
  항목이며 본 phase에서도 non-goal
- **webhook mode** (Telegram `webhookUrl`/`webhookSecret`/`webhook*`)
- **Discord voice, components v2, streaming, presence, PluralKit, proxy,
  mentionAliases, autoPresence, intents 세부**
- **채널 exec approvals** (`channels.*.execApprovals`)
- **`channels add/remove/login/logout` CLI** (직접 config write + plugins install 사용)
- **`channels status --probe` live probe, `capabilities`, `resolve`, `logs`,
  `dead-letters`**
- **`accessGroups`, `bindings` (agent routing)**
- **plugin update/remove** (Discord에 필요한 `@openclaw/discord` install만)
- **`doctor --fix` migration, `secrets audit/configure/apply` CLI**
- **gateway start/stop/restart/lifecycle** (Phase 8 diagnostics 영역)
- **채널 config backup/rollback**
- **실제 OpenClaw/plugin mutation을 기본 테스트에서** (S5)
- **Phase 7 (Automations) 시작**

---

# Phase 종료 보고

`ClawDesk-build` 완료 보고는 다음을 포함한다.

1. 생성/변경 파일
2. 구현 내용
3. PHASE_06.md Exit Criteria 대응표
4. 실행한 검증 명령
5. 실제 검증 결과 (passed / failed / ignored/skipped)
6. 추가 dependency와 이유
7. 실제 OpenClaw 또는 시스템에 실행한 명령
8. 기존 ProcessRunner / security boundary 유지 확인
9. 미검증 사항
10. Phase 6 Non-Goals 미구현 확인
11. Phase 7 미시작 확인

real E2E를 실행하지 않았다면 반드시:

```text
real E2E: NOT-RUN
```

으로 기록한다.

실행하지 않은 검증을 PASS라고 쓰지 않는다.

Phase 6 완료 후 Phase 7로 넘어가지 않는다.
