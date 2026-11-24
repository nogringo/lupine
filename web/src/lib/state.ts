import type { DriveInfo, DriveLists, RemoteDrive } from "../../core/lupine";

export type { DriveInfo, DriveItem, DriveLists, RemoteDrive } from "../../core/lupine";

export type SyncStatus =
  | { kind: "opening" }
  | { kind: "syncing" }
  | { kind: "synced"; failed: string[] }
  | { kind: "failed"; message: string };

export interface Transfer {
  id: string;
  kind: "upload" | "download";
  name: string;
  size: number;
  done: number;
  error?: string;
}

export interface OpenDrive {
  info: DriveInfo;
  sync: SyncStatus;
  /** Changes no relay took yet. */
  unsent: number;
  historyDays: number;
  /** Grows whenever items change, so listings know to read again. */
  revision: number;
  transfers: Transfer[];
  /** Blobs being copied to the drive's servers. */
  fill?: { done: number; total: number };
}

export type AppState =
  | { phase: "starting" }
  | { phase: "unsupported"; reason: string }
  /** Another tab holds the drive, and this browser has no shared worker to reach it. This one takes over when it closes. */
  | { phase: "elsewhere" }
  /** `retired`: another device changed the drive key. */
  | { phase: "signedOut"; retired: boolean }
  | { phase: "open"; drive: OpenDrive };

export interface Account {
  /** npub. */
  user: string;
  /** Requests wait for the user's approval in a signer. */
  remote: boolean;
  drives: RemoteDrive[];
  /** The user's own inbox relays and Blossom servers, for a new drive. */
  suggested: DriveLists;
}

/** A file to upload, under `folders` within the target folder. */
export interface Upload {
  file: File;
  folders: string[];
}

/** `replace`: same-name files get a new version rather than a sibling. */
export type Collision = "replace" | "keepBoth";

export type Nip07Call = (method: string, params: unknown[]) => Promise<unknown>;
