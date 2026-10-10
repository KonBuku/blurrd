/**
 * The notification plugin's web side for the mock backend. In the app,
 * the plugin replaces `window.Notification` with a function that sends
 * `plugin:notification|notify` and reports the permission as granted on
 * Windows; this does the same, so `@tauri-apps/plugin-notification`
 * reaches the mocked `invoke` in a plain browser too.
 */

/** The part of Tauri's injected internals the shim calls. */
interface TauriInternals {
  invoke(cmd: string, args: unknown): Promise<unknown>;
}

function internals(): TauriInternals {
  // oxlint-disable-next-line no-underscore-dangle
  return (window as unknown as { __TAURI_INTERNALS__: TauriInternals }).__TAURI_INTERNALS__;
}

/** Replaces `window.Notification` with the plugin's shape. */
export function installNotificationShim(): void {
  const shim = Object.assign(
    function MockNotification(title: string, options: NotificationOptions = {}) {
      void internals().invoke("plugin:notification|notify", { options: { ...options, title } });
    },
    {
      permission: "granted" as NotificationPermission,
      requestPermission: () => Promise.resolve("granted" as NotificationPermission),
    },
  );
  Object.defineProperty(window, "Notification", {
    value: shim,
    configurable: true,
    writable: true,
  });
}
