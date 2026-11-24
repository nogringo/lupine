import QRCode from "qrcode";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { callback, hasExtension, lupine, nip07 } from "../lib/client";
import { message, short } from "../lib/format";
import type { Account } from "../lib/state";
import { Copy } from "./dialogs";
import { Flower } from "./Flower";

type Step =
  | { kind: "choose" }
  | { kind: "pair"; link?: string }
  | { kind: "key" }
  | { kind: "recovery" }
  | { kind: "working"; label: string }
  | { kind: "account"; account: Account }
  | { kind: "create"; account: Account };

export function SignIn({ retired, onCreated }: { retired: boolean; onCreated: (recoveryKey: string) => void }) {
  const [step, setStep] = useState<Step>({ kind: "choose" });
  const [error, setError] = useState<string>();
  const [approval, setApproval] = useState<string>();
  // Results of a flow the user left are dropped.
  const flow = useRef(0);

  const begin = (next: Step) => {
    flow.current++;
    setError(undefined);
    setApproval(undefined);
    setStep(next);
    return flow.current;
  };

  const attempt = async (label: string, from: Step, action: () => Promise<Account | void>) => {
    const id = begin({ kind: "working", label });
    try {
      const account = await action();
      if (flow.current !== id) return;
      if (account) setStep(account.drives.length ? { kind: "account", account } : { kind: "create", account });
    } catch (e) {
      if (flow.current !== id) return;
      setStep(from);
      setError(message(e));
    }
  };

  const back = () => {
    begin({ kind: "choose" });
    void lupine.endSignIn();
  };

  const approve = callback((url: string) => setApproval(url));

  const pair = async () => {
    const id = begin({ kind: "pair" });
    try {
      const link = await lupine.pairSigner(approve);
      if (flow.current !== id) return;
      setStep({ kind: "pair", link });
      const account = await lupine.pairingAccepted();
      if (flow.current !== id) return;
      setStep(account.drives.length ? { kind: "account", account } : { kind: "create", account });
    } catch (e) {
      if (flow.current !== id) return;
      setStep({ kind: "choose" });
      setError(message(e));
    }
  };

  return (
    <div className="welcome">
      <section className="welcome-art">
        <Flower />
        <h1>Your files, sealed before they leave this browser.</h1>
        <p>Lupine keeps an encrypted drive on Nostr relays and Blossom servers. They store it without being able to read a name or a byte.</p>
      </section>
      <main className="welcome-panel">
        <div className="stack">
          {retired && step.kind === "choose" && (
            <p className="key">Another device changed the drive key. Sign in again to open the drive.</p>
          )}
          {step.kind === "choose" && (
            <Choose
              onPair={pair}
              onExtension={() =>
                attempt("Waiting for your extension", { kind: "choose" }, () =>
                  lupine.signInWithExtension(callback(nip07)),
                )
              }
              onKey={() => begin({ kind: "key" })}
              onRecovery={() => begin({ kind: "recovery" })}
            />
          )}
          {step.kind === "pair" && <Pairing link={step.link} onBack={back} />}
          {step.kind === "key" && (
            <KeyForm
              onBack={back}
              onSubmit={(key) =>
                attempt(key.startsWith("bunker://") ? "Waiting for your signer" : "Signing in", { kind: "key" }, () =>
                  lupine.signInWithKey(key, approve),
                )
              }
            />
          )}
          {step.kind === "recovery" && (
            <RecoveryForm
              onBack={back}
              onSubmit={(key) => attempt("Opening the drive", { kind: "recovery" }, () => lupine.importRecoveryKey(key))}
            />
          )}
          {step.kind === "working" && (
            <div className="stack" aria-live="polite">
              <h2>{step.label}</h2>
              <p className="muted">Looking for your relays and your drives.</p>
              <div>
                <button className="button quiet" onClick={back}>
                  Cancel
                </button>
              </div>
            </div>
          )}
          {step.kind === "account" && (
            <Drives
              account={step.account}
              onOpen={(id) => attempt("Opening the drive", step, () => lupine.openRemoteDrive(id))}
              onCreate={() => begin({ kind: "create", account: step.account })}
              onBack={back}
            />
          )}
          {step.kind === "create" && (
            <CreateForm
              account={step.account}
              onBack={step.account.drives.length ? () => begin({ kind: "account", account: step.account }) : back}
              onSubmit={(name, relays, servers) =>
                attempt("Creating the drive", step, async () => {
                  onCreated(await lupine.createDrive(name, relays, servers));
                })
              }
            />
          )}
          {approval && (
            <p className="key">
              Your signer asks you to approve Lupine.{" "}
              <a href={approval} target="_blank" rel="noreferrer">
                Open the approval page
              </a>
            </p>
          )}
          {error && <p className="error">{error}</p>}
        </div>
      </main>
    </div>
  );
}

function Choose({
  onPair,
  onExtension,
  onKey,
  onRecovery,
}: {
  onPair: () => void;
  onExtension: () => void;
  onKey: () => void;
  onRecovery: () => void;
}) {
  const extension = hasExtension();
  return (
    <>
      <h2>Sign in with Nostr</h2>
      <p className="muted">Your key opens your drive once. After that, this browser keeps only the drive's own key.</p>
      <div className="methods">
        <button className="method" onClick={onPair}>
          <strong>Signer app</strong>
          <span>Scan a code with a NIP-46 signer, such as Amber.</span>
        </button>
        <button className="method" onClick={onExtension} disabled={!extension}>
          <strong>Browser extension</strong>
          <span>{extension ? "Use the Nostr extension of this browser." : "No Nostr extension found in this browser."}</span>
        </button>
        <button className="method" onClick={onKey}>
          <strong>Bunker link or secret key</strong>
          <span>Paste a bunker:// link or an nsec.</span>
        </button>
        <button className="method" onClick={onRecovery}>
          <strong>Recovery key</strong>
          <span>Open a drive without your Nostr key.</span>
        </button>
      </div>
    </>
  );
}

function Pairing({ link, onBack }: { link?: string; onBack: () => void }) {
  const [svg, setSvg] = useState<string>();
  useEffect(() => {
    if (link) void QRCode.toString(link, { type: "svg", margin: 0, errorCorrectionLevel: "M" }).then(setSvg);
  }, [link]);
  return (
    <>
      <h2>Scan with your signer app</h2>
      <p className="muted">Then accept the connection there. This page continues on its own.</p>
      {svg ? <div className="qr" dangerouslySetInnerHTML={{ __html: svg }} /> : <div className="qr" aria-busy="true" />}
      {link && (
        <div className="row">
          <Copy text={link} />
          <a className="button" href={link}>
            Open in a signer app
          </a>
        </div>
      )}
      <div>
        <button className="button quiet" onClick={onBack}>
          Back
        </button>
      </div>
    </>
  );
}

function KeyForm({ onBack, onSubmit }: { onBack: () => void; onSubmit: (key: string) => void }) {
  const [key, setKey] = useState("");
  const send = (event: FormEvent) => {
    event.preventDefault();
    if (key.trim()) onSubmit(key.trim());
  };
  return (
    <form className="stack" onSubmit={send}>
      <h2>Bunker link or secret key</h2>
      <label className="field">
        <span>bunker:// link or nsec</span>
        <input
          className="input"
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          autoFocus
        />
        <small>An nsec is used for this sign-in only. This browser does not keep it.</small>
      </label>
      <div className="row">
        <button className="button primary" disabled={!key.trim()}>
          Sign in
        </button>
        <button type="button" className="button quiet" onClick={onBack}>
          Back
        </button>
      </div>
    </form>
  );
}

function RecoveryForm({ onBack, onSubmit }: { onBack: () => void; onSubmit: (key: string) => void }) {
  const [key, setKey] = useState("");
  return (
    <form
      className="stack"
      onSubmit={(event) => {
        event.preventDefault();
        if (key.trim()) onSubmit(key.trim());
      }}
    >
      <h2>Open with a recovery key</h2>
      <label className="field">
        <span>Recovery key</span>
        <textarea
          className="input"
          spellCheck={false}
          placeholder="lupine1..."
          value={key}
          onChange={(e) => setKey(e.target.value)}
          autoFocus
        />
        <small>It gives full access to the drive, like a device.</small>
      </label>
      <div className="row">
        <button className="button primary" disabled={!key.trim()}>
          Open drive
        </button>
        <button type="button" className="button quiet" onClick={onBack}>
          Back
        </button>
      </div>
    </form>
  );
}

function Drives({
  account,
  onOpen,
  onCreate,
  onBack,
}: {
  account: Account;
  onOpen: (id: string) => void;
  onCreate: () => void;
  onBack: () => void;
}) {
  return (
    <>
      <h2>Choose a drive</h2>
      <p className="muted">Signed in as {short(account.user)}.</p>
      <div className="drives">
        {account.drives.map((drive) => (
          <button key={drive.id} className="method" onClick={() => onOpen(drive.id)}>
            <strong>{drive.name}</strong>
            <span>Open this drive in this browser.</span>
          </button>
        ))}
      </div>
      <div className="row">
        <button className="button" onClick={onCreate}>
          Create a drive
        </button>
        <button className="button quiet" onClick={onBack}>
          Use another account
        </button>
      </div>
    </>
  );
}

function CreateForm({
  account,
  onBack,
  onSubmit,
}: {
  account: Account;
  onBack: () => void;
  onSubmit: (name: string, relays: string[], servers: string[]) => void;
}) {
  const [name, setName] = useState("Lupine");
  const [relays, setRelays] = useState(account.suggested.relays.join("\n"));
  const [servers, setServers] = useState(account.suggested.servers.join("\n"));
  const lines = (text: string) => text.split("\n").map((line) => line.trim()).filter(Boolean);
  const ready = lines(relays).length > 0 && lines(servers).length > 0;
  return (
    <form
      className="stack"
      onSubmit={(event) => {
        event.preventDefault();
        if (ready) onSubmit(name, lines(relays), lines(servers));
      }}
    >
      <h2>Create a drive</h2>
      <p className="muted">
        {account.drives.length === 0 && "You have no drive yet. "}
        Signed in as {short(account.user)}.
      </p>
      <label className="field">
        <span>Name</span>
        <input className="input" value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label className="field">
        <span>Relays</span>
        <textarea className="input" spellCheck={false} value={relays} onChange={(e) => setRelays(e.target.value)} />
        <small>One per line. They keep the drive's encrypted index. Prefer relays you run or pay for.</small>
      </label>
      <label className="field">
        <span>Blossom servers</span>
        <textarea className="input" spellCheck={false} value={servers} onChange={(e) => setServers(e.target.value)} />
        <small>One per line. They keep the encrypted file contents, so they must accept any file type: media hosts refuse them.</small>
      </label>
      <div className="row">
        <button className="button primary" disabled={!ready}>
          Create drive
        </button>
        <button type="button" className="button quiet" onClick={onBack}>
          Back
        </button>
      </div>
    </form>
  );
}
