import { confirm } from "@tauri-apps/plugin-dialog";

// Never use window.confirm: the plugin's injected shim is async and some
// versions call an unregistered command. The public API uses `message`.
let pending = false;

export async function confirmAction(message: string): Promise<boolean> {
  if (pending) return false;
  pending = true;
  try {
    return (await confirm(message, { title: "Satelite", kind: "warning" })) === true;
  } catch (error) {
    // Fail closed: an unavailable dialog must never authorize deletion.
    console.error("Confirmation dialog failed; action cancelled", error);
    return false;
  } finally {
    pending = false;
  }
}
