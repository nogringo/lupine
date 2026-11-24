import { type FormEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { message } from "../lib/format";
import { nameProblem } from "../lib/names";

export function Modal({
  title,
  wide,
  onClose,
  children,
}: {
  title: string;
  wide?: boolean;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
  }, []);
  return (
    <dialog ref={ref} className={wide ? "wide" : undefined} onClose={onClose} aria-labelledby="dialog-title">
      <div className="stack">
        <h2 id="dialog-title">{title}</h2>
        {children}
      </div>
    </dialog>
  );
}

/** Runs `action`, keeping the dialog open with the error when it fails. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(undefined);
    try {
      await action();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, run };
}

export function NameDialog({
  title,
  initial,
  submit,
  onSubmit,
  onClose,
}: {
  title: string;
  initial: string;
  submit: string;
  onSubmit: (name: string) => Promise<void>;
  onClose: () => void;
}) {
  const [name, setName] = useState(initial);
  const { busy, error, run } = useAction();
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const field = input.current;
    if (!field) return;
    field.focus();
    const dot = initial.lastIndexOf(".");
    field.setSelectionRange(0, dot > 0 ? dot : initial.length);
  }, [initial]);
  const problem = nameProblem(name);
  const send = (event: FormEvent) => {
    event.preventDefault();
    if (problem) return;
    void run(async () => {
      await onSubmit(name.trim());
      onClose();
    });
  };
  return (
    <Modal title={title} onClose={onClose}>
      <form onSubmit={send}>
        <label className="field">
          <span>Name</span>
          <input ref={input} className="input" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        {error && <p className="error">{error}</p>}
        <div className="buttons">
          <button type="button" className="button quiet" onClick={onClose}>
            Cancel
          </button>
          <button className="button primary" disabled={busy || problem !== undefined}>
            {submit}
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function ConfirmDialog({
  title,
  body,
  confirm,
  onConfirm,
  onClose,
}: {
  title: string;
  body: ReactNode;
  confirm: string;
  onConfirm: () => Promise<void> | void;
  onClose: () => void;
}) {
  const { busy, error, run } = useAction();
  return (
    <Modal title={title} onClose={onClose}>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void run(async () => {
            await onConfirm();
            onClose();
          });
        }}
      >
        <div>{body}</div>
        {error && <p className="error">{error}</p>}
        <div className="buttons">
          <button type="button" className="button quiet" onClick={onClose} autoFocus>
            Cancel
          </button>
          <button className="button danger" disabled={busy}>
            {confirm}
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function Copy({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className="button"
      onClick={() => {
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
    >
      {copied ? "Copied" : "Copy"}
    </button>
  );
}
