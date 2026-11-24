// How tabs share the drive: the worker of the tab holding DRIVE_LOCK, the
// leader, serves every tab. The hub, a shared worker, passes it their ports.

export const DRIVE_LOCK = "lupine-drive";

/** Each tab holds its own for its life, so the leader learns when it closes. */
export function tabLock(tab: string): string {
  return `lupine-tab-${tab}`;
}

/** The hub posts here when a new leader takes over, so followers connect to it. */
export const ANNOUNCEMENTS = "lupine-leader";

export type HubMessage =
  | { type: "lead"; accept: MessagePort }
  | { type: "follow"; tab: string; port: MessagePort };

/** A follower's port, from the hub to the leader's worker. */
export interface Follower {
  tab: string;
  port: MessagePort;
}
