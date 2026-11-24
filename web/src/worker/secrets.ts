// The drive secret, which the Apple app keeps in the keychain.
const DATABASE = "lupine";
const STORE = "secrets";

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function run<T>(mode: IDBTransactionMode, use: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await open();
  try {
    return await new Promise<T>((resolve, reject) => {
      const request = use(db.transaction(STORE, mode).objectStore(STORE));
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  } finally {
    db.close();
  }
}

export function secret(drive: string): Promise<string | undefined> {
  return run("readonly", (store) => store.get(drive));
}

export async function keepSecret(drive: string, secret: string): Promise<void> {
  await run("readwrite", (store) => store.put(secret, drive));
}

export async function forgetSecret(drive: string): Promise<void> {
  await run("readwrite", (store) => store.delete(drive));
}
