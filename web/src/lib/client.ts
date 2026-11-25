import * as Comlink from "comlink";
import type { Host } from "../worker";
import type { Service } from "../worker/service";
import type { AppState, Nip07Call } from "./state";
import { ANNOUNCEMENTS, DRIVE_LOCK, type HubMessage, tabLock } from "./tabs";

interface Nip07 {
  getPublicKey(): Promise<string>;
  signEvent(event: unknown): Promise<unknown>;
  nip44?: {
    encrypt(pubkey: string, plaintext: string): Promise<string>;
    decrypt(pubkey: string, ciphertext: string): Promise<string>;
  };
}

declare global {
  interface Window {
    nostr?: Nip07;
  }
}

/** The leader's service, reached through a port. */
interface Connection {
  remote: Comlink.Remote<Service>;
  /** Rejects once the page leaves this connection, which fails the calls under way. */
  closed: Promise<never>;
  close(): void;
}

const tab = crypto.randomUUID();
const host = Comlink.wrap<Host>(new Worker(new URL("../worker/index.ts", import.meta.url), { type: "module" }));
const hub =
  "SharedWorker" in window
    ? new SharedWorker(new URL("../hub/index.ts", import.meta.url), { type: "module", name: "lupine-hub" })
    : undefined;

let leading = false;
let current: Connection | undefined;
let first!: (connection: Connection) => void;
let connection = new Promise<Connection>((resolve) => (first = resolve));

/** The drive's service, in the worker of whichever tab leads. */
export const lupine = new Proxy({} as Comlink.Remote<Service>, {
  get: (_, method) =>
    method === "then"
      ? undefined
      : async (...args: unknown[]) => {
          const { remote, closed } = await connection;
          const call = (remote as unknown as Record<PropertyKey, (...args: unknown[]) => Promise<unknown>>)[method];
          return Promise.race([call(...args), closed]);
        },
});

/**
 * Follows the drive from the leader, which this tab becomes once the tabs
 * before it close. `onFailure`: the drive could not open.
 */
export function start(onState: (state: AppState) => void, onFailure: (error: unknown) => void): void {
  let shown = false;
  // A new leader starts over: the page keeps its view, folder and errors included, until it is ready.
  const show = (state: AppState) => {
    if (state.phase === "starting" && shown) return;
    shown = true;
    onState(state);
  };

  const use = (next: Connection) => {
    current?.close();
    current = next;
    first(next);
    connection = Promise.resolve(next);
    void next.remote.subscribe(Comlink.proxy(show));
    next.remote.start().catch((error: unknown) => {
      if (current === next) onFailure(error);
    });
  };

  void holdTab().then(async () => {
    if (hub) {
      const follow = () => {
        const { port1, port2 } = new MessageChannel();
        hub.port.postMessage({ type: "follow", tab, port: port1 } satisfies HubMessage, [port1]);
        use(over(port2));
      };
      new BroadcastChannel(ANNOUNCEMENTS).onmessage = () => {
        if (!leading) follow();
      };
      follow();
    } else {
      void navigator.locks?.query().then(({ held }) => {
        if (!leading && held?.some((lock) => lock.name === DRIVE_LOCK)) show({ phase: "elsewhere" });
      });
    }
    const accept = new MessageChannel();
    await host.lead(Comlink.transfer(accept.port1, [accept.port1]));
    leading = true;
    const own = new MessageChannel();
    void host.connect(Comlink.transfer(own.port1, [own.port1]), tab);
    use(over(own.port2));
    hub?.port.postMessage({ type: "lead", accept: accept.port2 } satisfies HubMessage, [accept.port2]);
  });
}

function over(port: MessagePort): Connection {
  let close!: () => void;
  const closed = new Promise<never>((_, reject) => {
    close = () => {
      port.close();
      reject(new Error("The tab that held the drive closed. Try again."));
    };
  });
  closed.catch(() => {});
  return { remote: Comlink.wrap<Service>(port), closed, close };
}

/** Held until the tab closes, before any worker serves it. */
function holdTab(): Promise<void> {
  if (!("locks" in navigator)) return Promise.resolve();
  return new Promise((resolve) => {
    void navigator.locks.request(tabLock(tab), () => {
      resolve();
      return new Promise<never>(() => {});
    });
  });
}

/** Functions the worker calls back, such as listeners and signers. */
export const callback = Comlink.proxy;

export function hasExtension(): boolean {
  return window.nostr !== undefined;
}

/** Runs the worker's NIP-07 calls in the page, where `window.nostr` lives. */
export const nip07: Nip07Call = async (method, params) => {
  const nostr = window.nostr;
  if (!nostr) throw new Error("No Nostr extension is installed.");
  const [pubkey, text] = params as [string, string];
  switch (method) {
    case "getPublicKey":
      return nostr.getPublicKey();
    case "signEvent":
      return nostr.signEvent(params[0]);
    case "nip44.encrypt":
    case "nip44.decrypt":
      if (!nostr.nip44) throw new Error("This extension does not support NIP-44 encryption.");
      return method === "nip44.encrypt" ? nostr.nip44.encrypt(pubkey, text) : nostr.nip44.decrypt(pubkey, text);
    default:
      throw new Error(`Unknown NIP-07 method ${method}`);
  }
};
