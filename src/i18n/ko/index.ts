export const ko = {
  app: {
    title: "ClawDesk",
    subtitle: "OpenClaw 데스크톱 매니저",
  },
  common: {
    ok: "확인",
    cancel: "취소",
    close: "닫기",
    save: "저장",
    delete: "삭제",
    loading: "불러오는 중...",
    error: "오류가 발생했습니다.",
    retry: "다시 시도",
    notFound: "찾을 수 없습니다.",
    unsupported: "지원되지 않는 환경입니다.",
  },
  install: {
    title: "OpenClaw 준비",
    detecting: "환경을 확인하는 중...",
    openclawLabel: "OpenClaw",
    nodeLabel: "Node.js",
    openclawNotInstalled: "미설치",
    openclawInstalled: "설치됨",
    versionUnknown: "버전 미확인",
    nodeNotInstalled: "Node.js가 설치되어 있지 않습니다. Node.js(22.22.3 이상)를 설치한 후 다시 시도해 주세요.",
    nodeUnsupported: "지원되지 않는 Node.js 버전입니다. (지원: 22.22.3+, 24.15+, 25.9+, 26+)",
    nodeVersion: "Node.js 버전",
    installButton: "OpenClaw 설치",
    installing: "OpenClaw 설치 중입니다... (최대 15분까지 소요될 수 있습니다)",
    installed: "OpenClaw가 설치되었습니다.",
    alreadyInstalled: "OpenClaw가 이미 설치되어 있습니다.",
    version: "버전",
    retry: "다시 시도",
    errors: {
      "node-not-found":
        "이 기기에 Node.js가 설치되어 있지 않습니다. Node.js(22.22.3 이상)를 설치한 후 다시 시도해 주세요.",
      "unsupported-node-version":
        "현재 Node.js 버전은 지원되지 않습니다. 지원되는 버전(22.22.3+, 24.15+, 25.9+, 26+)이 필요합니다.",
      "npm-not-found":
        "Node.js 설치와 함께 npm을 찾을 수 없습니다. Node.js 설치를 복구하거나 다시 설치해 주세요.",
      "unsupported-npm-version":
        "현재 npm 버전(11.13~11.15)으로는 OpenClaw를 설치할 수 없습니다. npm 버전을 조정해 주세요.",
      "openclaw-install-failed":
        "OpenClaw 설치에 실패했습니다. 네트워크 연결을 확인한 후 다시 시도해 주세요.",
      "openclaw-install-verify-failed":
        "OpenClaw 설치 후 검증에 실패했습니다. 다시 시도해 주세요.",
      "process-timeout": "작업이 시간 내로 완료되지 않았습니다. 다시 시도해 주세요.",
      fallback: "설치 중 오류가 발생했습니다. 다시 시도해 주세요.",
    },
  },
  models: {
    title: "모델 및 Provider",
    providers: "Provider 목록",
    noProviders: "등록된 provider가 없습니다.",
    addProvider: "Provider 추가",
    editProvider: "Provider 편집",
    providerId: "Provider ID",
    providerIdHint: "영문/숫자로 시작, 128자 이하 (., _, - 허용)",
    baseUrl: "Base URL",
    baseUrlHint: "http(s) 절대 URL (선택)",
    apiType: "API 유형",
    modelCount: "모델 수",
    addModel: "모델 추가",
    modelId: "모델 ID",
    modelName: "이름",
    modelReasoning: "추론(reasoning) 지원",
    modelInput: "입력 유형",
    contextWindow: "컨텍스트 윈도우",
    maxTokens: "최대 토큰",
    supportsEffort: "Reasoning effort 지원",
    supportedEfforts: "지원 effort",
    deleteProviderConfirm: "이 provider와 함께 등록된 API key도 삭제됩니다. 계속할까요?",
    defaultModel: "기본 모델",
    defaultModelNone: "기본 모델이 설정되어 있지 않습니다.",
    setDefault: "기본 모델로 설정",
    reasoning: "Reasoning 기본값",
    reasoningNone: "기본값이 설정되어 있지 않습니다. (OpenClaw 기본 동작 적용)",
    reasoningDisabledNote: "현재 기본 모델이 reasoning을 지원하지 않아 effort 선택을 사용할 수 없습니다.",
    apiKey: "API Key",
    apiKeyRegistered: "등록됨",
    apiKeyUnregistered: "미등록",
    apiKeyExternal: "외부에서 관리 중 (ClawDesk로는 변경할 수 없음)",
    apiKeyRegister: "API Key 등록/변경",
    apiKeyRegisterHint: "값은 OS 보안 저장소(DPAPI)에만 저장되며 한 번 저장하면 다시 표시되지 않습니다.",
    apiKeyDeleteConfirm: "이 provider의 API key를 삭제할까요?",
    saving: "저장 중...",
    deleting: "삭제 중...",
    loading: "불러오는 중...",
    errors: {
      "provider-id-invalid": "Provider ID나 Base URL 형식이 올바르지 않습니다.",
      "model-id-invalid": "모델 ID나 모델 참조 형식이 올바르지 않습니다.",
      "thinking-level-invalid": "선택한 reasoning effort 값이 유효하지 않습니다.",
      "openclaw-config-read-failed": "OpenClaw 설정을 읽는 데 실패했습니다. OpenClaw 설치 상태를 확인해 주세요.",
      "openclaw-config-write-failed": "OpenClaw 설정 저장에 실패했습니다. 다시 시도해 주세요.",
      "openclaw-config-invalid": "OpenClaw가 요청한 설정을 거부했습니다. 입력값을 확인해 주세요.",
      "secret-store-unavailable": "보안 저장소(DPAPI)에 접근할 수 없습니다. API key를 저장할 수 없습니다.",
      "secret-ref-registration-failed": "API key 연결 등록에 실패했습니다. 다시 시도해 주세요.",
      "openclaw-not-found": "OpenClaw를 찾을 수 없습니다. 먼저 OpenClaw를 설치해 주세요.",
      "process-timeout": "작업이 시간 내로 완료되지 않았습니다. 다시 시도해 주세요.",
      "process-failed": "작업 실행에 실패했습니다. 다시 시도해 주세요.",
      fallback: "오류가 발생했습니다. 다시 시도해 주세요.",
    },
  },
} as const;

export type KoStrings = typeof ko;
export type Namespace = keyof KoStrings;
export type KeyOf<N extends Namespace> = keyof KoStrings[N];

export function getStrings<N extends Namespace>(ns: N): KoStrings[N] {
  return ko[ns];
}
