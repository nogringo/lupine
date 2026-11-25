import { type FormEvent, useEffect, useState } from "react";
import type { ShareInfo } from "../../core/lupine";
import { lupine } from "../lib/client";
import { date, message } from "../lib/format";
import type { DriveItem } from "../lib/state";
import { Copy, Modal } from "./dialogs";

const DAY = 24 * 60 * 60 * 1000;

/** Expiry choices, in days from now. 0: never. */
const EXPIRIES = [
  { days: 0, label: "Never" },
  { days: 1, label: "In a day" },
  { days: 7, label: "In a week" },
  { days: 30, label: "In a month" },
];

const expiry = (days: number) => (days === 0 ? undefined : Date.now() + days * DAY);

const url = (share: ShareInfo) => new URL(`s/#${share.link}`, location.href).href;

export function ShareDialog({ item, onClose }: { item: DriveItem; onClose: () => void }) {
  const [shares, setShares] = useState<ShareInfo[]>();
  const [error, setError] = useState<string>();
  const [created, setCreated] = useState<string>();

  const reload = () =>
    lupine.shares(item).then(setShares, (e: unknown) => setError(message(e)));

  useEffect(() => {
    void reload();
  }, [item]);

  return (
    <Modal title={`Share ${item.name}`} wide onClose={onClose}>
      <p>
        Anyone who has a link opens the file without an account, as it is and as it changes. Stopping a link closes it
        for whoever has not opened it yet.
      </p>
      {error && <p className="error">{error}</p>}
      {shares && shares.length > 0 && (
        <ul className="shares">
          {shares.map((share) => (
            <ShareRow
              key={share.id}
              share={share}
              fresh={share.id === created}
              onChanged={reload}
              onError={(e) => setError(message(e))}
            />
          ))}
        </ul>
      )}
      <NewShare
        item={item}
        first={shares?.length === 0}
        onCreated={(share) => {
          setError(undefined);
          setCreated(share.id);
          void reload();
        }}
      />
      <div className="buttons">
        <button type="button" className="button primary" onClick={onClose}>
          Done
        </button>
      </div>
    </Modal>
  );
}

function NewShare({ item, first, onCreated }: { item: DriveItem; first: boolean; onCreated: (share: ShareInfo) => void }) {
  const [label, setLabel] = useState("");
  const [days, setDays] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const create = (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(undefined);
    lupine
      .createShare(item, label, expiry(days))
      .then(
        (share) => {
          setLabel("");
          setDays(0);
          onCreated(share);
        },
        (e: unknown) => setError(message(e)),
      )
      .finally(() => setBusy(false));
  };

  return (
    <form className="new-share" onSubmit={create}>
      <h3>{first ? "Create a link" : "Another link"}</h3>
      <div className="row">
        <label className="field grow">
          <span>Label</span>
          <input
            className="input"
            value={label}
            placeholder="Link"
            maxLength={100}
            onChange={(e) => setLabel(e.target.value)}
          />
        </label>
        <ExpiryField days={days} onChange={setDays} />
        <button className="button" disabled={busy}>
          {busy ? "Creating..." : "Create link"}
        </button>
      </div>
      {error && <p className="error">{error}</p>}
    </form>
  );
}

function ExpiryField({ days, keep, onChange }: { days: number; keep?: string; onChange: (days: number) => void }) {
  return (
    <label className="field">
      <span>Expires</span>
      <select className="input" value={days} onChange={(e) => onChange(Number(e.target.value))}>
        {keep && <option value={-1}>{keep}</option>}
        {EXPIRIES.map((choice) => (
          <option key={choice.days} value={choice.days}>
            {choice.label}
          </option>
        ))}
      </select>
    </label>
  );
}

type Mode = { kind: "view" } | { kind: "edit" } | { kind: "stop"; busy: boolean };

function ShareRow({
  share,
  fresh,
  onChanged,
  onError,
}: {
  share: ShareInfo;
  fresh: boolean;
  onChanged: () => void;
  onError: (error: unknown) => void;
}) {
  const [mode, setMode] = useState<Mode>({ kind: "view" });
  const link = url(share);

  const stop = () => {
    setMode({ kind: "stop", busy: true });
    lupine.stopShare(share.id).then(onChanged, (e: unknown) => {
      setMode({ kind: "stop", busy: false });
      onError(e);
    });
  };

  const send = () => {
    navigator.share({ title: share.label, url: link }).catch((e: unknown) => {
      if (!(e instanceof DOMException && e.name === "AbortError")) onError(e);
    });
  };

  if (mode.kind === "edit") {
    return (
      <li className="share">
        <EditShare
          share={share}
          onDone={() => {
            setMode({ kind: "view" });
            onChanged();
          }}
          onCancel={() => setMode({ kind: "view" })}
          onError={onError}
        />
      </li>
    );
  }

  return (
    <li className={fresh ? "share fresh" : "share"}>
      <div className="share-head">
        <strong>{share.label}</strong>
        <small className="muted">{share.expires ? `Expires ${date(share.expires)}` : "Does not expire"}</small>
      </div>
      <p className="key">{link}</p>
      {mode.kind === "stop" ? (
        <div className="row">
          <span className="grow">Stop this link? It cannot be turned back on.</span>
          <button className="button quiet" disabled={mode.busy} onClick={() => setMode({ kind: "view" })}>
            Cancel
          </button>
          <button className="button danger" disabled={mode.busy} onClick={stop}>
            {mode.busy ? "Stopping..." : "Stop link"}
          </button>
        </div>
      ) : (
        <div className="row">
          <Copy text={link} />
          {"share" in navigator && (
            <button className="button" onClick={send}>
              Share...
            </button>
          )}
          <button className="button quiet" onClick={() => setMode({ kind: "edit" })}>
            Edit
          </button>
          <button className="button quiet danger-text" onClick={() => setMode({ kind: "stop", busy: false })}>
            Stop
          </button>
        </div>
      )}
    </li>
  );
}

function EditShare({
  share,
  onDone,
  onCancel,
  onError,
}: {
  share: ShareInfo;
  onDone: () => void;
  onCancel: () => void;
  onError: (error: unknown) => void;
}) {
  const [label, setLabel] = useState(share.label);
  const [days, setDays] = useState(-1);

  const save = (event: FormEvent) => {
    event.preventDefault();
    const expires = days === -1 ? undefined : (expiry(days) ?? null);
    lupine.editShare(share.id, label, expires).then(onDone, onError);
  };

  return (
    <form onSubmit={save}>
      <div className="row">
        <label className="field grow">
          <span>Label</span>
          <input className="input" value={label} maxLength={100} onChange={(e) => setLabel(e.target.value)} autoFocus />
        </label>
        <ExpiryField
          days={days}
          keep={share.expires ? `Keep (${date(share.expires)})` : "Keep (never)"}
          onChange={setDays}
        />
      </div>
      <div className="buttons">
        <button type="button" className="button quiet" onClick={onCancel}>
          Cancel
        </button>
        <button className="button primary">Save</button>
      </div>
    </form>
  );
}
