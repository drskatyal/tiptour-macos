// Browser-side helper that installs a fake `window.__TAURI_INTERNALS__`
// and `window.__TAURI_EVENT_PLUGIN_INTERNALS__` so Tauri's
// `@tauri-apps/api/core` and `@tauri-apps/api/event` modules behave as
// if a real Tauri runtime were attached — without ever launching one.
//
// The shape mirrors what the real runtime exposes:
//   - `invoke(cmd, args, options)`   → routed to a per-test handler map
//   - `transformCallback(fn, once)`  → stashes the callback and returns
//                                      a numeric id that `plugin:event|
//                                      listen` will reference back to
//                                      this transport when an event
//                                      fires.
//   - `unregisterCallback(id)`       → removes a stored callback.
//
// We also expose two test-only globals so specs can drive the page:
//   - `window.__mockInvokeCalls`   → array of every invoke ever made,
//                                    in order. Used to assert that the
//                                    UI dispatched the correct command
//                                    with the correct args.
//   - `window.__mockEmit(name, payload)` → push an event payload to
//                                    every listener registered against
//                                    `name`.
//
// This file is intentionally written as a stringly-injected init script
// (Playwright's `addInitScript`) so it runs *before* any module on the
// page evaluates — otherwise the modules would `await listen(...)` at
// top level against an undefined runtime and crash the page.

import type { Page } from "@playwright/test";

export interface InvokeMockSpec {
  // Map of command name → (args) => result | Promise<result>. Anything
  // not in the map returns `undefined` and records the call so the
  // spec can still assert it was attempted.
  handlers?: Record<
    string,
    (args: Record<string, unknown> | undefined) => unknown | Promise<unknown>
  >;
}

export interface MockedInvokeCall {
  cmd: string;
  args: Record<string, unknown> | undefined;
}

declare global {
  interface Window {
    __mockInvokeCalls: MockedInvokeCall[];
    __mockEmit: (eventName: string, payload: unknown) => void;
    __mockSetHandler: (
      cmd: string,
      handler: (args: Record<string, unknown> | undefined) => unknown | Promise<unknown>,
    ) => void;
    __TAURI_INTERNALS__: {
      invoke: (
        cmd: string,
        args?: Record<string, unknown>,
        options?: unknown,
      ) => Promise<unknown>;
      transformCallback: (cb: (msg: unknown) => void, once?: boolean) => number;
      unregisterCallback: (id: number) => void;
      // Used by Channel implementations; we don't rely on it but the
      // real runtime exposes it.
      ipc: (msg: unknown) => void;
    };
    __TAURI_EVENT_PLUGIN_INTERNALS__: {
      unregisterListener: (event: string, eventId: number) => void;
    };
  }
}

export async function installTauriMock(page: Page, spec: InvokeMockSpec = {}): Promise<void> {
  // Serialize handlers as their function source so they survive the
  // page boundary. Values that are plain objects/arrays/scalars are
  // converted to a constant-returning function.
  const serializedHandlers: Record<string, string> = {};
  if (spec.handlers) {
    for (const [commandName, handlerFn] of Object.entries(spec.handlers)) {
      serializedHandlers[commandName] = handlerFn.toString();
    }
  }

  await page.addInitScript(
    ({ serializedHandlers }) => {
      const callbackByCallbackId = new Map<number, (msg: unknown) => void>();
      const listenerIdToEvent = new Map<number, { event: string; callbackId: number }>();
      // event name → set of callback ids
      const callbackIdsByEventName = new Map<string, Set<number>>();
      let nextCallbackId = 1;
      let nextListenerId = 1;

      const handlerByCommandName: Record<
        string,
        (args: Record<string, unknown> | undefined) => unknown | Promise<unknown>
      > = {};
      // Rehydrate the handlers passed in via the init-script bridge.
      for (const [commandName, source] of Object.entries(serializedHandlers)) {
        // eslint-disable-next-line no-new-func
        handlerByCommandName[commandName] = new Function(
          "return (" + source + ")",
        )() as (args: Record<string, unknown> | undefined) => unknown | Promise<unknown>;
      }

      window.__mockInvokeCalls = [];

      window.__mockSetHandler = (commandName, handler) => {
        handlerByCommandName[commandName] = handler;
      };

      window.__mockEmit = (eventName, payload) => {
        const callbackIds = callbackIdsByEventName.get(eventName);
        if (!callbackIds) return;
        for (const callbackId of callbackIds) {
          const callback = callbackByCallbackId.get(callbackId);
          if (callback) {
            // Tauri delivers events as `{event, id, payload}` — match
            // that shape so the runtime-side `listen` callback receives
            // exactly what it would in production.
            callback({ event: eventName, id: callbackId, payload });
          }
        }
      };

      window.__TAURI_INTERNALS__ = {
        async invoke(cmd, args) {
          window.__mockInvokeCalls.push({ cmd, args });
          // Built-in event-plugin commands — these are how `listen`
          // and `unlisten` actually wire up to a real runtime.
          if (cmd === "plugin:event|listen") {
            const eventName = (args as { event: string }).event;
            const callbackId = (args as { handler: number }).handler;
            const listenerId = nextListenerId++;
            listenerIdToEvent.set(listenerId, { event: eventName, callbackId });
            let bucket = callbackIdsByEventName.get(eventName);
            if (!bucket) {
              bucket = new Set();
              callbackIdsByEventName.set(eventName, bucket);
            }
            bucket.add(callbackId);
            return listenerId;
          }
          if (cmd === "plugin:event|unlisten") {
            const eventName = (args as { event: string }).event;
            const listenerId = (args as { eventId: number }).eventId;
            const entry = listenerIdToEvent.get(listenerId);
            if (entry) {
              callbackIdsByEventName.get(entry.event)?.delete(entry.callbackId);
              listenerIdToEvent.delete(listenerId);
            }
            return undefined;
          }
          if (cmd === "plugin:event|emit" || cmd === "plugin:event|emit_to") {
            // Local-frontend emits — bounce them straight back through
            // the listen callback table so the page can drive itself.
            const eventName = (args as { event: string }).event;
            const payload = (args as { payload?: unknown }).payload;
            window.__mockEmit(eventName, payload);
            return undefined;
          }
          const handler = handlerByCommandName[cmd];
          if (!handler) return undefined;
          return await handler(args as Record<string, unknown> | undefined);
        },
        transformCallback(callback) {
          const callbackId = nextCallbackId++;
          callbackByCallbackId.set(callbackId, callback);
          return callbackId;
        },
        unregisterCallback(callbackId) {
          callbackByCallbackId.delete(callbackId);
        },
        ipc() {
          // not used in our tests
        },
      };

      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener(eventName, listenerCallbackId) {
          callbackIdsByEventName.get(eventName)?.delete(listenerCallbackId);
        },
      };
    },
    { serializedHandlers },
  );
}
