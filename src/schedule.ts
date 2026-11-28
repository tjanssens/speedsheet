// The one module that may use timers. ESLint forbids setTimeout, setInterval and
// requestAnimationFrame everywhere else: a bare "wait a bit and hope" timer hides an ordering
// problem instead of fixing it, and timers created and dropped all over the code pile up.
// Every delay here has a name and a reason, and repeated requests collapse into one.

/** Run `fn` once, `ms` after the last call to `run()` (typing, cursor moves, presence). */
export class Debounced {
  private timer = 0;
  constructor(
    private readonly ms: number,
    private readonly fn: () => void,
  ) {}

  run() {
    window.clearTimeout(this.timer);
    this.timer = window.setTimeout(() => {
      this.timer = 0;
      this.fn();
    }, this.ms);
  }

  cancel() {
    window.clearTimeout(this.timer);
    this.timer = 0;
  }
}

/** Run `fn` once after `ms`. Returns a function that cancels it. */
export function after(ms: number, fn: () => void): () => void {
  const timer = window.setTimeout(fn, ms);
  return () => window.clearTimeout(timer);
}

/** Calls `fn` at most once per animation frame, however often `request()` is called. */
export class FrameTask {
  private frame = 0;
  constructor(private readonly fn: () => void) {}

  request() {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.fn();
    });
  }

  get pending() {
    return this.frame !== 0;
  }

  cancel() {
    if (this.frame) cancelAnimationFrame(this.frame);
    this.frame = 0;
  }
}
