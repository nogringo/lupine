/// <reference lib="webworker" />
// Serves the files of share links to the pages at its scope, decrypting only the bytes they ask for.
import init, { openSharedFile, type SharedFile } from "../../core/lupine.js";
import { message } from "../lib/format";
import { mimeType } from "../lib/media";
import { CLAIM, LINK_REQUEST, type LinkRequest, STREAM } from "./stream";

declare const self: ServiceWorkerGlobalScope;

/** A response starts with this much, so playback and seeks start fast. Later reads double, up to a chunk. */
const FIRST_WINDOW = 256 << 10;
const MAX_WINDOW = 8 << 20;
const ASK_TIMEOUT = 2000;

let ready: Promise<unknown> | undefined;
const files = new Map<string, Promise<SharedFile>>();

self.addEventListener("install", (event) => event.waitUntil(self.skipWaiting()));
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));
self.addEventListener("message", (event) => {
  if (event.data === CLAIM) event.waitUntil(self.clients.claim());
});

self.addEventListener("fetch", (event) => {
  const { request } = event;
  const prefix = new URL(STREAM, self.registration.scope).href;
  if (request.method !== "GET" || !request.url.startsWith(prefix)) return;
  const id = request.url.slice(prefix.length).split(/[/?#]/)[0];
  event.respondWith(serve(request, id));
});

async function serve(request: Request, id: string): Promise<Response> {
  let file: SharedFile;
  try {
    file = await open(id);
  } catch (error) {
    return failed(404, error);
  }
  const size = file.size;
  const range = requested(request.headers.get("Range"), size);
  if (range === "unsatisfiable") {
    return new Response(null, { status: 416, headers: { "Content-Range": `bytes */${size}` } });
  }
  const [start, end] = range ?? [0, size];
  const first = Math.min(end, start + FIRST_WINDOW);
  let head: Uint8Array<ArrayBuffer>;
  try {
    head = await file.readRange(start, first);
  } catch (error) {
    return failed(502, error);
  }
  const type = mimeType(file.name);
  const download = new URL(request.url).searchParams.has("download");
  const headers = new Headers({
    "Content-Type": type,
    "Content-Length": String(end - start),
    "Content-Disposition": `${download ? "attachment" : "inline"}; filename*=UTF-8''${rfc5987(file.name)}`,
    "Accept-Ranges": "bytes",
    "Cache-Control": "no-store",
    "X-Content-Type-Options": "nosniff",
  });
  // Opened as a page, an SVG would run its scripts on this origin, which holds the drive's secret.
  // Chrome shows no PDF in a sandbox, and its PDF viewer keeps the file's scripts from the origin.
  if (type !== "application/pdf") headers.set("Content-Security-Policy", "sandbox");
  if (range) headers.set("Content-Range", `bytes ${start}-${end - 1}/${size}`);
  return new Response(body(file, head, first, end), { status: range ? 206 : 200, headers });
}

function body(file: SharedFile, head: Uint8Array<ArrayBuffer>, from: number, end: number): ReadableStream<Uint8Array> {
  let position = from;
  let window = FIRST_WINDOW;
  return new ReadableStream({
    start(controller) {
      if (head.length > 0) controller.enqueue(head);
      if (position >= end) controller.close();
    },
    async pull(controller) {
      window = Math.min(window * 2, MAX_WINDOW);
      const next = Math.min(end, position + window);
      controller.enqueue(await file.readRange(position, next));
      position = next;
      if (position >= end) controller.close();
    },
  });
}

/** The single range a `Range` header asks for, end excluded. Any other header gets the whole file. */
function requested(header: string | null, size: number): [number, number] | "unsatisfiable" | undefined {
  const match = header?.match(/^bytes=(\d*)-(\d*)$/);
  if (!match || (match[1] === "" && match[2] === "")) return undefined;
  if (size === 0) return "unsatisfiable";
  if (match[1] === "") {
    const suffix = Number(match[2]);
    return suffix === 0 ? "unsatisfiable" : [Math.max(0, size - suffix), size];
  }
  const start = Number(match[1]);
  if (start >= size) return "unsatisfiable";
  const end = match[2] === "" ? size : Math.min(size, Number(match[2]) + 1);
  return end > start ? [start, end] : undefined;
}

function open(id: string): Promise<SharedFile> {
  let file = files.get(id);
  if (!file) {
    file = (async () => {
      const shown = await shownBy(id);
      if (!shown) throw new Error("no open page shows this file");
      ready ??= init();
      await ready;
      return openSharedFile(shown);
    })();
    files.set(id, file);
    file.catch(() => files.delete(id));
  }
  return file;
}

async function shownBy(id: string): Promise<string | undefined> {
  const scope = self.registration.scope;
  const streams = new URL(STREAM, scope).href;
  const pages = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  const asked = pages.filter((page) => page.url.startsWith(scope) && !page.url.startsWith(streams));
  const answers = await Promise.all(asked.map((page) => ask(page, id)));
  return answers.find((shown) => shown !== undefined);
}

function ask(page: Client, id: string): Promise<string | undefined> {
  return new Promise((resolve) => {
    const { port1, port2 } = new MessageChannel();
    const timer = setTimeout(() => resolve(undefined), ASK_TIMEOUT);
    port1.onmessage = ({ data }) => {
      clearTimeout(timer);
      port1.close();
      resolve(typeof data === "string" ? data : undefined);
    };
    page.postMessage({ type: LINK_REQUEST, id } satisfies LinkRequest, [port2]);
  });
}

function failed(status: number, error: unknown): Response {
  return new Response(message(error), { status, headers: { "Content-Type": "text/plain; charset=utf-8" } });
}

/** `encodeURIComponent` leaves a few characters that a `filename*` parameter may not hold. */
function rfc5987(text: string): string {
  return encodeURIComponent(text).replace(/['()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`);
}
