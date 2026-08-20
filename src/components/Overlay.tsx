import type { ReactNode } from "react";
import { Button } from "@heroui/react";
import { XIcon } from "./Icons";

export function Overlay({
  title,
  subtitle,
  onClose,
  children,
  footer,
  width = "max-w-lg",
}: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: string;
}) {
  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-backdrop/40 p-6 backdrop-blur-[2px]"
      onClick={onClose}
    >
      <div
        className={`flex max-h-[88vh] w-full ${width} flex-col overflow-hidden rounded-2xl border border-border bg-background shadow-xl`}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={title}
      >
        <div className="flex items-start gap-3 px-5 pt-5 pb-3">
          <div className="min-w-0 flex-1">
            <h2 className="text-base font-semibold text-foreground">{title}</h2>
            {subtitle && <p className="mt-0.5 text-[13px] text-muted">{subtitle}</p>}
          </div>
          <Button variant="ghost" size="sm" isIconOnly aria-label="Close" onPress={onClose}>
            <XIcon />
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-4">{children}</div>

        {footer && (
          <div className="flex justify-end gap-2 border-t border-border px-5 py-3.5">{footer}</div>
        )}
      </div>
    </div>
  );
}
