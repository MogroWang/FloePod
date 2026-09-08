export type Draft<Value> = { value: Value; revision: number };

/** Acknowledgement clears only the edit that was saved; failures retain retryable input. */
export function createDrafts<Key, Value>(entries = new Map<Key, Draft<Value>>()) {
  let revision = 0;
  return {
    get: (key: Key) => entries.get(key)?.value,
    preview(key: Key, value: Value): Draft<Value> {
      const draft = { value, revision: ++revision };
      entries.set(key, draft);
      return draft;
    },
    current: (key: Key) => entries.get(key),
    accept(key: Key, saved: Draft<Value>) {
      if (entries.get(key)?.revision === saved.revision) entries.delete(key);
    },
  };
}
