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
} as const;

export type KoStrings = typeof ko;
export type Namespace = keyof KoStrings;
export type KeyOf<N extends Namespace> = keyof KoStrings[N];

export function getStrings<N extends Namespace>(ns: N): KoStrings[N] {
  return ko[ns];
}
