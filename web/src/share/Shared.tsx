import { useEffect, useState } from "react";
import init, { openShareLink } from "../../core/lupine.js";
import { bytes, message } from "../lib/format";
import { mimeType, viewable } from "../lib/media";
import { Mark } from "../ui/Flower";
import { Media, UNSUPPORTED } from "../ui/Media";
import { CLAIM, LINK_REQUEST, type LinkRequest, STREAM } from "./stream";
import "../ui/styles.css";

/** Without a service worker, the page holds the whole file in memory, a few times over while it decrypts. */
const MEMORY_LIMIT = 256 << 20;
const CONTROL_TIMEOUT = 5000;

interface State {
  name?: string;
  size?: number;
  /** Bytes read so far, when the file is read into memory. */
  done?: number;
  url?: string;
  /** Where the Download button points. */
  download?: string;
  error?: string;
}

let started = false;

/** Shows the file a share link names. The link's fragment holds its keys, and never reaches the web server. */
export function Shared() {
  const [state, setState] = useState<State>({});

  useEffect(() => {
    if (started) return;
    started = true;
    addEventListener("hashchange", () => location.reload());
    const update = (change: Partial<State>) => setState((s) => ({ ...s, ...change }));
    open(location.hash.slice(1), update).catch((error: unknown) => update({ error: message(error) }));
  }, []);

  const { name, size = 0, done, url, download, error } = state;
  const media = name ? viewable(name) : undefined;

  return (
    <div className="shared">
      <header className="topbar">
        <Mark />
        <h1>{name ?? "Lupine"}</h1>
        {download ? (
          <a className="button" href={download} download={name}>
            Download
          </a>
        ) : (
          <button className="button" disabled>
            Download
          </button>
        )}
      </header>
      <main className="viewer-stage" aria-busy={!url && !error}>
        {error ? (
          <p className="viewer-note">{error}</p>
        ) : done !== undefined && !url ? (
          <div className="viewer-note">
            <progress max={size || 1} value={done} />
            <small>
              {bytes(done)} of {bytes(size)}
            </small>
          </div>
        ) : url && media ? (
          <Media media={media} url={url} name={name!} onError={() => setState((s) => ({ ...s, error: UNSUPPORTED[media] }))} />
        ) : url ? (
          <p className="viewer-note">This browser cannot show this file. Download it to open it.</p>
        ) : null}
      </main>
    </div>
  );
}

async function open(fragment: string, update: (change: Partial<State>) => void): Promise<void> {
  await init();
  const shared = await openShareLink(fragment);
  const { name, size } = shared;
  document.title = name;
  update({ name, size });

  const url = await streamed(shared.shown, name);
  if (url) {
    shared.free();
    // Reading the first byte tells a file gone from its servers apart from one the browser cannot show.
    if (size > 0) {
      const probe = await fetch(url, { headers: { Range: "bytes=0-0" } });
      if (!probe.ok) {
        update({ error: await probe.text() });
        return;
      }
      await probe.body?.cancel();
    }
    update({ url, download: `${url}?download` });
    return;
  }

  try {
    if (size > MEMORY_LIMIT) {
      update({ error: "This browser cannot open a file this large from a link. Try a recent Chrome, Edge, Firefox or Safari." });
      return;
    }
    update({ done: 0 });
    const content = await shared.read((done) => update({ done }));
    const blob = URL.createObjectURL(new File([content], name, { type: mimeType(name) }));
    update({ url: blob, download: blob });
  } finally {
    shared.free();
  }
}

/** The URL the service worker serves the file `shown` describes at, or `undefined` when this browser has none. */
async function streamed(shown: string, name: string): Promise<string | undefined> {
  const workers = navigator.serviceWorker;
  if (!workers) return undefined;
  const id = crypto.randomUUID();
  workers.addEventListener("message", (event) => {
    const request = event.data as LinkRequest | undefined;
    if (request?.type === LINK_REQUEST) event.ports[0]?.postMessage(request.id === id ? shown : null);
  });
  workers.startMessages();
  try {
    await workers.register("sw.js", { type: "module", scope: "./" });
    await controlled(workers);
  } catch (error) {
    console.warn("Reading the shared file into memory, without a service worker:", error);
    return undefined;
  }
  return new URL(`${STREAM}${id}/${encodeURIComponent(name)}`, location.href).href;
}

function controlled(workers: ServiceWorkerContainer): Promise<void> {
  if (workers.controller) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("the service worker did not take the page")), CONTROL_TIMEOUT);
    workers.addEventListener(
      "controllerchange",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
    void workers.ready.then((registration) => registration.active?.postMessage(CLAIM));
  });
}
