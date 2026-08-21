# Phase 3.1 — Phase 3 Review Revision

Status: completed (2026-08-21)

## 목표

Phase 3 검증(`clawdesk-review`)에서 FAIL 판정된 4건(F1-F4)을 수정하고, 그에 수반되는
테스트/문서 정비를 닫는다. 이 phase는 **Phase 3 구현의 재설계가 아니라 계약
(`PHASE_03.md`) 정합성 회복**이다.

근거: Phase 3 검증 보고 (2026-08-21) — `cargo check/test/clippy`, `pnpm
typecheck/lint/test` 7개 검증 명령은 전부 통과, FAIL은 계약 문구 대비 설계 편차 4건.

| # | severity | 내용 |
| --- | --- | --- |
| F1 | MAJOR | `set_api_key` 순서가 계약 life-cycle(DPAPI → declaration → SecretRef)과 반대 (DPAPI last) |
| F2 | MAJOR | DPAPI 실패 시 config write가 이미 발생 (계약: 0회) + dangling ref 파생 delete edge case |
| F3 | MINOR | fake CLI가 protected path 규칙 미모방 (`--replace` 없는 entry 제거 거부 없음) |
| F4 | MINOR | provider update가 계약의 whole-entry `--replace`가 아님 (subpath) — 계약 문서로 공식화 필요 |

---

# Exit Criteria

## 1. F1/F2 — API key life-cycle 순서 (MAJOR)

### set_api_key 재배열 (계약 `PHASE_03.md:146-153` 순서로)

현재 `application/services/api_key.rs:82-125`는
`declaration(:103) → SecretRef write(:110-117) → DPAPI last(:118-120)` 순서다.
아래 순서로 재배열한다:

```text
set-provider-api-key:
  1. validate_provider_id + 비어있지 않은 key 확인 (write 0, 실행 0)
  2. provider 존재 확인 (config get) + externally-managed(Other) 거부
  3. DPAPI set (SecretStore, key id = providers/<id>/apiKey)   ← 가장 먼저 mutation
  4. secrets.providers.clawdesk declaration 보장 (idempotent)  (dry-run → write)
  5. models.providers.<id>.apiKey = SecretRef write            (dry-run → write)
```

- [ ] 3단계 DPAPI 실패 → **config write 0회** + `secret-store-unavailable`,
      store에 값 0
- [ ] 4/5단계 실패 → `secret-ref-registration-failed`, store entry는 남는다
      (orphan — 아래 delete cleanup이 처리)
- [ ] 파일头部 doc comment(`api_key.rs:1-12`)와 `:118` `DPAPI last` 주석을
      새 순서로 수정 (현재 주석은 부정확해짐)

### delete_api_key orphan cleanup (F2 파생 상태 처리)

재배열 후 등장하는 신규 상태: **store entry 있음 + config ref 없음(orphan)**.
`delete_api_key`(`api_key.rs:124-157`)에 최소 cleanup 분기를 추가한다:

```text
delete-provider-api-key:
  1. ref == Managed        → unset ref → store delete → (마지막이면) declaration unset (현재 유지)
  2. ref != Managed + store entry 있음 → store delete → (managed key가 남지 않으면)
                            declaration unset, apiKey subpath write 0회, Ok(())   ← 신규 (orphan cleanup)
  3. ref != Managed + store entry 없음 → read-failed (현재 유지)
```

- [ ] case 2: apiKey subpath write 0회, store entry 삭제, managed key가 남지
      않으면 declaration 제거
- [ ] case 3 기존 동작 유지

### 테스트 수정/신규 (`services/tests_api_key.rs`)

- [ ] `set_key_dpapi_failure_is_fail_safe` **반전**: ref write 발생 assert를
      제거하고 **config log 0행(write 0회) + store empty +
      `secret-store-unavailable`** assert
- [ ] 신규: set 성공 시 config log 순서 = `read-providers` →
      `read-raw:secrets.providers.clawdesk` → (신규면) `write:secrets.providers.clawdesk:plain`
      → `write:models.providers.<id>.apiKey:plain` + store에 값 존재
      (DPAPI가 config write보다 선행함을 log 빈칸이 보장 — store spy 필요 시
      `FakeStore`에 호출 log 필드 추가 허용)
- [ ] 신규: orphan cleanup (ref Absent + store entry → delete Ok, config write 0)
- [ ] 기존 `delete_key_without_managed_ref_is_read_failed`를 case 3
      (store entry 없음) 시나리오로 재정의
- [ ] 나머지 기존 api_key 테스트(시퀀스, idempotent, stale declaration,
      external key, invalid id) 통과 유지

## 2. F3 — fake CLI protected path 모방 (MINOR)

`fixtures/fake-openclaw/main.rs` `config set` 핸들러(`:401` 부근)에 protected path
거부 규칙을 추가한다.

- [ ] protected path: `models.providers`, `models.providers.<id>`,
      `models.providers.<id>.models`
- [ ] 규칙: protected path 대상에 기존 object/array가 존재하고, new JSON이 기존
      key/entry를 **제거**하는 replacement인데 `--replace`가 없으면
      `{"ok":false,...,"errors":[{"kind":"protected-path","message":...}]}`
      envelope + state 불변 (dry-run 포함, exit 0 — 기존 dry-run 의미와 동일)
- [ ] `--replace` 있으면 허용 (현재 동작 유지), `--merge`은 제거를 일으키지 않아
      기존 merge 시뮬레이션 유지
- [ ] 신규 contract case (`tests/models_contract.rs`):
  - `models.providers` 대상에서 기존 entry를 떨어뜨리는 set + `--merge`/무플래그
    → `ok:false` 거부, state 불변
  - 동일 payload + `--replace` → 적용
- [ ] 기존 `adapter_config_lifecycle_over_fake_cli` 12 sub-scenario 회귀 0
  (adapter는 항상 명시 플래그를 보내므로 동작 변화 없어야 함)

## 3. F4 — PHASE_03.md 문서 개정 (MINOR)

`PHASE_03.md`를 현 구현(검증됨)과 일치시킨다. **구현을 다시 바꾸지 않는다.**

- [ ] §1 "provider 수정 (update)" 조항 개정: whole-entry `--replace` 문구 대신
      subpath 방식을 공식화 + 근거 1문단:
      "redacted read에는 실제 apiKey가 없으므로 whole-entry `--replace`는
      등록된 SecretRef를 삭제한다(S7 충돌). 따라서 UI 편집 필드
      (`baseUrl`/`api` Plain, `models` `--replace`)만 subpath로 쓰고,
      provider entry 자체는 재작성하지 않는다. 계약 의도(기존 필드 보존,
      다른 provider 무영향)는 더 강하게 충족한다."
- [ ] §2 command 표에 `get-default-model` (Rust `get_default_model`,
      → `modelRef | null`) 행 추가 — UI "기본 모델 표시"(`PHASE_03.md:319`)에
      필요한 읽기 command (Phase 3에서 12번째로 구현됨)
- [ ] Phase 3.1 완료 시: `PHASE_03.md` Status `not started` → `completed
      (2026-08-21, review revision: PHASE_03_1.md)`, `ROADMAP.md` Phase 3
      `not started` → `completed`, ROADMAP에 3.1 행 추가 후 완료 시 `completed`
      갱신

## 4. 계약 테스트 보강 — API key argv capture

검증 보고 권고 사항: `PHASE_03.md:398` "key 등록 시 argv에 key 값 0 포함
(capture 검증)"을 경계 합성 단위가 아닌 **contract 레벨**로 검증한다.

- [ ] `tests/models_contract.rs` 신규 테스트:
  - `ApiKeyService`를 (fake `OpenClawPort` detect → fake CLI 경로, real
    `OpenClawConfigAdapter`, real `SecretStore`(sandbox root,
    `WindowsDpapi`), real resolver exe 경로)로 조립
  - `set_api_key` 호출 후 `capture.jsonl` assert:
    - 모든 argv row에 raw key plaintext 0 포함
    - declaration write(`secrets.providers.clawdesk`) + ref write
      (`models.providers.<id>.apiKey`) 1회씩, ref body = SecretRef object
  - `state()["models"]["providers"][<id>]["apiKey"]` = SecretRef
    (plaintext 아님), sandbox store에 값 저장 확인
- [ ] sandbox/`GlobalEnvGuard` 패턴 재사용 (기존 scenario 헬퍼와 동일)

---

# 5. 보안 / Architecture 불변식 (Phase 3 동일, 재확인)

- [ ] structured `executable + argv`만 (S1), shell 0
- [ ] key plaintext 저장 0 (DPAPI blob만, S7), argv/로그/UI/테스트 출력 0 (S3/S8)
- [ ] 기본 `cargo test`에서 real OpenClaw mutation 0 (S5)
- [ ] layering/React-invoke 불변식 유지

# 6. 검증 명령

```text
cargo check
cargo test
cargo clippy
cargo fmt --check
pnpm typecheck
pnpm test
pnpm lint
```

모두 통과해야 한다. Phase 3.1은 frontend 수정을 포함하지 않으므로
pnpm 결과는 회귀 확인(변화 0)이다.

real E2E는 opt-in이므로 기본 종료 검증에서 실행하지 않는다.
실행하지 않으면 `real E2E: NOT-RUN`으로 명시한다.

# Non-Goals

- Phase 3 product flow의 재설계 (이 문서의 4개 수정 외 변경 0)
- provider update 방식을 whole-entry `--replace`로 역회귀 (F4는 문서 개정)
- fake CLI의 추가 시뮬레이션 (protected path 외 — timeout/부분 출력 등)
- new IPC command, new dependency
- Phase 4(Skills/Plugins) 시작
- real E2E 실행 (opt-in 유지)

# Phase 종료 보고

`clawdesk-build` 완료 보고 형식(7개 항목) + `real E2E: NOT-RUN` 명시 +
"Phase 4 미시작 확인".
