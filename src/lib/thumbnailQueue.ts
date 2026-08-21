/** Keep native image work bounded across every ThumbImg instance in this WebView. */
const MAX_CONCURRENT_THUMBNAILS = 3;

let active = 0;
const queue: Array<() => void> = [];

function pump() {
  while (active < MAX_CONCURRENT_THUMBNAILS) {
    const start = queue.shift();
    if (!start) return;
    active += 1;
    start();
  }
}

export function withThumbnailSlot<T>(task: () => Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    queue.push(() => {
      void task()
        .then(resolve, reject)
        .finally(() => {
          active -= 1;
          pump();
        });
    });
    pump();
  });
}
