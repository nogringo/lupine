import { useEffect, useState } from "react";
import { start } from "../lib/client";
import { message } from "../lib/format";
import type { AppState } from "../lib/state";
import { Drive } from "./Drive";
import { SignIn } from "./SignIn";
import "./styles.css";

let started = false;

export function App() {
  const [state, setState] = useState<AppState>({ phase: "starting" });
  const [failure, setFailure] = useState<string>();
  /** Shown once the drive it belongs to opens. */
  const [newRecoveryKey, setNewRecoveryKey] = useState<string>();

  useEffect(() => {
    if (started) return;
    started = true;
    start(setState, (error) => setFailure(message(error)));
  }, []);

  if (failure) {
    return (
      <main className="notice">
        <h1>Lupine could not start</h1>
        <p className="error">{failure}</p>
        <button className="button" onClick={() => location.reload()}>
          Reload
        </button>
      </main>
    );
  }

  switch (state.phase) {
    case "starting":
      return <main className="notice" aria-busy="true" />;
    case "unsupported":
      return (
        <main className="notice">
          <h1>This browser cannot run Lupine</h1>
          <p>{state.reason} Use a recent Chrome, Edge, Firefox or Safari.</p>
        </main>
      );
    case "elsewhere":
      return (
        <main className="notice">
          <h1>Lupine is open in another tab</h1>
          <p className="muted">One tab at a time holds the drive. Close the other one and this tab opens it.</p>
        </main>
      );
    case "signedOut":
      return <SignIn retired={state.retired} onCreated={setNewRecoveryKey} />;
    case "open":
      return (
        <Drive
          drive={state.drive}
          newRecoveryKey={newRecoveryKey}
          onRecoveryKeySaved={() => setNewRecoveryKey(undefined)}
        />
      );
  }
}
