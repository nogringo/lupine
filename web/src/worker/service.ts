import init, * as core from "../../core/lupine.js";
import type { DriveItem, ImportedDrive, ShareInfo, SyncEvent } from "../../core/lupine";
import { mimeType } from "../lib/media";
import { folded, freeName } from "../lib/names";
import type { Account, AppState, Collision, Nip07Call, OpenDrive, SyncStatus, Transfer, Upload } from "../lib/state";
import { clearDownloads, downloadFile } from "./downloads";
import { forgetSecret, keepSecret, secret as keptSecret } from "./secrets";

const DATABASE = "/lupine.db";
const PAGE = 500;
const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;
/** Progress repaints the page at most this often. */
const PROGRESS_INTERVAL = 150;
/** The session reports no sent change, so the count of unsent ones is read again. */
const UNSENT_INTERVAL = 1000;
const STORAGE_ATTEMPTS = 10;

export type Listener = (state: AppState) => void;

/** What runs while a drive is open. */
interface Opened {
  local: core.LocalDrive;
  secret: string;
  sync: core.DriveSync;
  writer: core.ItemWriter;
  reader: core.ContentReader;
  running: Promise<void>;
  keptDescriptor: boolean;
  transfers: Map<string, core.Cancellation>;
  unsentTimer?: ReturnType<typeof setInterval>;
  fill?: Background;
  collection?: Background;
  lastFill?: number;
  lastCollection?: number;
}

interface Background {
  cancellation: core.Cancellation;
  done: Promise<void>;
}

/**
 * The drive, as every tab sees it. Lives in the leader tab's dedicated
 * worker: OPFS sync access handles, which the local database needs, exist
 * only there.
 */
export class Service {
  private state: AppState = { phase: "starting" };
  private listeners = new Set<Listener>();
  private repaint?: ReturnType<typeof setTimeout>;
  private opened?: Opened;
  private signIn?: core.SignIn;
  private pairing?: core.SignerPairing;
  private started?: Promise<void>;

  subscribe(listener: Listener): void {
    this.listeners.add(listener);
    listener(this.state);
  }

  unsubscribe(listener: Listener): void {
    this.listeners.delete(listener);
  }

  /** Opens the drive, once this worker holds it. */
  start(): Promise<void> {
    this.started ??= this.open();
    return this.started;
  }

  // Signing in

  /** An nsec or a `bunker://` link. `approve(url)` shows a signer's approval page. */
  async signInWithKey(key: string, approve: (url: string) => void): Promise<Account> {
    await this.endSignIn();
    this.signIn = await core.signIn(key, (url: string) => void approve(url));
    return this.account();
  }

  async signInWithExtension(call: Nip07Call): Promise<Account> {
    await this.endSignIn();
    this.signIn = await core.signInWithExtension((method: string, params: unknown[]) => call(method, params));
    return this.account();
  }

  /** A `nostrconnect://` link for a signer app to scan. */
  async pairSigner(approve: (url: string) => void): Promise<string> {
    await this.endSignIn();
    this.pairing = core.pairSigner((url: string) => void approve(url));
    return this.pairing.link;
  }

  /** Resolves once the user accepts in the signer app. */
  async pairingAccepted(): Promise<Account> {
    const pairing = this.pairing;
    if (!pairing) throw new Error("No signer pairing is under way.");
    try {
      this.signIn = await pairing.accepted();
    } finally {
      if (this.pairing === pairing) this.pairing = undefined;
      pairing.free();
    }
    return this.account();
  }

  async endSignIn(): Promise<void> {
    this.pairing?.cancel();
    this.pairing = undefined;
    const signIn = this.signIn;
    this.signIn = undefined;
    if (signIn) {
      await signIn.close();
      signIn.free();
    }
  }

  /** Opens drive `id` of the signed-in user. */
  async openRemoteDrive(id: string): Promise<void> {
    const imported = await this.requireSignIn().openDrive(DATABASE, id);
    await this.setUp(imported);
  }

  /** Returns the new drive's recovery key. */
  async createDrive(name: string, relays: string[], servers: string[]): Promise<string> {
    const imported = await this.requireSignIn().createDrive(DATABASE, name, relays, servers);
    await this.setUp(imported);
    return imported.recoveryKey;
  }

  async importRecoveryKey(recoveryKey: string): Promise<void> {
    await this.endSignIn();
    await this.setUp(await core.importRecoveryKey(DATABASE, recoveryKey.trim()));
  }

  // Browsing

  children(parent?: string): DriveItem[] {
    const local = this.requireDrive().local;
    const items: DriveItem[] = [];
    for (;;) {
      const page = local.children(parent, items.at(-1)?.id, PAGE);
      items.push(...page);
      if (page.length < PAGE) return items;
    }
  }

  item(id: string): DriveItem {
    return this.requireDrive().local.item(id);
  }

  /** The folders from the root down to `id`, included. */
  path(id?: string): DriveItem[] {
    const path: DriveItem[] = [];
    for (let current = id; current; ) {
      const item = this.item(current);
      path.unshift(item);
      current = item.parent;
    }
    return path;
  }

  // Changing

  async createFolder(parent: string | undefined, name: string): Promise<DriveItem> {
    const opened = this.requireDrive();
    const cancellation = new core.Cancellation();
    try {
      const item = await opened.writer.createItem(
        { request: crypto.randomUUID(), parent, name, isFolder: true },
        undefined,
        () => {},
        cancellation,
      );
      this.written();
      return item;
    } finally {
      cancellation.free();
    }
  }

  /** Uploads files into `parent`, making the folders they sit in. */
  async upload(parent: string | undefined, uploads: Upload[], collision: Collision): Promise<void> {
    // Promises, so files of the same new folder make it once.
    const folders = new Map<string, Promise<string | undefined>>([["", Promise.resolve(parent)]]);
    const folder = (path: string[]): Promise<string | undefined> => {
      const key = path.join("/");
      let id = folders.get(key);
      if (!id) {
        id = folder(path.slice(0, -1)).then(async (above) => {
          const name = path.at(-1)!;
          const existing = this.children(above).find((item) => item.isFolder && folded(item.name) === folded(name));
          return existing?.id ?? (await this.createFolder(above, name)).id;
        });
        folders.set(key, id);
      }
      return id;
    };
    await Promise.all(
      uploads.map(async ({ file, folders: path }) => {
        let target: string | undefined;
        try {
          target = await folder(path);
        } catch (error) {
          this.failTransfer("upload", file, error);
          return;
        }
        await this.uploadFile(target, file, collision);
      }),
    );
  }

  async rename(item: DriveItem, name: string): Promise<DriveItem> {
    return this.edit(item, { parent: item.parent, name });
  }

  async move(item: DriveItem, parent: string | undefined): Promise<DriveItem> {
    return this.edit(item, { parent, name: item.name });
  }

  /** Deletes for good, with everything a folder holds. */
  remove(item: DriveItem): void {
    this.requireDrive().writer.deleteItem(item.id, item.contentVersion, item.isFolder);
    this.written();
  }

  /** The file's content, decrypted, for the page to save or show. `id` names its transfer. */
  async download(item: DriveItem, id: string = crypto.randomUUID()): Promise<File> {
    const opened = this.requireDrive();
    const transfer: Transfer = { id, kind: "download", name: item.name, size: item.size, done: 0 };
    const cancellation = new core.Cancellation();
    // Before any await, so a cancel sent right after this call finds it.
    opened.transfers.set(transfer.id, cancellation);
    this.updateTransfer(transfer);
    try {
      const handle = await downloadFile();
      const sink = await handle.createSyncAccessHandle();
      try {
        await opened.reader.fetch(item.id, item.contentVersion, sink, (done) => this.progress(transfer, done), cancellation);
        sink.flush();
      } finally {
        sink.close();
      }
      this.removeTransfer(transfer.id);
      return new File([await handle.getFile()], item.name, { type: mimeType(item.name) });
    } catch (error) {
      if (isCancelled(error)) this.removeTransfer(transfer.id);
      else this.updateTransfer({ ...transfer, error: message(error) });
      throw error;
    } finally {
      opened.transfers.delete(transfer.id);
      cancellation.free();
    }
  }

  // Share links

  shares(item: DriveItem): ShareInfo[] {
    return this.requireDrive().local.shares(item.id);
  }

  /** `expires` in Unix milliseconds. */
  async createShare(item: DriveItem, label: string, expires?: number): Promise<ShareInfo> {
    const share = await this.requireDrive().writer.createShare(item.id, item.contentVersion, label, expires);
    this.written();
    return share;
  }

  /** `expires`: `undefined` keeps it, `null` removes it. */
  editShare(id: string, label: string, expires?: number | null): void {
    this.requireDrive().writer.editShare(id, label, expires);
    this.written();
  }

  async stopShare(id: string): Promise<void> {
    await this.requireDrive().writer.stopShare(id);
    this.written();
  }

  /** Stops a transfer, or clears one that failed. */
  cancelTransfer(id: string): void {
    const cancellation = this.opened?.transfers.get(id);
    if (cancellation) cancellation.cancel();
    else this.removeTransfer(id);
  }

  // Settings

  recoveryKey(): string {
    const opened = this.requireDrive();
    return opened.local.recoveryKey(opened.secret);
  }

  setHistoryDays(days: number): void {
    this.requireDrive().local.setHistoryDays(days);
    this.refresh();
  }

  /** Publishes the drive's new relays and servers, then opens it again with them. */
  async changeLists(relays: string[], servers: string[]): Promise<void> {
    const opened = this.requireDrive();
    const changed = await core.changeLists(opened.local, opened.secret, relays, servers);
    if (changed.relays || changed.servers) await this.reopen();
  }

  /** Removes the drive from this browser: its database and its secret. */
  async signOut(): Promise<void> {
    const opened = this.opened;
    if (!opened) return;
    const { id } = opened.local.info();
    await this.close();
    await forgetSecret(id);
    core.removeLocalDrive(DATABASE);
    this.setState({ phase: "signedOut", retired: false });
  }

  // Opening and closing

  private async open(): Promise<void> {
    const missing = unsupported();
    if (missing) {
      this.setState({ phase: "unsupported", reason: missing });
      return;
    }
    await init();
    await initStorage();
    await clearDownloads();
    await this.openDrive();
  }

  private async setUp(imported: ImportedDrive): Promise<void> {
    await keepSecret(imported.id, imported.secret);
    await this.endSignIn();
    await this.openDrive();
  }

  private async openDrive(): Promise<void> {
    const local = core.openLocalDrive(DATABASE);
    if (!local) {
      this.setState({ phase: "signedOut", retired: false });
      return;
    }
    const info = local.info();
    const secret = await keptSecret(info.id);
    if (!secret || core.driveKey(secret) !== info.driveKey) {
      local.free();
      this.setState({ phase: "signedOut", retired: false });
      return;
    }
    let sync: core.DriveSync;
    try {
      sync = await core.openDriveSync(local, secret);
    } catch (error) {
      local.free();
      throw error;
    }
    const opened: Opened = {
      local,
      secret,
      sync,
      writer: core.openItemWriter(local, secret),
      reader: core.openContentReader(local, secret),
      running: Promise.resolve(),
      keptDescriptor: false,
      transfers: new Map(),
    };
    this.opened = opened;
    this.setState({
      phase: "open",
      drive: {
        info,
        sync: { kind: "opening" },
        unsent: local.unsent(),
        historyDays: local.historyDays(),
        revision: 0,
        transfers: [],
      },
    });
    opened.running = sync.run((event) => this.synced(opened, event)).catch((error) => {
      if (this.opened === opened) this.setSync({ kind: "failed", message: message(error) });
    });
  }

  /** Stops everything that uses the drive, which stays set up. */
  private async close(): Promise<void> {
    const opened = this.opened;
    if (!opened) return;
    this.opened = undefined;
    clearInterval(opened.unsentTimer);
    for (const cancellation of opened.transfers.values()) cancellation.cancel();
    for (const background of [opened.fill, opened.collection]) background?.cancellation.cancel();
    await Promise.all([opened.fill?.done, opened.collection?.done]);
    opened.sync.stop();
    await opened.running;
    opened.writer.free();
    opened.reader.free();
    opened.sync.free();
    opened.local.free();
  }

  private async reopen(): Promise<void> {
    await this.close();
    await this.openDrive();
  }

  private synced(opened: Opened, event: SyncEvent): void {
    if (this.opened !== opened) return;
    switch (event.type) {
      case "syncing":
        this.setSync({ kind: "syncing" });
        break;
      case "synced":
        this.setSync({ kind: "synced", failed: event.failed });
        this.refresh();
        if (!opened.keptDescriptor) {
          opened.keptDescriptor = true;
          opened.sync.keepDescriptor().catch((error) => console.warn("Republishing the descriptor failed", error));
        }
        if (event.failed.length === 0) {
          this.startFill(opened);
          this.startCollection(opened);
        }
        break;
      case "itemsChanged":
        this.refresh(true);
        break;
      case "listsChanged":
        void this.reopen();
        break;
      case "retired":
        void this.close().then(() => this.setState({ phase: "signedOut", retired: true }));
        break;
      case "compacted":
        break;
    }
  }

  /** Copies the blobs the drive's new servers lack, once the session knows every version. */
  private startFill(opened: Opened): void {
    if (opened.fill || !opened.local.info().filling) return;
    if (opened.lastFill && Date.now() - opened.lastFill < HOUR) return;
    opened.lastFill = Date.now();
    const cancellation = new core.Cancellation();
    const done = core
      .fillServers(opened.local, opened.secret, (done, total) => this.setFill({ done, total }), cancellation)
      .then((report) => {
        if (report.lost > 0) console.warn(`${report.lost} blobs are on no server`);
        for (const failure of report.failed) console.warn("Copying a blob failed:", failure);
      })
      .catch((error) => {
        if (!isCancelled(error)) console.warn("Copying blobs failed", error);
      })
      .finally(() => {
        opened.fill = undefined;
        cancellation.free();
        if (this.opened === opened) {
          this.setFill(undefined);
          this.refresh();
        }
      });
    opened.fill = { cancellation, done };
  }

  /** Deletes the blobs no version referenced for 30 days, once a day, never during a fill. */
  private startCollection(opened: Opened): void {
    if (opened.collection || opened.fill) return;
    if (opened.lastCollection && Date.now() - opened.lastCollection < DAY) return;
    opened.lastCollection = Date.now();
    const cancellation = new core.Cancellation();
    const done = core
      .collectBlobs(opened.local, opened.secret, cancellation)
      .then((report) => {
        for (const failure of report.failed) console.warn("Deleting a blob failed:", failure);
      })
      .catch((error) => {
        if (!isCancelled(error)) console.warn("Collecting blobs failed", error);
      })
      .finally(() => {
        opened.collection = undefined;
        cancellation.free();
      });
    opened.collection = { cancellation, done };
  }

  // Writing

  private async uploadFile(parent: string | undefined, file: File, collision: Collision): Promise<void> {
    const opened = this.requireDrive();
    const transfer: Transfer = { id: crypto.randomUUID(), kind: "upload", name: file.name, size: file.size, done: 0 };
    const cancellation = new core.Cancellation();
    opened.transfers.set(transfer.id, cancellation);
    this.updateTransfer(transfer);
    const progress = (done: number) => this.progress(transfer, done);
    try {
      const siblings = this.children(parent);
      const same = siblings.find((item) => folded(item.name) === folded(file.name));
      if (same && !same.isFolder && collision === "replace") {
        const edit = { id: same.id, isFolder: false, baseContentVersion: same.contentVersion, modified: file.lastModified };
        await opened.writer.modifyItem(edit, file, progress, cancellation);
      } else {
        const name = freeName(file.name, new Set(siblings.map((item) => folded(item.name))));
        const item = { request: crypto.randomUUID(), parent, name, isFolder: false, modified: file.lastModified };
        await opened.writer.createItem(item, file, progress, cancellation);
      }
      this.removeTransfer(transfer.id);
      this.written();
    } catch (error) {
      if (isCancelled(error)) this.removeTransfer(transfer.id);
      else this.updateTransfer({ ...transfer, error: message(error) });
    } finally {
      opened.transfers.delete(transfer.id);
      cancellation.free();
    }
  }

  private async edit(item: DriveItem, location: { parent?: string; name: string }): Promise<DriveItem> {
    const opened = this.requireDrive();
    const cancellation = new core.Cancellation();
    try {
      const edit = { id: item.id, isFolder: item.isFolder, baseContentVersion: item.contentVersion, location };
      const edited = await opened.writer.modifyItem(edit, undefined, () => {}, cancellation);
      this.written();
      return edited;
    } finally {
      cancellation.free();
    }
  }

  /** The local database changed: the session sends it, listings read it again. */
  private written(): void {
    this.opened?.sync.written();
    this.refresh(true);
  }

  // State

  private failTransfer(kind: Transfer["kind"], file: File, error: unknown): void {
    this.updateTransfer({ id: crypto.randomUUID(), kind, name: file.name, size: file.size, done: 0, error: message(error) });
  }

  private progress(transfer: Transfer, done: number): void {
    transfer.done = done;
    this.updateDrive((drive) => ({
      ...drive,
      transfers: drive.transfers.map((t) => (t.id === transfer.id ? { ...t, done } : t)),
    }), true);
  }

  private updateTransfer(transfer: Transfer): void {
    this.updateDrive((drive) => {
      const others = drive.transfers.filter((t) => t.id !== transfer.id);
      return { ...drive, transfers: [...others, transfer] };
    });
  }

  private removeTransfer(id: string): void {
    this.updateDrive((drive) => ({ ...drive, transfers: drive.transfers.filter((t) => t.id !== id) }));
  }

  private setSync(sync: SyncStatus): void {
    this.updateDrive((drive) => ({ ...drive, sync }));
  }

  private setFill(fill: OpenDrive["fill"]): void {
    this.updateDrive((drive) => ({ ...drive, fill }), true);
  }

  /** Reads the drive's info again, and bumps the revision when items changed. */
  private refresh(itemsChanged = false): void {
    const opened = this.opened;
    if (!opened) return;
    const unsent = opened.local.unsent();
    this.updateDrive((drive) => ({
      ...drive,
      info: opened.local.info(),
      unsent,
      historyDays: opened.local.historyDays(),
      revision: drive.revision + (itemsChanged ? 1 : 0),
    }));
    if (unsent > 0) {
      opened.unsentTimer ??= setInterval(() => this.watchUnsent(opened), UNSENT_INTERVAL);
    }
  }

  private watchUnsent(opened: Opened): void {
    const unsent = opened.local.unsent();
    if (unsent === 0) {
      clearInterval(opened.unsentTimer);
      opened.unsentTimer = undefined;
    }
    this.updateDrive((drive) => (drive.unsent === unsent ? drive : { ...drive, unsent }));
  }

  private updateDrive(change: (drive: OpenDrive) => OpenDrive, throttled = false): void {
    if (this.state.phase !== "open") return;
    this.state = { phase: "open", drive: change(this.state.drive) };
    this.emit(throttled);
  }

  private setState(state: AppState): void {
    this.state = state;
    this.emit(false);
  }

  private emit(throttled: boolean): void {
    if (throttled) {
      this.repaint ??= setTimeout(() => this.emit(false), PROGRESS_INTERVAL);
      return;
    }
    clearTimeout(this.repaint);
    this.repaint = undefined;
    for (const listener of this.listeners) listener(this.state);
  }

  private async account(): Promise<Account> {
    const signIn = this.requireSignIn();
    const [drives, suggested] = await Promise.all([signIn.drives(), signIn.suggestedLists()]);
    return { user: signIn.user, remote: signIn.usesRemoteSigner, drives, suggested };
  }

  private requireSignIn(): core.SignIn {
    if (!this.signIn) throw new Error("Sign in first.");
    return this.signIn;
  }

  private requireDrive(): Opened {
    if (!this.opened) throw new Error("No drive is open.");
    return this.opened;
  }
}

/** The previous leader's worker may hold the files a moment after its lock goes. */
async function initStorage(): Promise<void> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await core.initStorage();
    } catch (error) {
      if (attempt === STORAGE_ATTEMPTS) throw error;
      await new Promise((resolve) => setTimeout(resolve, 100 * attempt));
    }
  }
}

export function unsupported(): string | undefined {
  if (!("locks" in navigator)) return "This browser lacks the Web Locks API.";
  if (!navigator.storage?.getDirectory) return "This browser lacks the origin private file system.";
  if (!("createSyncAccessHandle" in FileSystemFileHandle.prototype)) {
    return "This browser cannot write files from a worker.";
  }
  return undefined;
}

function isCancelled(error: unknown): boolean {
  return error instanceof Error && error.name === "Cancelled";
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
