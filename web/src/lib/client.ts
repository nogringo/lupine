import * as Comlink from "comlink";
import type { Service } from "../worker/service";
import type { Nip07Call } from "./state";

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

const worker = new Worker(new URL("../worker/index.ts", import.meta.url), { type: "module" });

export const lupine = Comlink.wrap<Service>(worker);

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

/** Hands a downloaded file to the browser, which saves it. */
export function save(file: File): void {
  const url = URL.createObjectURL(file);
  const link = document.createElement("a");
  link.href = url;
  link.download = file.name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

/** Shows a downloaded file in a new tab, when the browser can. */
export function show(file: File): void {
  const url = URL.createObjectURL(file);
  window.open(url, "_blank", "noopener");
  setTimeout(() => URL.revokeObjectURL(url), 10 * 60_000);
}
