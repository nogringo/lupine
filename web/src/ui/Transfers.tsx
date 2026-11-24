import { lupine } from "../lib/client";
import { bytes } from "../lib/format";
import type { Transfer } from "../lib/state";

export function Transfers({ transfers }: { transfers: Transfer[] }) {
  if (transfers.length === 0) return null;
  const running = transfers.filter((t) => !t.error);
  const uploads = running.filter((t) => t.kind === "upload").length;
  const downloads = running.length - uploads;
  const parts = [
    uploads && (uploads === 1 ? "Uploading 1 file" : `Uploading ${uploads} files`),
    downloads && (downloads === 1 ? "downloading 1 file" : `downloading ${downloads} files`),
  ].filter(Boolean);
  const heading = parts.length ? parts.join(", ") : "Transfers that failed";
  return (
    <aside className="transfers" aria-label="Transfers">
      <h2>{heading.charAt(0).toUpperCase() + heading.slice(1)}</h2>
      {transfers.map((transfer) => (
        <div key={transfer.id} className="transfer">
          <span className="name">{transfer.name}</span>
          <button className="button quiet" onClick={() => void lupine.cancelTransfer(transfer.id)}>
            {transfer.error ? "Dismiss" : "Cancel"}
          </button>
          {transfer.error ? (
            <small className="error">{transfer.error}</small>
          ) : (
            <>
              <progress max={transfer.size || 1} value={transfer.done} />
              <small>
                {bytes(transfer.done)} of {bytes(transfer.size)}
              </small>
            </>
          )}
        </div>
      ))}
    </aside>
  );
}
