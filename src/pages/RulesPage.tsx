import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { listen } from "@tauri-apps/api/event";
import {
  createRuleSet,
  deleteRuleSet,
  getRuleSet,
  getSettings,
  listAllNodes,
  listRemoteRuleItems,
  listRuleSets,
  removeRule,
  refreshRemoteRuleSet,
  renameRuleSet,
  reorderRuleSets,
  resetRuleSet,
  resetBuiltinRuleSet,
  saveRule,
  setRuleEnabled,
  setRuleSetEnabled,
  setRuleSetDnsStrategy,
  setRuleSetStrategy,
  updateSettings,
} from "../api";
import { GlassButton } from "../components/GlassButton";
import { SolidSelect } from "../components/SolidSelect";
import { GlassSeg } from "../components/GlassSeg";
import { GlassSwitchControl } from "../components/GlassSwitchControl";
import { useI18n } from "../i18n";
import type {
  ProxyNode,
  Rule,
  RuleSetDnsStrategy,
  RuleSetStrategy,
  RuleSetSummary,
  RemoteRulePage,
  RuleTarget,
  RuleType,
} from "../types";

type RouteFinal = "proxy" | "direct" | "block";

const TYPE_OPTS: { value: RuleType; label: string }[] = [
  { value: "domain_suffix", label: "DOMAIN-SUFFIX" },
  { value: "domain", label: "DOMAIN" },
  { value: "domain_keyword", label: "DOMAIN-KEYWORD" },
  { value: "ip_cidr", label: "IP-CIDR" },
  { value: "process", label: "PROCESS" },
];

const REMOTE_PAGE_SIZE = 100;

interface Props {
  /** Hide page chrome when embedded under Settings. */
  embedded?: boolean;
}

export function RulesPage({ embedded = false }: Props) {
  const { t } = useI18n();
  const [sets, setSets] = useState<RuleSetSummary[]>([]);
  const [viewSetId, setViewSetId] = useState<string | null>(null);
  const [rules, setRules] = useState<Rule[]>([]);
  const [filter, setFilter] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [routeFinal, setRouteFinal] = useState<RouteFinal>("proxy");
  const [finalBusy, setFinalBusy] = useState(false);

  const [editOpen, setEditOpen] = useState(false);
  const [editRule, setEditRule] = useState<Rule | null>(null);
  const [ruleType, setRuleType] = useState<RuleType>("domain_suffix");
  const [payload, setPayload] = useState("");
  const [target, setTarget] = useState<RuleTarget>("proxy");
  const [pinNodeId, setPinNodeId] = useState<string>("");
  const [nodeQuery, setNodeQuery] = useState("");
  const [smartInclude, setSmartInclude] = useState("");
  const [smartExclude, setSmartExclude] = useState("");
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
  const [enabled, setEnabled] = useState(true);
  const [busy, setBusy] = useState(false);

  /** New rule-set modal (window.prompt is unreliable in Tauri WebView). */
  const [newSetOpen, setNewSetOpen] = useState(false);
  const [newSetName, setNewSetName] = useState("自定义规则集");
  const [newSetKind, setNewSetKind] = useState<"local" | "remote">("local");
  const [newSetUrl, setNewSetUrl] = useState("");
  const [newSetTarget, setNewSetTarget] = useState<RouteFinal>("proxy");
  const [newSetBusy, setNewSetBusy] = useState(false);
  const [renameSetTarget, setRenameSetTarget] = useState<RuleSetSummary | null>(null);
  const [renameSetName, setRenameSetName] = useState("");
  const [renameSetBusy, setRenameSetBusy] = useState(false);
  /** Row ⋮ menu open for this rule id */
  const [menuRuleId, setMenuRuleId] = useState<string | null>(null);
  /** Rule-set card ⋮ menu open for this set id. */
  const [menuSetId, setMenuSetId] = useState<string | null>(null);
  const [remoteBusyIds, setRemoteBusyIds] = useState<Set<string>>(new Set());
  /** Rule-set ids with a background enable/disable restart in flight. */
  const [togglingIds, setTogglingIds] = useState<Set<string>>(new Set());
  const toggleGenRef = useRef<Map<string, number>>(new Map());
  const togglePrevRef = useRef<Map<string, boolean>>(new Map());
  const [remotePage, setRemotePage] = useState<RemoteRulePage | null>(null);
  const [remotePageIndex, setRemotePageIndex] = useState(0);
  const [remoteRulesLoading, setRemoteRulesLoading] = useState(false);
  const [remoteRulesError, setRemoteRulesError] = useState<string | null>(null);

  /** Pointer drag (HTML5 DnD is unreliable in Tauri WebView). */
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const setsRef = useRef(sets);
  setsRef.current = sets;
  const dragRef = useRef<{
    id: string;
    startIds: string[];
    items: RuleSetSummary[];
    pointerId: number;
    moved: boolean;
  } | null>(null);
  const persistLockRef = useRef(false);

  const reloadSets = useCallback(async () => {
    const list = await listRuleSets();
    setSets(list);
    const preferred =
      list.find((s) => s.enabled)?.id ?? list[0]?.id ?? null;
    setViewSetId((prev) =>
      prev && list.some((set) => set.id === prev) ? prev : preferred,
    );
    return { list, preferred };
  }, []);

  const reloadRouteFinal = useCallback(async () => {
    try {
      const s = await getSettings();
      const rf = (s.route_final ?? "proxy").toLowerCase();
      if (rf === "direct" || rf === "block" || rf === "proxy") {
        setRouteFinal(rf);
      }
    } catch {
      /* keep default */
    }
  }, []);

  const onRouteFinalChange = async (next: RouteFinal) => {
    if (next === routeFinal || finalBusy) return;
    setFinalBusy(true);
    setError(null);
    try {
      const s = await updateSettings({ routeFinal: next });
      const rf = (s.route_final ?? next).toLowerCase();
      setRouteFinal(
        rf === "direct" || rf === "block" || rf === "proxy" ? rf : next,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setFinalBusy(false);
    }
  };

  const reloadRules = useCallback(async (setId: string | null) => {
    if (!setId) {
      setRules([]);
      return;
    }
    const set = await getRuleSet(setId);
    setRules([...set.rules].sort((a, b) => a.ord - b.ord));
  }, []);

  const reload = useCallback(async () => {
    setError(null);
    try {
      await reloadRouteFinal();
      const { list, preferred } = await reloadSets();
      const sid = viewSetId && list.some((set) => set.id === viewSetId)
        ? viewSetId
        : preferred;
      if (sid) {
        setViewSetId(sid);
        await reloadRules(sid);
      }
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setLoading(false);
    }
  }, [reloadSets, reloadRules, reloadRouteFinal, viewSetId]);

  useEffect(() => {
    void reload();
    void ensureNodesLoaded();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (viewSetId) void reloadRules(viewSetId);
  }, [viewSetId, reloadRules]);

  useEffect(() => {
    if (!menuRuleId && !menuSetId) return;
    function onDocPointerDown(e: PointerEvent) {
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("[data-rule-menu], [data-ruleset-menu]")) return;
      setMenuRuleId(null);
      setMenuSetId(null);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        setMenuRuleId(null);
        setMenuSetId(null);
      }
    }
    document.addEventListener("pointerdown", onDocPointerDown, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDocPointerDown, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuRuleId, menuSetId]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<{ id: string; status: string; error?: string | null }>(
      "remote-rule-set-status",
      (event) => {
        const { id, status, error: downloadError } = event.payload;
        setRemoteBusyIds((current) => {
          const next = new Set(current);
          if (status === "downloading") next.add(id);
          else next.delete(id);
          return next;
        });
        if (status === "error" && downloadError) setError(downloadError);
        void reloadSets();
      },
    ).then((dispose) => {
      unlisten = dispose;
    });
    return () => unlisten?.();
  }, [reloadSets]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<{
      id: string;
      enabled: boolean;
      status: "restarting" | "ready" | "error";
      error?: string | null;
    }>("rule-set-apply-status", (event) => {
      const { id, status, error: applyError } = event.payload;

      if (status === "restarting") {
        setTogglingIds((cur) => new Set(cur).add(id));
        return;
      }

      setTogglingIds((cur) => {
        const next = new Set(cur);
        next.delete(id);
        return next;
      });

      if (status === "ready") {
        void reloadSets();
        return;
      }

      // status === "error": roll back the switch to its pre-click value.
      // The store write already succeeded — only the visual state reverts.
      const prev = togglePrevRef.current.get(id);
      if (prev !== undefined) {
        setSets((list) =>
          list.map((s) => (s.id === id ? { ...s, enabled: prev } : s)),
        );
      }
      setError(applyError ?? "重启内核失败");
    }).then((dispose) => {
      unlisten = dispose;
    });
    return () => unlisten?.();
  }, [reloadSets]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return rules;
    return rules.filter(
      (r) =>
        r.payload.toLowerCase().includes(q) ||
        r.type.toLowerCase().includes(q) ||
        r.target.toLowerCase().includes(q) ||
        (r.node_name ?? "").toLowerCase().includes(q) ||
        (r.smart_include ?? []).some((k) => k.toLowerCase().includes(q)) ||
        (r.smart_exclude ?? []).some((k) => k.toLowerCase().includes(q)),
    );
  }, [rules, filter]);

  const nodeById = useMemo(() => {
    const m = new Map<string, ProxyNode>();
    for (const n of nodes) m.set(n.id, n);
    return m;
  }, [nodes]);

  const filteredNodes = useMemo(() => {
    const q = nodeQuery.trim().toLowerCase();
    if (!q) return nodes;
    return nodes.filter(
      (n) =>
        n.name.toLowerCase().includes(q) ||
        n.server.toLowerCase().includes(q) ||
        n.protocol.toLowerCase().includes(q),
    );
  }, [nodes, nodeQuery]);

  /** Split by whitespace (spaces / tabs / newlines). */
  function parseKeywords(raw: string): string[] {
    return raw
      .split(/\s+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  const smartKeywordOverlap = useMemo(() => {
    if (target !== "smart") return [] as string[];
    const include = parseKeywords(smartInclude);
    const exclude = parseKeywords(smartExclude);
    const out: string[] = [];
    for (const a of include) {
      const al = a.toLowerCase();
      if (exclude.some((b) => b.toLowerCase() === al) && !out.some((x) => x.toLowerCase() === al)) {
        out.push(a);
      }
    }
    return out;
  }, [target, smartInclude, smartExclude]);

  const smartMatchCount = useMemo(() => {
    if (target !== "smart") return 0;
    const include = parseKeywords(smartInclude);
    const exclude = parseKeywords(smartExclude);
    return nodes.filter((n) => {
      const name = n.name.toLowerCase();
      // Blacklist OR: any hit → skip
      if (exclude.some((k) => name.includes(k.toLowerCase()))) return false;
      // Whitelist OR: empty = allow all; else any hit allows
      if (include.length === 0) return true;
      return include.some((k) => name.includes(k.toLowerCase()));
    }).length;
  }, [target, smartInclude, smartExclude, nodes]);

  const viewSet = sets.find((s) => s.id === viewSetId);

  useEffect(() => {
    setRemotePageIndex(0);
  }, [viewSetId]);

  useEffect(() => {
    if (!viewSet?.remote?.local_path) {
      setRemotePage(null);
      setRemoteRulesError(null);
      setRemoteRulesLoading(false);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setRemoteRulesLoading(true);
      setRemoteRulesError(null);
      void listRemoteRuleItems(
        viewSet.id,
        remotePageIndex * REMOTE_PAGE_SIZE,
        REMOTE_PAGE_SIZE,
        filter,
      )
        .then((page) => {
          if (cancelled) return;
          if (page.total > 0 && page.items.length === 0 && remotePageIndex > 0) {
            setRemotePageIndex(0);
            return;
          }
          setRemotePage(page);
          if (!filter.trim()) {
            setSets((current) =>
              current.map((set) =>
                set.id === viewSet.id ? { ...set, rule_count: page.total } : set,
              ),
            );
          }
        })
        .catch((err) => {
          if (!cancelled) setRemoteRulesError(String(err));
        })
        .finally(() => {
          if (!cancelled) setRemoteRulesLoading(false);
        });
    }, 180);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [filter, remotePageIndex, viewSet?.id, viewSet?.remote?.local_path]);

  const targetOpts: { value: RuleTarget; label: string }[] = useMemo(
    () => [
      { value: "proxy", label: t("rules.targetProxy") },
      { value: "direct", label: t("rules.targetDirect") },
      { value: "block", label: t("rules.targetBlock") },
      { value: "node", label: t("rules.targetNode") },
      { value: "smart", label: t("rules.targetSmart") },
    ],
    [t],
  );

  function targetLabel(r: Rule): { text: string; stale: boolean; cls: string } {
    if (r.target === "smart") {
      const parts: string[] = [t("rules.smartLabel")];
      const inc = (r.smart_include ?? []).filter(Boolean);
      const exc = (r.smart_exclude ?? []).filter(Boolean);
      if (inc.length) {
        parts.push(t("rules.smartLabelInc", { k: inc.join("/") }));
      }
      if (exc.length) {
        parts.push(t("rules.smartLabelExc", { k: exc.join("/") }));
      }
      return { text: parts.join(" · "), stale: false, cls: "target-smart" };
    }
    if (r.target !== "node") {
      return { text: r.target, stale: false, cls: `target-${r.target}` };
    }
    const id = r.node_id ?? "";
    const live = id ? nodeById.get(id) : undefined;
    if (live) {
      return { text: live.name, stale: false, cls: "target-node" };
    }
    const was = r.node_name?.trim() || id || "—";
    return {
      text: t("rules.nodeStaleLabel", { name: was }),
      stale: true,
      cls: "target-stale",
    };
  }

  async function ensureNodesLoaded() {
    try {
      const list = await listAllNodes();
      setNodes(list);
    } catch {
      setNodes([]);
    }
  }

  function openCreate() {
    setEditRule(null);
    setRuleType("domain_suffix");
    setPayload("");
    setTarget(
      viewSet?.strategy === "smart"
        ? "proxy"
        : (viewSet?.strategy ?? "proxy") as RuleTarget,
    );
    setPinNodeId("");
    setNodeQuery("");
    setSmartInclude("");
    setSmartExclude("");
    setEnabled(true);
    setEditOpen(true);
    void ensureNodesLoaded();
  }

  function openEdit(r: Rule) {
    setEditRule(r);
    setRuleType(r.type);
    setPayload(r.payload);
    setTarget(r.target);
    setPinNodeId(r.node_id ?? "");
    setNodeQuery("");
    setSmartInclude((r.smart_include ?? []).join(" "));
    setSmartExclude((r.smart_exclude ?? []).join(" "));
    setEnabled(r.enabled);
    setEditOpen(true);
    void ensureNodesLoaded();
  }

  async function onSave(e: FormEvent) {
    e.preventDefault();
    if (!viewSetId || !payload.trim()) return;
    const effectiveTarget = viewSet?.strategy === "smart"
      ? target
      : (viewSet?.strategy ?? "proxy") as RuleTarget;
    if (effectiveTarget === "node" && !pinNodeId.trim()) {
      setError(t("rules.needNode"));
      return;
    }
    if (effectiveTarget === "smart" && smartKeywordOverlap.length > 0) {
      setError(
        t("rules.smartKeywordConflict", { k: smartKeywordOverlap.join("、") }),
      );
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await saveRule({
        setId: viewSetId,
        id: editRule?.id ?? null,
        ruleType,
        payload: payload.trim(),
        target: effectiveTarget,
        ord: editRule?.ord ?? null,
        enabled,
        nodeId: effectiveTarget === "node" ? pinNodeId : null,
        smartInclude: effectiveTarget === "smart" ? parseKeywords(smartInclude) : null,
        smartExclude: effectiveTarget === "smart" ? parseKeywords(smartExclude) : null,
      });
      setEditOpen(false);
      await reloadRules(viewSetId);
      await reloadSets();
      void ensureNodesLoaded();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setBusy(false);
    }
  }

  function nextToggleGen(id: string) {
    const g = (toggleGenRef.current.get(id) ?? 0) + 1;
    toggleGenRef.current.set(id, g);
    return g;
  }

  async function onToggleSet(id: string, nextEnabled: boolean) {
    const current = sets.find((s) => s.id === id);
    if (!current || current.enabled === nextEnabled) return;

    const prevEnabled = current.enabled;
    const gen = nextToggleGen(id);
    togglePrevRef.current.set(id, prevEnabled);

    // Optimistic: flip the switch immediately, restart happens in the
    // background (see the `rule-set-apply-status` listener below).
    setSets((list) =>
      list.map((s) => (s.id === id ? { ...s, enabled: nextEnabled } : s)),
    );
    setTogglingIds((cur) => new Set(cur).add(id));
    setError(null);

    try {
      await setRuleSetEnabled(id, nextEnabled); // resolves once persisted, not once restarted
    } catch (err) {
      // Only the latest click for this id should roll back / clear pending.
      if (toggleGenRef.current.get(id) === gen) {
        setSets((list) =>
          list.map((s) => (s.id === id ? { ...s, enabled: prevEnabled } : s)),
        );
        setTogglingIds((cur) => {
          const next = new Set(cur);
          next.delete(id);
          return next;
        });
        setError(typeof err === "string" ? err : String(err));
      }
    }
  }

  async function onStrategyChange(strategy: RuleSetStrategy) {
    if (!viewSetId || !viewSet || strategy === viewSet.strategy || busy) return;
    if (
      viewSet.strategy === "smart" &&
      strategy !== "smart" &&
      !confirm("切换为整组策略后，所有单项将统一使用该策略。继续吗？")
    ) return;
    setBusy(true);
    setError(null);
    try {
      await setRuleSetStrategy(viewSetId, strategy);
      await Promise.all([reloadSets(), reloadRules(viewSetId)]);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function onDnsStrategyChange(strategy: RuleSetDnsStrategy) {
    if (!viewSetId || !viewSet || strategy === viewSet.dns_strategy || busy) return;
    setBusy(true);
    setError(null);
    try {
      await setRuleSetDnsStrategy(viewSetId, strategy);
      await Promise.all([reloadSets(), reloadRules(viewSetId)]);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function onResetAllBuiltin() {
    if (!confirm("完全恢复所有内置规则集？\n用户创建和导入的规则集不会被删除。")) return;
    setBusy(true);
    setError(null);
    try {
      await resetBuiltinRuleSet();
      setViewSetId(null);
      await reload();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  function openNewSet() {
    setNewSetName("自定义规则集");
    setNewSetKind("local");
    setNewSetUrl("");
    setNewSetTarget("proxy");
    setNewSetOpen(true);
    setError(null);
  }

  async function onCreateSet(e: FormEvent) {
    e.preventDefault();
    const name = newSetName.trim();
    if (!name) {
      setError("请输入规则集名称");
      return;
    }
    setNewSetBusy(true);
    setError(null);
    try {
      if (newSetKind === "remote" && !/^https?:\/\//i.test(newSetUrl.trim())) {
        setError("请输入以 http:// 或 https:// 开头的远程规则集 URL");
        return;
      }
      const set = await createRuleSet(
        name,
        newSetKind === "remote" ? newSetUrl.trim() : null,
        newSetKind === "remote" ? newSetTarget : null,
      );
      const list = await listRuleSets();
      setSets(list);
      setViewSetId(set.id);
      setRules([]);
      setNewSetOpen(false);
      if (newSetKind === "remote") void onRefreshRemoteSet(set.id);
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setNewSetBusy(false);
    }
  }

  async function onRefreshRemoteSet(id: string) {
    setRemoteBusyIds((current) => new Set(current).add(id));
    setError(null);
    try {
      await refreshRemoteRuleSet(id);
      await reloadSets();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
      await reloadSets().catch(() => undefined);
    } finally {
      setRemoteBusyIds((current) => {
        const next = new Set(current);
        next.delete(id);
        return next;
      });
    }
  }

  function openRenameSet(target: RuleSetSummary) {
    setMenuSetId(null);
    setRenameSetTarget(target);
    setRenameSetName(target.name);
  }

  async function onRenameSet(e: FormEvent) {
    e.preventDefault();
    if (!renameSetTarget || !renameSetName.trim() || renameSetBusy) return;
    setRenameSetBusy(true);
    setError(null);
    try {
      await renameRuleSet(renameSetTarget.id, renameSetName.trim());
      await reloadSets();
      setRenameSetTarget(null);
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setRenameSetBusy(false);
    }
  }

  async function onDeleteSet(target: RuleSetSummary | null | undefined = viewSet) {
    if (!target || busy) return;
    if (!confirm(`删除规则集「${target.name}」？`)) return;
    setBusy(true);
    setError(null);
    try {
      await deleteRuleSet(target.id);
      if (viewSetId === target.id) setViewSetId(null);
      await reload();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setBusy(false);
    }
  }

  function isFactorySet(s: RuleSetSummary | undefined | null) {
    if (!s) return false;
    return s.id.startsWith("builtin-");
  }

  async function onResetFactory(target: RuleSetSummary | null | undefined = viewSet) {
    if (!target || !isFactorySet(target)) return;
    const name = target.name;
    if (
      !confirm(
        `将「${name}」恢复为出厂配置文件？\n当前对该集的编辑会丢失。\n（重启程序不会自动重置，只会保留你的修改。）`,
      )
    ) {
      return;
    }
    try {
      await resetRuleSet(target.id);
      if (viewSetId === target.id) await reloadRules(target.id);
      await reloadSets();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    }
  }

  async function onToggle(rule: Rule) {
    if (!viewSetId) return;
    try {
      await setRuleEnabled(rule.id, !rule.enabled, viewSetId);
      await reloadRules(viewSetId);
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    }
  }

  async function onDelete(id: string) {
    if (!viewSetId || !confirm("删除该规则？")) return;
    try {
      await removeRule(id, viewSetId);
      await reloadRules(viewSetId);
      await reloadSets();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    }
  }

  async function persistOrder(items: RuleSetSummary[], startIds: string[]) {
    const orderedIds = items.map((s) => s.id);
    if (orderedIds.join("\0") === startIds.join("\0")) return;
    if (persistLockRef.current) return;
    persistLockRef.current = true;
    setError(null);
    try {
      const list = await reorderRuleSets(orderedIds);
      setSets(list);
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
      await reloadSets();
    } finally {
      persistLockRef.current = false;
    }
  }

  function onHandlePointerDown(id: string, e: ReactPointerEvent<HTMLElement>) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const snapshot = setsRef.current.map((s) => ({ ...s }));
    dragRef.current = {
      id,
      startIds: snapshot.map((s) => s.id),
      items: snapshot,
      pointerId: e.pointerId,
      moved: false,
    };
    setDraggingId(id);
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
  }

  function onHandlePointerMove(e: ReactPointerEvent<HTMLElement>) {
    const d = dragRef.current;
    if (!d || d.pointerId !== e.pointerId) return;
    e.preventDefault();
    // Use list geometry (midpoints), not elementFromPoint — after live reorder the
    // pointer often still sits on the dragged row and HTML5/DOM hit-tests stick.
    const nodes = Array.from(
      document.querySelectorAll<HTMLElement>("[data-ruleset-id]"),
    );
    if (nodes.length === 0) return;
    let targetIndex = nodes.length - 1;
    for (let i = 0; i < nodes.length; i++) {
      const rect = nodes[i].getBoundingClientRect();
      if (e.clientY < rect.top + rect.height / 2) {
        targetIndex = i;
        break;
      }
    }
    const fromIndex = d.items.findIndex((s) => s.id === d.id);
    if (fromIndex < 0 || fromIndex === targetIndex) return;
    const next = [...d.items];
    const [moved] = next.splice(fromIndex, 1);
    next.splice(targetIndex, 0, moved);
    d.items = next;
    d.moved = true;
    setSets(next);
  }

  function finishPointerDrag(e: ReactPointerEvent<HTMLElement>) {
    const d = dragRef.current;
    if (!d || d.pointerId !== e.pointerId) return;
    dragRef.current = null;
    setDraggingId(null);
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
    if (!d.moved) return;
    void persistOrder(d.items, d.startIds);
  }

  async function moveSet(id: string, dir: -1 | 1) {
    const list = setsRef.current;
    const idx = list.findIndex((s) => s.id === id);
    const to = idx + dir;
    if (idx < 0 || to < 0 || to >= list.length) return;
    const next = [...list];
    const [moved] = next.splice(idx, 1);
    next.splice(to, 0, moved);
    setSets(next);
    await persistOrder(
      next,
      list.map((s) => s.id),
    );
  }

  const body = (
    <>
      {!embedded && (
        <div className="rules-toolbar page-header">
          <div>
            <h1>{t("rules.title")}</h1>
            <p className="page-desc">{t("rules.desc")}</p>
          </div>
        </div>
      )}

      {error && <div className="banner error">{error}</div>}

      <div className="card rules-final-bar">
        <div className="rules-final-text">
          <strong>{t("rules.final")}</strong>
          <div className="muted" style={{ fontSize: 12 }}>
            {t("rules.finalHint")}
          </div>
        </div>
        <div className="rules-final-control">
          <GlassSeg
            value={routeFinal}
            ariaLabel={t("rules.final")}
            disabled={finalBusy}
            onChange={(v) => void onRouteFinalChange(v as RouteFinal)}
            options={[
              { value: "proxy", label: t("rules.finalProxy") },
              { value: "direct", label: t("rules.finalDirect") },
              { value: "block", label: t("rules.finalBlock") },
            ]}
          />
        </div>
      </div>

      <div className="rules-layout">
        <aside className="card ruleset-list rules-route-list">
          <div className="ruleset-list-actions">
            <GlassButton
              icon="+"
              onClick={openNewSet}
              title={t("rules.newSetTitle")}
            >
              {t("rules.newSet")}
            </GlassButton>
            <GlassButton
              icon="↺"
              onClick={() => void onResetAllBuiltin()}
              disabled={busy}
              title="删除并重新加载所有内置规则，不影响用户规则"
            >
              重置
            </GlassButton>
          </div>
          <div className="ruleset-list-title">
            {t("rules.sets")}
            <span className="ruleset-list-hint">{t("rules.dragHint")}</span>
          </div>
          {sets.map((s, index) => (
            <div
              key={s.id}
              data-ruleset-id={s.id}
              className={[
                "ruleset-item",
                viewSetId === s.id ? "selected" : "",
                draggingId === s.id ? "dragging" : "",
                draggingId && draggingId !== s.id ? "drag-targetable" : "",
              ]
                .filter(Boolean)
                .join(" ")}
              onClick={() => {
                if (dragRef.current?.moved) return;
                setViewSetId(s.id);
              }}
              role="listitem"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  setViewSetId(s.id);
                }
                if (e.key === "ArrowUp") {
                  e.preventDefault();
                  void moveSet(s.id, -1);
                }
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  void moveSet(s.id, 1);
                }
              }}
            >
              <div className="ruleset-item-top">
                <span
                  className="ruleset-drag"
                  title="按住拖动调整优先级"
                  role="button"
                  tabIndex={-1}
                  onPointerDown={(e) => onHandlePointerDown(s.id, e)}
                  onPointerMove={onHandlePointerMove}
                  onPointerUp={finishPointerDrag}
                  onPointerCancel={finishPointerDrag}
                >
                  ⋮⋮
                </span>
                <span className="ruleset-prio muted">{index + 1}</span>
                <span className="ruleset-name">{s.name}</span>
                {s.remote &&
                  (remoteBusyIds.has(s.id) ||
                    s.remote.download_status === "downloading") && (
                    <span
                      className="lat-spinner ruleset-download-spinner"
                      title="正在下载远程规则集"
                      aria-label="正在下载远程规则集"
                    />
                  )}
                {togglingIds.has(s.id) && (
                  <span
                    className="lat-spinner ruleset-toggle-spinner"
                    title="正在应用"
                    aria-label="正在应用"
                  />
                )}
                <GlassSwitchControl
                  checked={s.enabled}
                  size="sm"
                  title={s.enabled ? "关闭规则集" : "启用规则集"}
                  onClick={(e) => {
                    e.stopPropagation();
                  }}
                  onChange={(checked) => void onToggleSet(s.id, checked)}
                />
                <div className="rule-menu" data-ruleset-menu>
                  <button
                    type="button"
                    className="rule-menu-trigger"
                    aria-label={`${s.name} 操作`}
                    aria-haspopup="menu"
                    aria-expanded={menuSetId === s.id}
                    onClick={(e) => {
                      e.stopPropagation();
                      setMenuRuleId(null);
                      setMenuSetId((id) => (id === s.id ? null : s.id));
                    }}
                  >
                    ⋮
                  </button>
                  {menuSetId === s.id && (
                    <div
                      className={`rule-menu-pop ruleset-menu-pop${
                        index < Math.ceil(sets.length / 2) ? " open-down" : ""
                      }`}
                      role="menu"
                    >
                      <button
                        type="button"
                        role="menuitem"
                        className="rule-menu-item"
                        onClick={(e) => {
                          e.stopPropagation();
                          openRenameSet(s);
                        }}
                      >
                        重命名
                      </button>
                      {isFactorySet(s) && (
                        <button
                          type="button"
                          role="menuitem"
                          className="rule-menu-item"
                          onClick={(e) => {
                            e.stopPropagation();
                            setMenuSetId(null);
                            void onResetFactory(s);
                          }}
                        >
                          重置
                        </button>
                      )}
                      {s.remote && (
                        <button
                          type="button"
                          role="menuitem"
                          className="rule-menu-item"
                          disabled={remoteBusyIds.has(s.id)}
                          onClick={(e) => {
                            e.stopPropagation();
                            setMenuSetId(null);
                            void onRefreshRemoteSet(s.id);
                          }}
                        >
                          更新
                        </button>
                      )}
                      <button
                        type="button"
                        role="menuitem"
                        className="rule-menu-item danger"
                        disabled={
                          !!s.remote &&
                          (remoteBusyIds.has(s.id) ||
                            s.remote.download_status === "downloading")
                        }
                        onClick={(e) => {
                          e.stopPropagation();
                          setMenuSetId(null);
                          void onDeleteSet(s);
                        }}
                      >
                        删除
                      </button>
                    </div>
                  )}
                </div>
              </div>
              <div className="muted" style={{ fontSize: 12 }}>
                {s.rule_count} 条 · {s.strategy} · dns {s.strategy === "block" ? "reject" : s.dns_strategy}
              </div>
            </div>
          ))}
        </aside>

        <section className="rules-main">
          <div className="rules-toolbar card">
            <div className="header-actions rules-main-actions">
              <div className="rules-policy-control">
                <span className="muted rules-policy-label">路由</span>
                <GlassSeg
                  value={viewSet?.strategy ?? "proxy"}
                  ariaLabel="整组路由策略"
                  disabled={!viewSet || busy}
                  onChange={(value) => void onStrategyChange(value as RuleSetStrategy)}
                  options={[
                    { value: "proxy", label: "Proxy" },
                    { value: "direct", label: "Direct" },
                    { value: "block", label: "Block" },
                    ...(!viewSet?.remote ? [{ value: "smart", label: "智能" }] : []),
                  ]}
                />
              </div>
              {viewSet?.strategy !== "block" && <div className="rules-policy-control">
                <span className="muted rules-policy-label">DNS</span>
                <GlassSeg
                  value={viewSet?.dns_strategy ?? "remote"}
                  ariaLabel="整组 DNS 策略"
                  disabled={!viewSet || busy}
                  onChange={(value) => void onDnsStrategyChange(value as RuleSetDnsStrategy)}
                  options={[
                    { value: "local", label: "Local" },
                    { value: "domestic", label: "国内" },
                    { value: "remote", label: "远程" },
                  ]}
                />
              </div>}
              <div className="rules-toolbar-tail">
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  className="search rules-filter"
                  placeholder="过滤规则…"
                  value={filter}
                  onChange={(e) => {
                    setFilter(e.target.value);
                    setRemotePageIndex(0);
                  }}
                />
                <GlassButton
                  variant="primary"
                  icon="+"
                  onClick={openCreate}
                  disabled={!viewSetId || !!viewSet?.remote}
                  title={t("rules.addRuleTitle")}
                >
                  添加
                </GlassButton>
              </div>
            </div>
          </div>

          {loading ? (
            <div className="empty">加载中…</div>
          ) : viewSet?.remote ? (
            <>
              <div className="card remote-rule-status">
                <div className="muted remote-rule-url">{viewSet.remote.url}</div>
                <div className="muted">
                  {viewSet.remote.download_status === "downloading"
                    ? "正在由 Satelite 下载并校验，完成后自动加载。"
                    : viewSet.remote.download_status === "error"
                      ? `下载失败：${viewSet.remote.download_error ?? "未知错误"}`
                      : viewSet.remote.local_path
                        ? `已解析 ${viewSet.rule_count} 条规则，内核仅加载本地缓存。`
                        : "等待下载远程规则集。"}
                </div>
              </div>
              {remoteRulesLoading && !remotePage ? (
                <div className="empty muted">正在解析规则…</div>
              ) : remoteRulesError ? (
                <div className="empty card error">解析失败：{remoteRulesError}</div>
              ) : remotePage && remotePage.items.length > 0 ? (
                <div className="card table-wrap remote-rules-wrap">
                  <table className="remote-rules-table">
                    <colgroup>
                      <col className="col-index" />
                      <col className="col-kind" />
                      <col />
                    </colgroup>
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>类型</th>
                        <th>匹配条件</th>
                      </tr>
                    </thead>
                    <tbody>
                      {remotePage.items.map((item) => (
                        <tr key={item.index}>
                          <td className="rule-ord">{item.index}</td>
                          <td className="rule-type"><code>{item.kind}</code></td>
                          <td>
                            {item.complex ? (
                              <details className="remote-rule-details">
                                <summary title={item.summary}>
                                  {item.summary || "查看原始规则"}
                                </summary>
                                <pre>{item.raw}</pre>
                                {item.raw_truncated && (
                                  <div className="muted remote-rule-truncated">
                                    内容过长，仅显示前 4,000 个字符
                                  </div>
                                )}
                              </details>
                            ) : (
                              <div className="remote-rule-summary" title={item.summary}>
                                {item.summary}
                              </div>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  <div className="remote-rule-pagination">
                    <span className="muted">
                      {remotePage.offset + 1}–{remotePage.offset + remotePage.items.length} / {remotePage.total}
                    </span>
                    <GlassButton
                      onClick={() => setRemotePageIndex((page) => Math.max(0, page - 1))}
                      disabled={remotePageIndex === 0 || remoteRulesLoading}
                    >
                      上一页
                    </GlassButton>
                    <GlassButton
                      onClick={() => setRemotePageIndex((page) => page + 1)}
                      disabled={remotePage.offset + remotePage.items.length >= remotePage.total || remoteRulesLoading}
                    >
                      下一页
                    </GlassButton>
                  </div>
                </div>
              ) : viewSet.remote.local_path ? (
                <div className="empty card muted">
                  {filter.trim() ? "没有匹配的远程规则" : "远程规则集为空"}
                </div>
              ) : null}
            </>
          ) : filtered.length === 0 ? (
            <div className="empty card muted">暂无规则</div>
          ) : (
            <div className="card table-wrap rules-table-wrap">
              <table className="rules-table">
                <colgroup>
                  <col className="col-ord" />
                  <col className="col-type" />
                  <col className="col-payload" />
                  <col className="col-target" />
                  <col className="col-enabled" />
                  <col className="col-actions" />
                </colgroup>
                <thead>
                  <tr>
                    <th>{t("rules.ord")}</th>
                    <th>{t("rules.type")}</th>
                    <th>{t("rules.payload")}</th>
                    <th>{t("rules.target")}</th>
                    <th>{t("rules.enable")}</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((r) => (
                    <tr
                      key={r.id}
                      className={r.enabled ? "rule-row" : "rule-row row-disabled"}
                      onClick={() => openEdit(r)}
                    >
                      <td className="rule-ord">{r.ord}</td>
                      <td className="rule-type">
                        <code>{r.type}</code>
                      </td>
                      <td className="rule-payload" title={r.payload}>
                        {r.payload}
                      </td>
                      <td className="rule-target">
                        {(() => {
                          if (viewSet?.strategy !== "smart") {
                            return (
                              <span className={`pill target-${viewSet?.strategy ?? "proxy"}`}>
                                {viewSet?.strategy ?? "proxy"}
                              </span>
                            );
                          }
                          const lab = targetLabel(r);
                          return (
                            <span
                              className={`pill ${lab.cls}`}
                              title={
                                lab.stale
                                  ? t("rules.nodeStaleHint")
                                  : r.target === "node"
                                    ? r.node_id ?? lab.text
                                    : lab.text
                              }
                            >
                              {lab.text}
                            </span>
                          );
                        })()}
                      </td>
                      <td
                        className="rule-enabled-cell"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <GlassSwitchControl
                          checked={r.enabled}
                          size="sm"
                          title={r.enabled ? "关闭规则" : "启用规则"}
                          onChange={() => void onToggle(r)}
                        />
                      </td>
                      <td
                        className="rule-actions-cell"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <div className="rule-menu" data-rule-menu>
                          <button
                            type="button"
                            className="rule-menu-trigger"
                            aria-label={t("rules.menuAria")}
                            aria-haspopup="menu"
                            aria-expanded={menuRuleId === r.id}
                            onClick={(e) => {
                              e.stopPropagation();
                              setMenuRuleId((id) =>
                                id === r.id ? null : r.id,
                              );
                            }}
                          >
                            ⋮
                          </button>
                          {menuRuleId === r.id && (
                            <div className="rule-menu-pop" role="menu">
                              <button
                                type="button"
                                role="menuitem"
                                className="rule-menu-item"
                                onClick={() => {
                                  setMenuRuleId(null);
                                  openEdit(r);
                                }}
                              >
                                {t("common.edit")}
                              </button>
                              <button
                                type="button"
                                role="menuitem"
                                className="rule-menu-item danger"
                                onClick={() => {
                                  setMenuRuleId(null);
                                  void onDelete(r.id);
                                }}
                              >
                                {t("common.delete")}
                              </button>
                            </div>
                          )}
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      </div>

      {editOpen && (
        <div className="modal-backdrop" onClick={() => !busy && setEditOpen(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header className="modal-header">
              <h2>{editRule ? "编辑规则" : "添加规则"}</h2>
              <button type="button" className="icon-btn" onClick={() => setEditOpen(false)}>
                ×
              </button>
            </header>
            <form className="modal-body" onSubmit={(e) => void onSave(e)}>
              <div className="field">
                <span>类型</span>
                <SolidSelect
                  value={ruleType}
                  options={TYPE_OPTS}
                  onChange={(v) => setRuleType(v as RuleType)}
                  aria-label="类型"
                />
              </div>
              <label className="field">
                <span>匹配内容</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={payload}
                  onChange={(e) => setPayload(e.target.value)}
                  placeholder="google.com / youtube / 10.0.0.0/8"
                  autoFocus
                />
              </label>
              {viewSet?.strategy === "smart" && <div className="field">
                <span>{t("rules.outbound")}</span>
                <SolidSelect
                  value={target}
                  options={targetOpts}
                  onChange={(v) => setTarget(v as RuleTarget)}
                  aria-label={t("rules.outbound")}
                />
              </div>}
              {viewSet?.strategy === "smart" && target === "node" && (
                <div className="field rule-node-pick">
                  <span>{t("rules.pickNode")}</span>
                  {nodes.length === 0 ? (
                    <p className="muted" style={{ margin: 0, fontSize: 12 }}>
                      {t("rules.noNodes")}
                    </p>
                  ) : (
                    <>
                      <input
                        autoCapitalize="off"
                        autoCorrect="off"
                        spellCheck={false}
                        className="search"
                        value={nodeQuery}
                        onChange={(e) => setNodeQuery(e.target.value)}
                        placeholder={t("rules.pickNodePh")}
                      />
                      <SolidSelect
                        list
                        listSize={Math.min(8, Math.max(4, filteredNodes.length || 4))}
                        value={pinNodeId}
                        onChange={setPinNodeId}
                        aria-label={t("rules.pickNode")}
                        options={[
                          { value: "", label: t("rules.needNode") },
                          ...(pinNodeId && !nodeById.has(pinNodeId)
                            ? [
                                {
                                  value: pinNodeId,
                                  label: t("rules.nodeStaleLabel", {
                                    name: editRule?.node_name ?? pinNodeId,
                                  }),
                                },
                              ]
                            : []),
                          ...filteredNodes.map((n) => ({
                            value: n.id,
                            label: n.name,
                          })),
                        ]}
                      />
                      {pinNodeId && !nodeById.has(pinNodeId) && (
                        <p className="banner error" style={{ margin: "8px 0 0" }}>
                          {t("rules.nodeStaleHint")}
                        </p>
                      )}
                    </>
                  )}
                </div>
              )}
              {viewSet?.strategy === "smart" && target === "smart" && (
                <div className="field rule-smart-filters">
                  <p className="muted" style={{ margin: "0 0 8px", fontSize: 12 }}>
                    {t("rules.smartHint")}
                  </p>
                  <label className="field" style={{ marginBottom: 8 }}>
                    <span>{t("rules.smartInclude")}</span>
                    <input
                      autoCapitalize="off"
                      autoCorrect="off"
                      spellCheck={false}
                      value={smartInclude}
                      onChange={(e) => setSmartInclude(e.target.value)}
                      placeholder={t("rules.smartIncludePh")}
                    />
                  </label>
                  <label className="field" style={{ marginBottom: 8 }}>
                    <span>{t("rules.smartExclude")}</span>
                    <input
                      autoCapitalize="off"
                      autoCorrect="off"
                      spellCheck={false}
                      value={smartExclude}
                      onChange={(e) => setSmartExclude(e.target.value)}
                      placeholder={t("rules.smartExcludePh")}
                    />
                  </label>
                  {smartKeywordOverlap.length > 0 ? (
                    <p
                      className="banner error"
                      style={{ margin: "0 0 6px", fontSize: 12 }}
                    >
                      {t("rules.smartKeywordConflict", {
                        k: smartKeywordOverlap.join("、"),
                      })}
                    </p>
                  ) : (
                    <p
                      className="muted"
                      style={{
                        margin: 0,
                        fontSize: 12,
                        color:
                          smartMatchCount === 0
                            ? "var(--danger, #e55)"
                            : undefined,
                      }}
                    >
                      {t("rules.smartMatchCount", { n: smartMatchCount })}
                    </p>
                  )}
                </div>
              )}
              <label className="sys-proxy-row" style={{ border: "none", paddingTop: 0, marginTop: 0 }}>
                <span>{t("rules.enabled")}</span>
                <GlassSwitchControl
                  checked={enabled}
                  title={t("rules.enabled")}
                  onChange={setEnabled}
                />
              </label>
              <footer className="modal-footer">
                <button type="button" className="secondary" onClick={() => setEditOpen(false)}>
                  取消
                </button>
                <button
                  type="submit"
                  disabled={
                    busy ||
                    !payload.trim() ||
                    (viewSet?.strategy === "smart" && target === "node" && !pinNodeId.trim()) ||
                    (viewSet?.strategy === "smart" && target === "smart" &&
                      (nodes.length === 0 || smartKeywordOverlap.length > 0))
                  }
                >
                  {busy ? "保存中…" : "保存"}
                </button>
              </footer>
            </form>
          </div>
        </div>
      )}

      {newSetOpen && (
        <div
          className="modal-backdrop"
          onClick={() => !newSetBusy && setNewSetOpen(false)}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header className="modal-header">
              <h2>新建规则集</h2>
              <button
                type="button"
                className="icon-btn"
                onClick={() => setNewSetOpen(false)}
              >
                ×
              </button>
            </header>
            <form className="modal-body" onSubmit={(e) => void onCreateSet(e)}>
              <label className="field">
                <span>添加方式</span>
                <GlassSeg
                  value={newSetKind}
                  ariaLabel="规则集添加方式"
                  onChange={(value) => setNewSetKind(value as "local" | "remote")}
                  options={[
                    { value: "local", label: "本地规则" },
                    { value: "remote", label: "远程 URL" },
                  ]}
                />
              </label>
              <label className="field">
                <span>名称</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={newSetName}
                  onChange={(e) => setNewSetName(e.target.value)}
                  placeholder="例如：公司内网"
                  autoFocus
                  maxLength={64}
                />
              </label>
              {newSetKind === "remote" && (
                <>
                  <label className="field">
                    <span>远程 URL</span>
                    <input
                      autoCapitalize="off"
                      autoCorrect="off"
                      spellCheck={false}
                      value={newSetUrl}
                      onChange={(e) => setNewSetUrl(e.target.value)}
                      placeholder="https://example.com/rules.json"
                    />
                  </label>
                  <label className="field">
                    <span>整组路由策略</span>
                    <GlassSeg
                      value={newSetTarget}
                      ariaLabel="整组路由策略"
                      onChange={(value) => setNewSetTarget(value as RouteFinal)}
                      options={[
                        { value: "proxy", label: "Proxy" },
                        { value: "direct", label: "Direct" },
                        { value: "block", label: "Block" },
                      ]}
                    />
                  </label>
                </>
              )}
              <p className="muted" style={{ fontSize: 12, margin: 0 }}>
                {newSetKind === "remote"
                  ? "以 sing-box source 格式加载，每 1 小时更新；路由与 DNS 均按整组策略生成。"
                  : "创建后默认为启用；可在左侧开关控制是否参与路由。"}
              </p>
              <footer className="modal-footer">
                <button
                  type="button"
                  className="secondary"
                  disabled={newSetBusy}
                  onClick={() => setNewSetOpen(false)}
                >
                  取消
                </button>
                <button
                  type="submit"
                  disabled={
                    newSetBusy ||
                    !newSetName.trim() ||
                    (newSetKind === "remote" && !newSetUrl.trim())
                  }
                >
                  {newSetBusy ? "创建中…" : "创建"}
                </button>
              </footer>
            </form>
          </div>
        </div>
      )}

      {renameSetTarget && (
        <div
          className="modal-backdrop"
          onClick={() => !renameSetBusy && setRenameSetTarget(null)}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header className="modal-header">
              <h2>重命名规则集</h2>
              <button
                type="button"
                className="icon-btn"
                disabled={renameSetBusy}
                onClick={() => setRenameSetTarget(null)}
              >
                ×
              </button>
            </header>
            <form className="modal-body" onSubmit={(e) => void onRenameSet(e)}>
              <label className="field">
                <span>名称</span>
                <input
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                  value={renameSetName}
                  onChange={(e) => setRenameSetName(e.target.value)}
                  autoFocus
                  maxLength={64}
                />
              </label>
              <footer className="modal-footer">
                <button
                  type="button"
                  className="secondary"
                  disabled={renameSetBusy}
                  onClick={() => setRenameSetTarget(null)}
                >
                  取消
                </button>
                <button
                  type="submit"
                  disabled={renameSetBusy || !renameSetName.trim()}
                >
                  {renameSetBusy ? "保存中…" : "保存"}
                </button>
              </footer>
            </form>
          </div>
        </div>
      )}
    </>
  );

  if (embedded) {
    return <div className="settings-embed rules-embed">{body}</div>;
  }
  return <div className="page">{body}</div>;
}
