import { toast as sonner, type ExternalToast } from "sonner";
import { engineErrorMessage } from "./engine";
import { errorSummary } from "./errorAdvice";
import { t } from "./i18n";
import { cn } from "./utils";

type Kind = "success" | "info" | "warning" | "error";
type Title = Parameters<typeof sonner.success>[0];

function show(kind: Kind, title: Title, options: ExternalToast = {}) {
  return sonner[kind](title, {
    duration: kind === "error" || kind === "warning" || options.action ? 10000 : 4000,
    closeButton: true,
    position: document.querySelector('[role="dialog"][aria-modal="true"]') ? "top-right" : "bottom-right",
    ...options,
    classNames: {
      ...options.classNames,
      content: cn(options.classNames?.content, "min-w-0"),
      title: cn(options.classNames?.title, "max-h-20 overflow-y-auto whitespace-pre-wrap break-words"),
      description: cn(options.classNames?.description, "max-h-32 overflow-y-auto whitespace-pre-line break-words"),
      icon: cn(options.classNames?.icon, "shrink-0"),
    },
  });
}

/** One presentation policy for action feedback; reports and form errors stay local. */
export const toast = {
  success: (title: Title, options?: ExternalToast) => show("success", title, options),
  info: (title: Title, options?: ExternalToast) => show("info", title, options),
  warning: (title: Title, options?: ExternalToast) => show("warning", title, options),
  error: (title: Title, options?: ExternalToast) => show("error", title, options),
  dismiss: sonner.dismiss,
};

/** Keep the action title short and retain the diagnostic alongside any advice. */
export function notifyError(title: string, reason: unknown, options?: ExternalToast) {
  const message = engineErrorMessage(reason);
  const summary = errorSummary(message);
  return toast.error(title, {
    description: [summary.text, summary.description, summary.text !== message ? t("详细信息：{0}", message) : null].filter(Boolean).join("\n"),
    ...options,
  });
}
