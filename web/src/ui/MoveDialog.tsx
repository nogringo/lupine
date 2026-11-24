import { useEffect, useState } from "react";
import { lupine } from "../lib/client";
import type { DriveItem } from "../lib/state";
import { Modal, useAction } from "./dialogs";
import { FolderIcon } from "./icons";

export function MoveDialog({ item, rootName, onClose }: { item: DriveItem; rootName: string; onClose: () => void }) {
  const [at, setAt] = useState<string | undefined>(item.parent);
  const [folders, setFolders] = useState<DriveItem[]>([]);
  const [path, setPath] = useState<DriveItem[]>([]);
  const { busy, error, run } = useAction();

  useEffect(() => {
    let live = true;
    void Promise.all([lupine.children(at), lupine.path(at)]).then(([children, path]) => {
      if (!live) return;
      // A folder cannot move into itself.
      setFolders(children.filter((child) => child.isFolder && child.id !== item.id));
      setPath(path);
    });
    return () => {
      live = false;
    };
  }, [at, item.id]);

  return (
    <Modal title={`Move ${item.name}`} onClose={onClose}>
      <ol className="crumbs" style={{ fontSize: "1rem" }}>
        <li>
          <button onClick={() => setAt(undefined)}>{rootName}</button>
        </li>
        {path.map((step) => (
          <li key={step.id}>
            <button onClick={() => setAt(step.id)}>{step.name}</button>
          </li>
        ))}
      </ol>
      <div className="picker">
        {folders.length === 0 && <p className="muted" style={{ padding: "0.75em" }}>No folders in here.</p>}
        {folders.map((folder) => (
          <button key={folder.id} onClick={() => setAt(folder.id)}>
            <FolderIcon />
            {folder.name}
          </button>
        ))}
      </div>
      {error && <p className="error">{error}</p>}
      <div className="buttons">
        <button className="button quiet" onClick={onClose}>
          Cancel
        </button>
        <button
          className="button primary"
          disabled={busy || at === item.parent}
          onClick={() =>
            void run(async () => {
              await lupine.move(item, at);
              onClose();
            })
          }
        >
          Move here
        </button>
      </div>
    </Modal>
  );
}
