import type { CoreKind, ProtocolCoreItem } from "./types";

const CORE_KINDS = ["singbox", "xray", "mihomo"];

export function asCoreKind(value: string | null | undefined): CoreKind | undefined {
  return value && CORE_KINDS.includes(value) ? value as CoreKind : undefined;
}

/** Unknown persisted values are displayed as automatic without rewriting them. */
export function protocolDefaultValue(rows: ProtocolCoreItem[] | undefined, protocol: string): string {
  const core = rows?.find((row) => row.protocol === protocol)?.core;
  return core && CORE_KINDS.includes(core) ? core : "auto";
}

export function updateProtocolDefault(rows: ProtocolCoreItem[] | undefined, protocol: string, core: string): ProtocolCoreItem[] {
  const rest = (rows ?? []).filter((row) => row.protocol !== protocol);
  return CORE_KINDS.includes(core) ? [...rest, { protocol, core }] : rest;
}
