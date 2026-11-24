import { type DragEvent, type KeyboardEvent, useEffect, useRef, useState } from "react";
import { lupine, save } from "../lib/client";
import { dropped, picked } from "../lib/drop";
import { bytes, date, message } from "../lib/format";
import { folded } from "../lib/names";
import type { Collision, DriveItem, OpenDrive, Upload } from "../lib/state";
import { ConfirmDialog, Copy, Modal, NameDialog } from "./dialogs";
import { Mark } from "./Flower";
import { FileIcon, FolderIcon, MoreIcon } from "./icons";
import { MoveDialog } from "./MoveDialog";
import { Settings } from "./Settings";
import { Transfers } from "./Transfers";

/** What a row drag carries, so drops tell a move from an upload. */
const ITEM = "application/x-lupine-item";

type Dialog =
  | { kind: "newFolder" }
  | { kind: "rename"; item: DriveItem }
  | { kind: "move"; item: DriveItem }
  | { kind: "delete"; item: DriveItem }
  | { kind: "collision"; uploads: Upload[]; conflicts: number }
  | { kind: "settings" };

type SortKey = "name" | "size" | "modified";

export function Drive({
  drive,
  newRecoveryKey,
  onRecoveryKeySaved,
}: {
  drive: OpenDrive;
  newRecoveryKey?: string;
  onRecoveryKeySaved: () => void;
}) {
  const [folder, setFolder] = useState<string>();
  const [items, setItems] = useState<DriveItem[]>([]);
  const [path, setPath] = useState<DriveItem[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [selected, setSelected] = useState<string>();
  const [menu, setMenu] = useState<{ item: DriveItem; x: number; y: number }>();
  const [dialog, setDialog] = useState<Dialog>();
  const [failure, setFailure] = useState<string>();
  const [filesOver, setFilesOver] = useState(false);
  /** A folder row, or a breadcrumb (the root as ""), under a drag. */
  const [target, setTarget] = useState<string>();
  const [sort, setSort] = useState<{ key: SortKey; up: boolean }>({ key: "name", up: true });
  const fileInput = useRef<HTMLInputElement>(null);
  const folderInput = useRef<HTMLInputElement>(null);
  const dragDepth = useRef(0);

  useEffect(() => {
    let live = true;
    Promise.all([lupine.children(folder), lupine.path(folder)])
      .then(([children, path]) => {
        if (!live) return;
        setItems(children);
        setPath(path);
        setLoaded(true);
      })
      .catch((error: unknown) => {
        if (!live) return;
        // Deleted elsewhere.
        if (error instanceof Error && error.name === "NoSuchItem") setFolder(undefined);
        else setFailure(message(error));
      });
    return () => {
      live = false;
    };
  }, [folder, drive.revision]);

  const open = (id?: string) => {
    setFolder(id);
    setSelected(undefined);
    setLoaded(false);
  };

  const act = (action: () => Promise<unknown>) => {
    setFailure(undefined);
    action().catch((error: unknown) => {
      if (!(error instanceof Error && error.name === "Cancelled")) setFailure(message(error));
    });
  };

  const download = (item: DriveItem) => act(async () => save(await lupine.download(item)));

  const activate = (item: DriveItem) => (item.isFolder ? open(item.id) : download(item));

  const upload = (uploads: Upload[], into = folder) => {
    if (uploads.length === 0) return;
    if (into === folder) {
      const names = new Set(items.filter((item) => !item.isFolder).map((item) => folded(item.name)));
      const conflicts = uploads.filter((u) => u.folders.length === 0 && names.has(folded(u.file.name))).length;
      if (conflicts > 0) {
        setDialog({ kind: "collision", uploads, conflicts });
        return;
      }
    }
    startUpload(uploads, "keepBoth", into);
  };

  const startUpload = (uploads: Upload[], collision: Collision, into = folder) =>
    act(() => lupine.upload(into, uploads, collision));

  const move = (id: string, parent: string | undefined) => {
    const item = items.find((i) => i.id === id);
    if (!item || item.id === parent || item.parent === parent) return;
    act(() => lupine.move(item, parent));
  };

  // Drags: files from the desktop upload, rows move.

  const onAreaDragEnter = (event: DragEvent) => {
    if (!event.dataTransfer.types.includes("Files")) return;
    dragDepth.current++;
    setFilesOver(true);
  };
  const onAreaDragLeave = (event: DragEvent) => {
    if (!event.dataTransfer.types.includes("Files")) return;
    dragDepth.current = Math.max(0, dragDepth.current - 1);
    if (dragDepth.current === 0) setFilesOver(false);
  };
  const onAreaDragOver = (event: DragEvent) => {
    if (event.dataTransfer.types.includes("Files")) event.preventDefault();
  };
  const onAreaDrop = (event: DragEvent) => {
    dragDepth.current = 0;
    setFilesOver(false);
    if (!event.dataTransfer.types.includes("Files")) return;
    event.preventDefault();
    void dropped(event.dataTransfer).then((uploads) => upload(uploads));
  };

  const dropTarget = (id: string | undefined) => ({
    onDragOver: (event: DragEvent) => {
      const types = event.dataTransfer.types;
      if (!types.includes(ITEM) && !types.includes("Files")) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = types.includes(ITEM) ? "move" : "copy";
      setTarget(id ?? "");
    },
    onDragLeave: () => setTarget(undefined),
    onDrop: (event: DragEvent) => {
      setTarget(undefined);
      const moved = event.dataTransfer.getData(ITEM);
      if (moved) {
        event.preventDefault();
        event.stopPropagation();
        move(moved, id);
      } else if (event.dataTransfer.types.includes("Files") && id !== folder) {
        event.preventDefault();
        event.stopPropagation();
        dragDepth.current = 0;
        setFilesOver(false);
        void dropped(event.dataTransfer).then((uploads) => upload(uploads, id));
      }
    },
  });

  const onRowKey = (event: KeyboardEvent, item: DriveItem) => {
    if (event.key === "Enter") activate(item);
    else if (event.key === "Delete" || event.key === "Backspace") setDialog({ kind: "delete", item });
    else if (event.key === "F2") setDialog({ kind: "rename", item });
    else return;
    event.preventDefault();
  };

  const sorted = [...items].sort((a, b) => {
    if (a.isFolder !== b.isFolder) return a.isFolder ? -1 : 1;
    const order =
      sort.key === "size"
        ? a.size - b.size
        : sort.key === "modified"
          ? (a.modified ?? 0) - (b.modified ?? 0)
          : a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
    return sort.up ? order : -order;
  });

  const header = (key: SortKey, label: string, className?: string) => (
    <th
      className={className}
      aria-sort={sort.key === key ? (sort.up ? "ascending" : "descending") : undefined}
    >
      <button
        className="button quiet"
        onClick={() => setSort({ key, up: sort.key === key ? !sort.up : true })}
      >
        {label}
      </button>
    </th>
  );

  const status = statusOf(drive);
  const title = drive.info.name ?? "Lupine";

  return (
    <div className="drive">
      <header className="topbar">
        <Mark />
        <h1>{title}</h1>
        <span className={`status ${status.tone}`} title={status.detail}>
          {status.text}
        </span>
        <button className="button quiet" onClick={() => setDialog({ kind: "settings" })}>
          Settings
        </button>
      </header>

      <div className="toolbar">
        <nav aria-label="Folder">
          <ol className="crumbs">
            <li>
              <button className={target === "" ? "target" : undefined} onClick={() => open(undefined)} {...dropTarget(undefined)}>
                {title}
              </button>
            </li>
            {path.map((step) => (
              <li key={step.id}>
                <button
                  className={target === step.id ? "target" : undefined}
                  onClick={() => open(step.id)}
                  {...dropTarget(step.id)}
                >
                  {step.name}
                </button>
              </li>
            ))}
          </ol>
        </nav>
        <div className="row actions">
          <button className="button" onClick={() => setDialog({ kind: "newFolder" })}>
            New folder
          </button>
          <button className="button" onClick={() => folderInput.current?.click()}>
            Upload folder
          </button>
          <button className="button primary" onClick={() => fileInput.current?.click()}>
            Upload files
          </button>
          <input
            ref={fileInput}
            type="file"
            multiple
            hidden
            onChange={(e) => {
              if (e.target.files) upload([...e.target.files].map((file) => ({ file, folders: [] })));
              e.target.value = "";
            }}
          />
          <input
            ref={folderInput}
            type="file"
            hidden
            {...{ webkitdirectory: "" }}
            onChange={(e) => {
              if (e.target.files) upload(picked(e.target.files));
              e.target.value = "";
            }}
          />
        </div>
      </div>

      <section
        className="files"
        aria-label="Files"
        onDragEnter={onAreaDragEnter}
        onDragLeave={onAreaDragLeave}
        onDragOver={onAreaDragOver}
        onDrop={onAreaDrop}
      >
        {failure && (
          <p className="key error row" role="alert">
            {failure}
            <button className="button quiet" onClick={() => setFailure(undefined)}>
              Dismiss
            </button>
          </p>
        )}
        {loaded && items.length === 0 ? (
          <div className="empty">
            <h2>{folder ? "This folder is empty" : "Your drive is empty"}</h2>
            <p className="muted">Drop files or folders here, or upload them. They are encrypted in this browser before they leave it.</p>
          </div>
        ) : (
          <table>
            <thead>
              <tr>
                {header("name", "Name")}
                {header("size", "Size", "num")}
                {header("modified", "Modified", "num modified")}
                <th className="more">
                  <span hidden>Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {sorted.map((item) => (
                <tr
                  key={item.id}
                  tabIndex={0}
                  className={[selected === item.id && "selected", target === item.id && "target"].filter(Boolean).join(" ")}
                  draggable
                  onDragStart={(event) => {
                    event.dataTransfer.setData(ITEM, item.id);
                    event.dataTransfer.effectAllowed = "move";
                  }}
                  onClick={() => setSelected(item.id)}
                  onDoubleClick={() => activate(item)}
                  onKeyDown={(event) => onRowKey(event, item)}
                  onFocus={() => setSelected(item.id)}
                  {...(item.isFolder ? dropTarget(item.id) : {})}
                >
                  <td>
                    <div className="name">
                      {item.isFolder ? <FolderIcon /> : <FileIcon />}
                      <span>{item.name}</span>
                    </div>
                  </td>
                  <td className="num">{item.isFolder ? "" : bytes(item.size)}</td>
                  <td className="num modified">{date(item.modified)}</td>
                  <td className="more">
                    <button
                      className="button quiet"
                      aria-label={`Actions for ${item.name}`}
                      aria-haspopup="menu"
                      aria-expanded={menu?.item.id === item.id}
                      onClick={(event) => {
                        event.stopPropagation();
                        const rect = event.currentTarget.getBoundingClientRect();
                        setSelected(item.id);
                        setMenu({ item, x: rect.right, y: rect.bottom + 4 });
                      }}
                    >
                      <MoreIcon />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {filesOver && <div className="dropzone">Drop to upload into {path.at(-1)?.name ?? title}</div>}
      </section>

      {menu && (
        <ItemMenu
          {...menu}
          onClose={() => setMenu(undefined)}
          onOpen={() => activate(menu.item)}
          onRename={() => setDialog({ kind: "rename", item: menu.item })}
          onMove={() => setDialog({ kind: "move", item: menu.item })}
          onDelete={() => setDialog({ kind: "delete", item: menu.item })}
        />
      )}

      <Transfers transfers={drive.transfers} />

      {dialog?.kind === "newFolder" && (
        <NameDialog
          title="New folder"
          initial="New folder"
          submit="Create"
          onSubmit={async (name) => {
            await lupine.createFolder(folder, name);
          }}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog?.kind === "rename" && (
        <NameDialog
          title={`Rename ${dialog.item.isFolder ? "folder" : "file"}`}
          initial={dialog.item.name}
          submit="Rename"
          onSubmit={async (name) => {
            if (name !== dialog.item.name) await lupine.rename(dialog.item, name);
          }}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog?.kind === "move" && (
        <MoveDialog item={dialog.item} rootName={title} onClose={() => setDialog(undefined)} />
      )}
      {dialog?.kind === "delete" && (
        <ConfirmDialog
          title={`Delete ${dialog.item.name}?`}
          body={
            <p>
              {dialog.item.isFolder ? "The folder and everything in it are" : "The file is"} deleted from every
              device. The drive keeps no trash.
            </p>
          }
          confirm="Delete"
          onConfirm={() => lupine.remove(dialog.item)}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog?.kind === "collision" && (
        <Modal title="Some files are already here" onClose={() => setDialog(undefined)}>
          <p>
            {dialog.conflicts === 1
              ? "A file with the same name is already in this folder."
              : `${dialog.conflicts} files with the same names are already in this folder.`}{" "}
            Replacing makes a new version of each of them.
          </p>
          <div className="buttons">
            <button className="button quiet" onClick={() => setDialog(undefined)}>
              Cancel
            </button>
            <button
              className="button"
              onClick={() => {
                startUpload(dialog.uploads, "keepBoth");
                setDialog(undefined);
              }}
            >
              Keep both
            </button>
            <button
              className="button primary"
              onClick={() => {
                startUpload(dialog.uploads, "replace");
                setDialog(undefined);
              }}
            >
              Replace
            </button>
          </div>
        </Modal>
      )}
      {dialog?.kind === "settings" && <Settings drive={drive} onClose={() => setDialog(undefined)} />}
      {newRecoveryKey && !dialog && (
        <Modal title="Save your recovery key" onClose={onRecoveryKeySaved}>
          <p>It opens this drive without your Nostr key, on any device. Keep it somewhere safe: anyone who has it can read the drive.</p>
          <p className="key">{newRecoveryKey}</p>
          <div className="buttons">
            <Copy text={newRecoveryKey} />
            <button className="button primary" onClick={onRecoveryKeySaved}>
              I saved it
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}

function ItemMenu({
  item,
  x,
  y,
  onClose,
  onOpen,
  onRename,
  onMove,
  onDelete,
}: {
  item: DriveItem;
  x: number;
  y: number;
  onClose: () => void;
  onOpen: () => void;
  onRename: () => void;
  onMove: () => void;
  onDelete: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector("button")?.focus();
    const away = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    const escape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", away);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", away);
      document.removeEventListener("keydown", escape);
    };
  }, [onClose]);
  const choose = (action: () => void) => () => {
    onClose();
    action();
  };
  return (
    <div
      ref={ref}
      className="menu"
      role="menu"
      style={{ left: Math.max(8, x - 176), top: Math.min(y, window.innerHeight - 180) }}
    >
      <button role="menuitem" onClick={choose(onOpen)}>
        {item.isFolder ? "Open" : "Download"}
      </button>
      <button role="menuitem" onClick={choose(onRename)}>
        Rename
      </button>
      <button role="menuitem" onClick={choose(onMove)}>
        Move to...
      </button>
      <button role="menuitem" className="danger" onClick={choose(onDelete)}>
        Delete
      </button>
    </div>
  );
}

function statusOf(drive: OpenDrive): { text: string; tone: string; detail?: string } {
  const sync = drive.sync;
  if (sync.kind === "failed") return { text: "Sync stopped", tone: "bad", detail: sync.message };
  if (sync.kind === "opening") return { text: "Connecting", tone: "busy" };
  if (sync.kind === "syncing") return { text: "Syncing", tone: "busy" };
  if (sync.failed.length > 0) {
    const relays = sync.failed.length === 1 ? "1 relay" : `${sync.failed.length} relays`;
    return { text: `${relays} unreachable`, tone: "bad", detail: sync.failed.join("\n") };
  }
  if (drive.fill) return { text: `Copying files to new servers, ${drive.fill.done} of ${drive.fill.total}`, tone: "busy" };
  if (drive.unsent > 0) {
    return { text: drive.unsent === 1 ? "Sending 1 change" : `Sending ${drive.unsent} changes`, tone: "busy" };
  }
  return { text: "Up to date", tone: "ok" };
}
