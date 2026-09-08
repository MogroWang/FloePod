/** The destination's acknowledgement is the only authority for consuming a cut token. */
export async function dragOut<Token>(
  mode: "copy" | "move",
  effects: {
    setActive: (active: boolean) => Promise<unknown>;
    prepare: () => Promise<Token>;
    drag: (mode: "copy" | "move") => Promise<boolean>;
    finalize: (token: Token) => Promise<unknown>;
    cancel: (token: Token) => Promise<unknown>;
    cleanupFailed: (error: unknown) => void;
  },
): Promise<"copied" | "moved" | "cancelled" | "source-cleanup-failed"> {
  let token: Token | undefined;
  try {
    await effects.setActive(true);
    if (mode === "move") token = await effects.prepare();
    if (!(await effects.drag(mode))) return "cancelled";
    if (mode === "copy") return "copied";
    try {
      await effects.finalize(token!);
      token = undefined;
      return "moved";
    } catch (error) {
      effects.cleanupFailed(error);
      return "source-cleanup-failed";
    }
  } finally {
    if (token !== undefined) await effects.cancel(token).catch(effects.cleanupFailed);
    await effects.setActive(false).catch(effects.cleanupFailed);
  }
}
