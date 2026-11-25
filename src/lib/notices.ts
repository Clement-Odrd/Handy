import type { TFunction } from "i18next";
import { platform } from "@tauri-apps/plugin-os";
import type { NoticeCode } from "@/bindings";

interface NoticeDisplay {
  title: string;
  // Without one, the Live overlay shows the notice's detail instead.
  explanation?: (t: TFunction) => string;
}

// See "Error notices" in AGENTS.md for how to add a code.
const NOTICE_DISPLAY: Record<NoticeCode, NoticeDisplay> = {
  TranscriptionFailed: {
    title: "errors.transcriptionFailedTitle",
  },
  PasteFailed: {
    title: "errors.pasteFailedTitle",
    explanation: (t) => t("errors.pasteFailed"),
  },
  PasteBlockedElevated: {
    title: "errors.pasteBlockedElevatedTitle",
    explanation: (t) => t("errors.pasteBlockedElevated"),
  },
  NoMicrophone: {
    title: "errors.noInputDeviceTitle",
    explanation: (t) => t("errors.noInputDevice"),
  },
  MicrophoneSilent: {
    title: "errors.microphoneSilentTitle",
    explanation: (t) => t("errors.microphoneSilent"),
  },
  MicPermissionDenied: {
    title: "errors.micPermissionDeniedTitle",
    explanation: (t) =>
      t(`errors.micPermissionDenied.${platform()}`, {
        defaultValue: t("errors.micPermissionDenied.generic"),
      }),
  },
  RecordingFailed: {
    title: "errors.recordingFailedTitle",
  },
  ModelLoadFailed: {
    title: "errors.modelLoadFailedTitle",
    explanation: (t) => t("errors.modelLoadFailed"),
  },
  NoModelSelected: {
    title: "errors.noModelSelectedTitle",
    explanation: (t) => t("errors.noModelSelected"),
  },
  PostProcessFailed: {
    title: "errors.postProcessFailedTitle",
    explanation: (t) => t("errors.postProcessFailed"),
  },
  FileTranscriptionFailed: {
    title: "errors.fileTranscriptionFailedTitle",
  },
  FilePostProcessFailed: {
    title: "errors.filePostProcessFailedTitle",
    explanation: (t) => t("errors.filePostProcessFailed"),
  },
  VoxtralKeyMissing: {
    title: "errors.voxtralKeyMissingTitle",
    explanation: (t) => t("errors.voxtralKeyMissing"),
  },
  VoxtralKeyRejected: {
    title: "errors.voxtralKeyRejectedTitle",
    explanation: (t) => t("errors.voxtralKeyRejected"),
  },
  VoxtralRateLimited: {
    title: "errors.voxtralRateLimitedTitle",
    explanation: (t) => t("errors.voxtralRateLimited"),
  },
  VoxtralUnreachable: {
    title: "errors.voxtralUnreachableTitle",
    explanation: (t) => t("errors.voxtralUnreachable"),
  },
  VoxtralFailed: {
    title: "errors.voxtralFailedTitle",
  },
};

export const noticeDisplay = (code: NoticeCode) => NOTICE_DISPLAY[code];
