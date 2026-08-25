import type { DropAction, ModifierState } from "./types.ts";

export type ConcreteDropAction = Exclude<DropAction, "ask">;

export function dropActionFor(
  modifiers: ModifierState,
  configured: DropAction,
): ConcreteDropAction | null {
  if (modifiers.ctrl) return "copy";
  if (modifiers.shift) return "move";
  if (modifiers.alt) return "shortcut";
  return configured === "ask" ? null : configured;
}

