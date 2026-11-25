import { useEffect, useRef, useState } from "react";
import { lupine, save } from "../lib/client";
import { bytes, message } from "../lib/format";
import type { Viewable } from "../lib/media";
import type { DriveItem, Transfer } from "../lib/state";

const UNSUPPORTED: Record<Viewable, string> = {
  image: "This browser cannot show this image.",
  video: "This browser cannot play this video.",
  audio: "This browser cannot play this audio file.",
  pdf: "This browser cannot show this PDF.",
};

interface Shown {
  transfer: string;
  file?: File;
  url?: string;
  error?: string;
}

/** Shows a file of the drive over it: an image, a video, an audio file or a PDF. */
export function Viewer({
  item,
  media,
  transfers,
  onClose,
}: {
  item: DriveItem;
  media: Viewable;
  transfers: Transfer[];
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [shown, setShown] = useState<Shown>();

  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
  }, []);

  useEffect(() => {
    const transfer = crypto.randomUUID();
    let live = true;
    let url: string | undefined;
    setShown({ transfer });
    lupine.download(item, transfer).then(
      (file) => {
        if (!live) return;
        url = URL.createObjectURL(file);
        setShown({ transfer, file, url });
      },
      (error: unknown) => {
        if (live) setShown({ transfer, error: message(error) });
      },
    );
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
      // Stops the download if it still runs, or clears it if it failed.
      void lupine.cancelTransfer(transfer);
    };
  }, [item]);

  const unplayable = () => setShown((s) => s && { ...s, error: UNSUPPORTED[media] });

  const progress = transfers.find((t) => t.id === shown?.transfer);
  const file = shown?.file;

  return (
    <dialog ref={ref} className="viewer" aria-labelledby="viewer-title" onClose={onClose}>
      <header className="viewer-bar">
        <h2 id="viewer-title">{item.name}</h2>
        <button className="button" disabled={!file} onClick={() => file && save(file)}>
          Download
        </button>
        <button className="button quiet" onClick={onClose}>
          Close
        </button>
      </header>
      <div
        className="viewer-stage"
        aria-busy={!shown?.url && !shown?.error}
        onClick={(event) => {
          if (event.target === event.currentTarget) onClose();
        }}
      >
        {shown?.error ? (
          <p className="viewer-note">{shown.error}</p>
        ) : !shown?.url ? (
          <div className="viewer-note">
            <progress max={item.size || 1} value={progress?.done ?? 0} />
            <small>
              {bytes(progress?.done ?? 0)} of {bytes(item.size)}
            </small>
          </div>
        ) : (
          <Content media={media} url={shown.url} name={item.name} onError={unplayable} />
        )}
      </div>
    </dialog>
  );
}

function Content({ media, url, name, onError }: { media: Viewable; url: string; name: string; onError: () => void }) {
  switch (media) {
    case "image":
      return <img src={url} alt={name} onError={onError} />;
    case "video":
      return <video src={url} controls autoPlay onError={onError} />;
    case "audio":
      return <audio src={url} controls autoPlay onError={onError} />;
    case "pdf":
      // No sandbox: Chrome shows no PDF in one. The browser's PDF viewer keeps the file's scripts from this page.
      return <iframe src={url} title={name} />;
  }
}
