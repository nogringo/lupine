import { useEffect, useRef, useState } from "react";
import { message } from "../lib/format";
import type { OpenDrive } from "../lib/state";
import { useDismiss } from "./dialogs";

interface Problem {
  what?: string;
  why: string;
}

interface Summary {
  text: string;
  tone: "ok" | "busy" | "bad";
  /** Shown when the status is clicked. */
  problems?: Problem[];
}

/** The drive's sync status. Clicking it, when something failed, lists what. */
export function Status({ drive }: { drive: OpenDrive }) {
  const { text, tone, problems } = summary(drive);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useDismiss(ref, () => setOpen(false), open);

  const failing = problems !== undefined;
  useEffect(() => {
    if (!failing) setOpen(false);
  }, [failing]);

  return (
    <div ref={ref} className="status-anchor">
      {problems ? (
        <button
          className={`status ${tone}`}
          aria-expanded={open}
          aria-controls="status-details"
          onClick={() => setOpen(!open)}
        >
          {text}
        </button>
      ) : (
        <span className={`status ${tone}`}>{text}</span>
      )}
      {open && problems && (
        <ul id="status-details" className="status-details">
          {problems.map((problem, index) => (
            <li key={index}>
              {problem.what && <strong>{problem.what}</strong>}
              <span>{problem.why}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function summary(drive: OpenDrive): Summary {
  const sync = drive.sync;
  if (sync.kind === "failed") return { text: "Sync stopped", tone: "bad", problems: [{ why: message(sync.message) }] };
  if (sync.kind === "opening") return { text: "Connecting", tone: "busy" };
  if (sync.kind === "syncing") return { text: "Syncing", tone: "busy" };
  if (sync.failed.length > 0) {
    const relays = sync.failed.length === 1 ? "1 relay" : `${sync.failed.length} relays`;
    const problems = sync.failed.map((failure) => ({ what: failure.relay, why: message(failure.reason) }));
    return { text: `${relays} unreachable`, tone: "bad", problems };
  }
  if (drive.fill) return { text: `Copying files to new servers, ${drive.fill.done} of ${drive.fill.total}`, tone: "busy" };
  if (drive.unsent > 0) {
    return { text: drive.unsent === 1 ? "Sending 1 change" : `Sending ${drive.unsent} changes`, tone: "busy" };
  }
  return { text: "Up to date", tone: "ok" };
}
