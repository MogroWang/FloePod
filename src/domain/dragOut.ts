/** The destination's acknowledgement is the only authority for consuming a cut token. */
export type DragOutOutcome = "copied" | "moved" | "cancelled" | "ignored" | "source-cleanup-failed";

export async function dragOut<Token>(
  mode: "copy" | "move",
  effects: {
    setActive: (active: boolean) => Promise<unknown>;
    prepare: () => Promise<Token>;
    drag: (mode: "copy" | "move") => Promise<boolean>;
    /** 返回清理结果；全部被拒绝说明落点是应用自己（拖回自身）。 */
    finalize: (token: Token) => Promise<{ deleted: number; refused: number } | void>;
    cancel: (token: Token) => Promise<unknown>;
    cleanupFailed: (error: unknown) => void;
  },
): Promise<DragOutOutcome> {
  let token: Token | undefined;
  try {
    await effects.setActive(true);
    if (mode === "move") token = await effects.prepare();
    if (!(await effects.drag(mode))) return "cancelled";
    if (mode === "copy") return "copied";
    try {
      const outcome = await effects.finalize(token!);
      token = undefined;
      /* 一个源文件都没清理：落点在本应用内（拖回同一个匣或自有窗口），
         文件保持原样，按“忽略”返回，不能报告成已剪切移出。 */
      if (outcome && outcome.deleted === 0 && outcome.refused > 0) return "ignored";
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
