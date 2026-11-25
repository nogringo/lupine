/** Where the service worker serves shared files, under its scope: `stream/<id>/<name>`. */
export const STREAM = "stream/";

/** Sent by a page that a forced reload left without a service worker. */
export const CLAIM = "lupine-claim";

export const LINK_REQUEST = "lupine-share-link";

/**
 * The service worker asks the pages for the file a stream URL names, so it
 * keeps no key, and still serves after the browser stopped and restarted it.
 * The page that shows it answers with `SharedFile.shown`, the others with null.
 */
export interface LinkRequest {
  type: typeof LINK_REQUEST;
  id: string;
}
