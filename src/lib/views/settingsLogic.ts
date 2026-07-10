export interface UpdateActionTarget {
  status: string;
  checkForUpdates: () => Promise<boolean>;
  downloadAndInstall: () => Promise<void>;
}

export type SaveAndReloadResult<T> =
  | { ok: true; value: T; error: null }
  | { ok: false; value: T; error: string };

function errorMessage(error: unknown): string {
  if (typeof error === "string" && error.trim()) return error;
  if (error instanceof Error && error.message) return error.message;
  return String(error);
}

export async function performUpdateAction(
  updater: UpdateActionTarget,
): Promise<"checked" | "installed"> {
  if (updater.status === "available") {
    await updater.downloadAndInstall();
    return "installed";
  }

  await updater.checkForUpdates();
  return "checked";
}

export async function saveAndReload<T>(
  value: T,
  save: (value: T) => Promise<void>,
  load: () => Promise<T>,
  errorPrefix: string,
): Promise<SaveAndReloadResult<T>> {
  try {
    await save(value);
    return { ok: true, value: await load(), error: null };
  } catch (error) {
    return {
      ok: false,
      value,
      error: `${errorPrefix}: ${errorMessage(error)}`,
    };
  }
}

export function buildApiKeySavePayload<
  T extends { api_key: string; clear_api_key: boolean },
>(config: T, clearRequested: boolean): T {
  return {
    ...config,
    clear_api_key: clearRequested && config.api_key.trim() === "",
  };
}

export async function toggleBooleanPreference(
  current: boolean,
  apply: (next: boolean) => Promise<void>,
  errorPrefix: string,
): Promise<{ value: boolean; error: string | null }> {
  const next = !current;
  try {
    await apply(next);
    return { value: next, error: null };
  } catch (error) {
    return {
      value: current,
      error: `${errorPrefix}: ${errorMessage(error)}`,
    };
  }
}

export interface DebouncedSaver<T> {
  schedule: (value: T) => void;
  flush: () => Promise<void>;
}

export function createDebouncedSaver<T>(
  delayMs: number,
  save: (value: T) => Promise<void>,
): DebouncedSaver<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let hasPending = false;
  let pendingValue!: T;

  function clearTimer() {
    if (!timer) return;
    clearTimeout(timer);
    timer = null;
  }

  async function flush() {
    clearTimer();
    if (!hasPending) return;
    const value = pendingValue;
    hasPending = false;
    await save(value);
  }

  function schedule(value: T) {
    pendingValue = value;
    hasPending = true;
    clearTimer();
    timer = setTimeout(() => {
      timer = null;
      void flush().catch(() => {});
    }, delayMs);
  }

  return { schedule, flush };
}
