export type Subscribe = (
  method: string,
  params: Record<string, unknown>,
  onItem: (item: unknown) => void,
  onError: (error: Error) => void,
) => Promise<() => void>;

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error('Device stream failed');
}

export function watchWithRetry(
  subscribe: Subscribe,
  method: string,
  params: Record<string, unknown>,
  onItem: (item: unknown) => void,
  onError: (error: Error) => void,
  retryDelayMs = 2000,
): () => void {
  let disposed = false;
  let stop: (() => void) | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const retry = () => {
    if (disposed || timer) return;
    stop?.();
    stop = undefined;
    timer = setTimeout(() => { timer = undefined; void start(); }, retryDelayMs);
  };

  const fail = (error: unknown) => {
    if (disposed) return;
    onError(asError(error));
    retry();
  };

  const start = async () => {
    try {
      const cancel = await subscribe(method, params, item => {
        if (disposed) return;
        try { onItem(item); } catch (error) { fail(error); }
      }, fail);
      if (disposed || timer) cancel();
      else stop = cancel;
    } catch (error) { fail(error); }
  };

  void start();
  return () => {
    disposed = true;
    if (timer) clearTimeout(timer);
    stop?.();
  };
}
