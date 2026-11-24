import { useEffect, useRef, useState } from "react";
import { lupine } from "../lib/client";
import type { OpenDrive } from "../lib/state";
import { ConfirmDialog, Copy, Modal, useAction } from "./dialogs";

const HISTORY = [7, 30, 90, 180, 365];

export function Settings({ drive, onClose }: { drive: OpenDrive; onClose: () => void }) {
  const { info } = drive;
  const [recoveryKey, setRecoveryKey] = useState<string>();
  const [relays, setRelays] = useState(info.relays.join("\n"));
  const [servers, setServers] = useState(info.servers.join("\n"));
  const [signingOut, setSigningOut] = useState(false);
  const done = useRef<HTMLButtonElement>(null);
  useEffect(() => done.current?.focus(), [signingOut]);
  const lists = useAction();
  const lines = (text: string) => text.split("\n").map((line) => line.trim()).filter(Boolean);
  const listsChanged = relays !== info.relays.join("\n") || servers !== info.servers.join("\n");

  if (signingOut) {
    return (
      <ConfirmDialog
        title="Sign out of this browser?"
        body={
          <div className="stack">
            <p>The drive stays on its relays and servers. This browser forgets it and its key.</p>
            {drive.unsent > 0 && (
              <p className="error">
                {drive.unsent === 1 ? "1 change has" : `${drive.unsent} changes have`} not reached a relay yet and
                will be lost.
              </p>
            )}
          </div>
        }
        confirm="Sign out"
        onConfirm={() => lupine.signOut()}
        onClose={() => setSigningOut(false)}
      />
    );
  }

  return (
    <Modal title="Settings" wide onClose={onClose}>
      <div className="settings">
        <section>
          <h3>Drive key</h3>
          <p className="key">{info.driveKey}</p>
          <p className="muted">
            Relays and servers see this key, not your npub. Paid ones must allow it.
            {info.user && " This browser signed in as " + info.user + "."}
          </p>
        </section>

        <section>
          <h3>Recovery key</h3>
          <p className="muted">Opens the drive without your Nostr key, on any device. Anyone who has it can read the drive.</p>
          {recoveryKey ? (
            <>
              <p className="key">{recoveryKey}</p>
              <div className="row">
                <Copy text={recoveryKey} />
              </div>
            </>
          ) : (
            <div>
              <button className="button" onClick={() => void lupine.recoveryKey().then(setRecoveryKey)}>
                Show recovery key
              </button>
            </div>
          )}
        </section>

        <section>
          <h3>History</h3>
          <label className="field">
            <span>Replaced versions stay on the relays for</span>
            <select
              className="input"
              value={drive.historyDays}
              onChange={(e) => void lupine.setHistoryDays(Number(e.target.value))}
            >
              {[...new Set([...HISTORY, drive.historyDays])]
                .sort((a, b) => a - b)
                .map((days) => (
                  <option key={days} value={days}>
                    {days === 365 ? "1 year" : `${days} days`}
                  </option>
                ))}
            </select>
            <small>The shortest setting among your devices applies to the whole drive.</small>
          </label>
        </section>

        <section>
          <h3>Relays and servers</h3>
          <label className="field">
            <span>Relays</span>
            <textarea className="input" spellCheck={false} value={relays} onChange={(e) => setRelays(e.target.value)} />
          </label>
          <label className="field">
            <span>Blossom servers</span>
            <textarea className="input" spellCheck={false} value={servers} onChange={(e) => setServers(e.target.value)} />
            <small>New servers get a copy of every file, which takes a while for a large drive.</small>
          </label>
          {lists.error && <p className="error">{lists.error}</p>}
          <div>
            <button
              className="button"
              disabled={!listsChanged || lists.busy || !lines(relays).length || !lines(servers).length}
              onClick={() =>
                void lists.run(async () => {
                  await lupine.changeLists(lines(relays), lines(servers));
                  onClose();
                })
              }
            >
              {lists.busy ? "Saving" : "Save relays and servers"}
            </button>
          </div>
        </section>

        <section>
          <h3>This browser</h3>
          <div className="row">
            <button className="button" onClick={() => setSigningOut(true)}>
              Sign out
            </button>
          </div>
        </section>
      </div>
      <div className="buttons">
        <button ref={done} className="button primary" onClick={onClose}>
          Done
        </button>
      </div>
    </Modal>
  );
}
