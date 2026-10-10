export type NodeDeleteKey = Pick<KeyboardEvent,
  "key" | "repeat" | "defaultPrevented" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">;

export function shouldDeleteSelectedNodes(event: NodeDeleteKey, context: {
  editing: boolean;
  dialogOpen: boolean;
  readOnly: boolean;
  busy: boolean;
  selectedCount: number;
}): boolean {
  return event.key === "Delete" && !event.repeat && !event.defaultPrevented &&
    !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey &&
    !context.editing && !context.dialogOpen && !context.readOnly && !context.busy &&
    context.selectedCount > 0;
}
