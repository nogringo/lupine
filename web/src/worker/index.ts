import * as Comlink from "comlink";
import { DRIVE_LOCK, type Follower, tabLock } from "../lib/tabs";
import { type Listener, Service, unsupported } from "./service";

const service = new Service();

const host = {
  /** Resolves once this worker holds the drive. It then serves the followers `accept` brings. */
  async lead(accept: MessagePort): Promise<void> {
    if (!unsupported()) await holdDrive();
    accept.onmessage = ({ data }: MessageEvent<Follower>) => serve(data.port, data.tab);
  },

  /** Serves a tab's page through `port`, until that tab closes. */
  connect: serve,
};

export type Host = typeof host;

Comlink.expose(host);

function serve(port: MessagePort, tab: string): void {
  const listeners = new Set<Listener>();
  const client = new Proxy(service, {
    get(target, key) {
      if (key === "subscribe") {
        return (listener: Listener) => {
          listeners.add(listener);
          target.subscribe(listener);
        };
      }
      const value: unknown = Reflect.get(target, key);
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
  Comlink.expose(client, port);
  if (!("locks" in navigator)) return;
  // Calling a closed tab's listeners would leave each call pending.
  void navigator.locks.request(tabLock(tab), () => {
    for (const listener of listeners) service.unsubscribe(listener);
    port.close();
  });
}

/** Held for the life of the worker: OPFS sync access handles are exclusive. */
function holdDrive(): Promise<void> {
  return new Promise((resolve) => {
    void navigator.locks.request(DRIVE_LOCK, () => {
      resolve();
      return new Promise<never>(() => {});
    });
  });
}
