// Downloads land in OPFS files, which the page then saves. They stay until
// the next launch: the browser may still be copying one.

async function lupine(): Promise<FileSystemDirectoryHandle> {
  const root = await navigator.storage.getDirectory();
  return root.getDirectoryHandle("lupine", { create: true });
}

export async function clearDownloads(): Promise<void> {
  try {
    await (await lupine()).removeEntry("downloads", { recursive: true });
  } catch {
    // None yet.
  }
}

export async function downloadFile(): Promise<FileSystemFileHandle> {
  const downloads = await (await lupine()).getDirectoryHandle("downloads", { create: true });
  return downloads.getFileHandle(crypto.randomUUID(), { create: true });
}
