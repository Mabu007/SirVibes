import { Button } from "@heroui/react";

export function ConfirmDialog({
  title,
  body,
  confirmLabel = "Delete",
  destructive = true,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: string;
  confirmLabel?: string;
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-backdrop/40 p-6 backdrop-blur-[2px]"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-sm rounded-2xl border border-border bg-background p-5 shadow-xl"
        onClick={(e) => e.stopPropagation()}
        role="alertdialog"
        aria-label={title}
      >
        <h2 className="text-base font-semibold text-foreground">{title}</h2>
        <p className="mt-1.5 text-[13.5px] leading-snug text-muted">{body}</p>
        <div className="mt-5 flex justify-end gap-2">
          <Button variant="secondary" onPress={onCancel}>
            Cancel
          </Button>
          <Button variant={destructive ? "danger" : "primary"} autoFocus onPress={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}
