/**
 * The event plugin of the mock backend. `mockIPC`'s own event mock keeps
 * listeners after `unlisten` (it looks up the wrong argument), so events
 * emitted after a component unmounted would hit callbacks that no longer
 * exist; this bus removes them properly.
 */

/** The part of Tauri's injected internals the bus calls back into. */
interface TauriInternals {
  runCallback(id: number, data: unknown): void;
}

/** Arguments of the event plugin commands, as sent by `@tauri-apps/api/event`. */
interface EventArgs {
  readonly event?: string;
  readonly handler?: number;
  readonly eventId?: number;
  readonly payload?: unknown;
}

/** Tauri's internals, injected by `mockIPC`. */
function internals(): TauriInternals {
  // oxlint-disable-next-line no-underscore-dangle
  return (window as unknown as { __TAURI_INTERNALS__: TauriInternals }).__TAURI_INTERNALS__;
}

/** Prefix of the event plugin commands. */
const PREFIX = "plugin:event|";

/** In-memory event routing: listeners by event name. */
export interface EventBus {
  /** Whether `cmd` belongs to the event plugin. */
  handles(cmd: string): boolean;
  /** Answers an event plugin command. */
  invoke(cmd: string, args: EventArgs): number | null;
}

/** Creates an empty event bus. Targets are ignored: every listener hears every event. */
export function createEventBus(): EventBus {
  const listeners = new Map<string, Set<number>>();

  return {
    handles: (cmd) => cmd.startsWith(PREFIX),
    invoke: (cmd, { event = "", handler, eventId, payload }) => {
      switch (cmd) {
        case `${PREFIX}listen`: {
          if (handler === undefined) throw new Error("mock backend: listen without a handler");
          const set = listeners.get(event) ?? new Set();
          set.add(handler);
          listeners.set(event, set);
          // The event id is the handler's callback id.
          return handler;
        }
        case `${PREFIX}unlisten`:
          if (eventId !== undefined) listeners.get(event)?.delete(eventId);
          return null;
        case `${PREFIX}emit`:
        case `${PREFIX}emit_to`:
          // A copy: a callback may unlisten while the event is delivered.
          for (const id of Array.from(listeners.get(event) ?? [])) {
            internals().runCallback(id, { event, id, payload });
          }
          return null;
        default:
          throw new Error(`mock backend: unknown command ${cmd}`);
      }
    },
  };
}
