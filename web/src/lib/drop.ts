import type { Upload } from "./state";

/** The files of a drop, folders walked, each with the folders it sits in. */
export async function dropped(transfer: DataTransfer): Promise<Upload[]> {
  const entries = [...transfer.items]
    .map((item) => item.webkitGetAsEntry?.())
    .filter((entry): entry is FileSystemEntry => entry != null);
  if (entries.length === 0) return [...transfer.files].map((file) => ({ file, folders: [] }));
  const uploads: Upload[] = [];
  await Promise.all(entries.map((entry) => walk(entry, [], uploads)));
  return uploads;
}

async function walk(entry: FileSystemEntry, folders: string[], into: Upload[]): Promise<void> {
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
    if (file.name !== ".DS_Store") into.push({ file, folders });
    return;
  }
  const reader = (entry as FileSystemDirectoryEntry).createReader();
  const inside = [...folders, entry.name];
  // Each call returns a batch, an empty one at the end.
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
    if (batch.length === 0) break;
    await Promise.all(batch.map((child) => walk(child, inside, into)));
  }
}

/** Files from an `<input webkitdirectory>`, which names them by relative path. */
export function picked(files: FileList): Upload[] {
  return [...files].map((file) => {
    const parts = (file.webkitRelativePath || file.name).split("/");
    return { file, folders: parts.slice(0, -1) };
  });
}
