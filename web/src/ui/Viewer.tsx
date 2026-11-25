import { useEffect, useRef, useState } from "react";
import { lupine, save } from "../lib/client";
import { bytes, message } from "../lib/format";
import { viewable } from "../lib/media";
import type { DriveItem, Transfer } from "../lib/state";

interface Shown {
  transfer: string;
  file?: File;
  url?: string;
  error?: string;
}

/** Shows an image or a video of the drive over it. */
export function Viewer({
  item,
  transfers,
  onClose,
}: {
  item: DriveItem;
  transfers: Transfer[];
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [shown, setShown] = useState<Shown>();
  const kind = viewable(item.name);

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

  const unplayable = () =>
    setShown((s) => s && { ...s, error: `This browser cannot ${kind === "video" ? "play this video" : "show this image"}.` });

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
        ) : kind === "video" ? (
          <video src={shown.url} controls autoPlay onError={unplayable} />
        ) : (
          <img src={shown.url} alt={item.name} onError={unplayable} />
        )}
      </div>
    </dialog>
  );
}
