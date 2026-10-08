import { CircleAlert, CircleCheck, Info, TriangleAlert } from "lucide-react";
import { Toaster as Sonner, type ToasterProps } from "sonner";

import { Spinner } from "./spinner";

/**
 * The toast stack, mounted once at the root; show toasts with `toast()`
 * from `sonner`. Frosted shell (toast colour at 60 % with a 24 px backdrop
 * blur), 16 px radius, large shadow, 64 px from the top of the window.
 */
export function Toaster({ theme = "system", ...props }: ToasterProps) {
  return (
    <Sonner
      theme={theme}
      position="top-center"
      offset={64}
      gap={8}
      icons={{
        success: <CircleCheck className="size-4 text-success" />,
        info: <Info className="size-4 text-foreground-subtle" />,
        warning: <TriangleAlert className="size-4 text-warning" />,
        error: <CircleAlert className="size-4 text-destructive" />,
        loading: <Spinner className="text-foreground-subtle" />,
      }}
      toastOptions={{
        unstyled: true,
        classNames: {
          toast:
            "flex w-(--width) items-start gap-3 rounded-2xl border border-border bg-toast/60 px-4 py-3 text-ui-base text-foreground shadow-lg backdrop-blur-xl",
          icon: "mt-0.5 shrink-0",
          content: "flex min-w-0 flex-1 flex-col gap-0.5",
          title: "font-medium",
          description: "text-ui-base/5 text-foreground-subtle",
          actionButton:
            "ml-auto inline-flex h-7 shrink-0 items-center rounded-md bg-secondary px-2 text-ui-base text-secondary-foreground transition-colors hover:bg-secondary/80",
          cancelButton:
            "inline-flex h-7 shrink-0 items-center rounded-md px-2 text-ui-base text-foreground-subtle transition-colors hover:bg-hover",
        },
      }}
      {...props}
    />
  );
}
