/** Serializes effects that share an owner; a failed request never poisons later work. */
export class AsyncQueue<Key> {
  private readonly tails = new Map<Key, Promise<void>>();

  enqueue<Value>(key: Key, run: () => Promise<Value>): Promise<Value> {
    const result = (this.tails.get(key) ?? Promise.resolve()).then(run);
    const tail = result.then(
      () => undefined,
      () => undefined,
    );
    this.tails.set(key, tail);
    void tail.then(() => {
      if (this.tails.get(key) === tail) this.tails.delete(key);
    });
    return result;
  }

  async drain(): Promise<void> {
    while (this.tails.size) await Promise.all(this.tails.values());
  }
}

/** Request revisions are separate from values: A -> B -> A is still three edits. */
export class Revision {
  private value = 0;
  next(): number {
    return ++this.value;
  }
  isCurrent(revision: number): boolean {
    return revision === this.value;
  }
}
