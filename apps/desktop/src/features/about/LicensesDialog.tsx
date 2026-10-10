import { ChevronRight, ExternalLink } from "lucide-react";
import { useDeferredValue, useId, useState, type ComponentProps, type ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { formatCount } from "@/lib/media";
import { cn } from "@/lib/utils";

import {
  filterComponents,
  KIND_TITLES,
  type ComponentKind,
  type LicenseData,
  type LicensedComponent,
} from "./licenses";
import { openProjectPage, REPOSITORY_URL } from "./links";
import { useLicenses } from "./useLicenses";

const KINDS = Object.keys(KIND_TITLES) as readonly ComponentKind[];

/** One component: a disclosure button and, when open, its notes and licence texts. */
function ComponentRow({
  item,
  data,
}: {
  readonly item: LicensedComponent;
  readonly data: LicenseData;
}) {
  const [open, setOpen] = useState(false);
  const panelId = useId();

  return (
    <li className="border-t border-border first:border-t-0">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        className="flex min-h-9 w-full cursor-default items-center gap-2 px-3 py-1.5 text-left text-ui-base transition-colors hover:bg-hover"
        onClick={() => setOpen((value) => !value)}
      >
        <ChevronRight
          aria-hidden
          className={cn(
            "size-3.5 shrink-0 text-foreground-subtle transition-transform",
            open && "rotate-90",
          )}
        />
        <span className="min-w-0 flex-1 truncate font-medium">{item.name}</span>
        <span className="shrink-0 font-mono text-ui-sm text-foreground-subtle">{item.version}</span>
        <Badge variant="outline" className="max-w-56">
          <span className="truncate">{item.license}</span>
        </Badge>
      </button>
      {open && (
        <div id={panelId} className="flex flex-col gap-3 px-3 pt-1 pb-3 pl-8">
          <p className="text-ui-sm break-all text-foreground-subtle">{item.url}</p>
          {item.note !== undefined && <p className="text-ui-sm">{item.note}</p>}
          {item.source !== undefined && (
            <SourceLink url={item.source}>Download the corresponding source code</SourceLink>
          )}
          {item.texts.map((index) => {
            const entry = data.texts[index];
            if (!entry) return null;
            return (
              <section key={index} aria-label={entry.license} className="flex flex-col gap-1">
                <h4 className="text-ui-sm font-medium text-foreground-subtle">{entry.license}</h4>
                <pre
                  // Long texts scroll inside the row; keyboard users reach them.
                  // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex
                  tabIndex={0}
                  className="max-h-72 overflow-auto rounded-lg border border-border bg-surface p-3 font-mono text-ui-xs whitespace-pre-wrap"
                >
                  {entry.text}
                </pre>
              </section>
            );
          })}
        </div>
      )}
    </li>
  );
}

/** A link to a page of the project, opened in the default browser. */
export function SourceLink({
  url,
  children,
}: {
  readonly url: string;
  readonly children: ReactNode;
}) {
  // Only project pages may be opened (capability scope); other URLs are text.
  if (!url.startsWith(REPOSITORY_URL)) return <span className="text-ui-sm break-all">{url}</span>;
  return (
    <Button
      variant="link"
      size="sm"
      className="h-auto self-start px-0"
      onClick={() => void openProjectPage(url)}
    >
      {children}
      <ExternalLink aria-hidden />
    </Button>
  );
}

/** The searchable list, grouped by kind. */
function LicenseList({ data }: { readonly data: LicenseData }) {
  const [query, setQuery] = useState("");
  const deferred = useDeferredValue(query);
  const shown = filterComponents(data.packages, deferred);
  const searchId = useId();

  return (
    <>
      <Input
        id={searchId}
        type="search"
        size="lg"
        aria-label="Search the components"
        placeholder="Search by name, version or licence"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
      />
      <p aria-live="polite" className="sr-only">
        {formatCount(shown.length)} components shown
      </p>
      <div
        // The list scrolls; keyboard users reach it with Tab.
        // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex
        tabIndex={-1}
        className="-mx-1 min-h-0 flex-1 overflow-y-auto px-1 outline-none [scrollbar-gutter:stable]"
      >
        {shown.length === 0 && (
          <p className="py-8 text-center text-ui-base text-foreground-subtle">
            No component matches “{deferred}”.
          </p>
        )}
        {KINDS.map((kind) => {
          const items = shown.filter((item) => item.kind === kind);
          if (items.length === 0) return null;
          return (
            <section
              key={kind}
              aria-label={KIND_TITLES[kind]}
              className="mb-4 flex flex-col gap-1.5"
            >
              <h3 className="text-ui-sm font-medium text-foreground-subtle">
                {KIND_TITLES[kind]} ({formatCount(items.length)})
              </h3>
              <ul className="overflow-hidden rounded-xl border border-card-border bg-card">
                {items.map((item) => (
                  <ComponentRow key={`${item.name}@${item.version}`} item={item} data={data} />
                ))}
              </ul>
            </section>
          );
        })}
      </div>
    </>
  );
}

/** Loading and error states around {@link LicenseList}. */
function LicenseBody() {
  const licenses = useLicenses();
  if (licenses.isPending) {
    return (
      <div className="flex flex-1 items-center justify-center gap-2 text-ui-base text-foreground-subtle">
        <Spinner label="Loading licences" />
        <span aria-hidden>Loading licences…</span>
      </div>
    );
  }
  if (licenses.isError) {
    return (
      <p role="alert" className="text-ui-base text-destructive">
        The licence list could not be loaded: {licenses.error.message}
      </p>
    );
  }
  return <LicenseList data={licenses.data} />;
}

export interface LicensesDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** An element that opens the dialog and gets the focus back when it closes. */
  readonly trigger?: ReactNode;
  /** Handlers for dialogs without a trigger (see `useReturnFocus`). */
  readonly contentProps?: Pick<
    ComponentProps<typeof DialogContent>,
    "onOpenAutoFocus" | "onCloseAutoFocus"
  >;
}

/**
 * About › Open-source licenses (design 8.6): every third-party component
 * blurrd ships (bundled programs, OCR models, Rust crates, npm packages)
 * with version, licence and the full licence texts, searchable. The data
 * loads when the dialog first opens.
 */
export function LicensesDialog({ open, onOpenChange, trigger, contentProps }: LicensesDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {trigger !== undefined && <DialogTrigger asChild>{trigger}</DialogTrigger>}
      <DialogContent
        className="flex h-[calc(100%-4rem)] max-h-224 flex-col gap-3 sm:max-w-3xl"
        {...contentProps}
      >
        <DialogHeader>
          <DialogTitle>Open-source licenses</DialogTitle>
          <DialogDescription>
            blurrd ships these third-party components. Open one to read its licence.
          </DialogDescription>
        </DialogHeader>
        <LicenseBody />
      </DialogContent>
    </Dialog>
  );
}
