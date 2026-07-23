// A key's full value is returned only once at creation, so cache it per model
// to reuse across visits instead of minting a new key each time.
const PREFIX = "playground-key:";

export function getStoredPlaygroundKey(modelId: string): string | null {
  if (!modelId || typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(PREFIX + modelId);
  } catch {
    return null;
  }
}

export function storePlaygroundKey(modelId: string, key: string): void {
  if (!modelId || typeof window === "undefined") return;
  try {
    window.localStorage.setItem(PREFIX + modelId, key);
  } catch {
    // localStorage unavailable (private mode / quota) — in-memory reuse still works.
  }
}

export function clearStoredPlaygroundKey(modelId: string): void {
  if (!modelId || typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(PREFIX + modelId);
  } catch {
    // ignore
  }
}
