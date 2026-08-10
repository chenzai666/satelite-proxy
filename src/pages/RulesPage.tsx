import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  createRuleSet,
  deleteRuleSet,
  getSettings,
  listAllNodes,
  listRuleSets,
  listRules,
  removeRule,
  reorderRuleSets,
  resetRuleSet,
  saveRule,
  setRuleEnabled,
  setRuleSetEnabled,
  updateSettings,
} from "../api";
import { GlassButton } from "../components/GlassButton";
import { SolidSelect } from "../components/SolidSelect";
import { GlassSeg } from "../components/GlassSeg";
import { useI18n } from "../i18n";
import type {
  ProxyNode,
  Rule,
  RuleSetSummary,
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
  const [newSetBusy, setNewSetBusy] = useState(false);
  /** Row ⋮ menu open for this rule id */
  const [menuRuleId, setMenuRuleId] = useState<string | null>(null);

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
    setViewSetId((prev) => prev ?? preferred);
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
    const list = await listRules(setId);
    setRules(list);
  }, []);

  const reload = useCallback(async () => {
    setError(null);
    try {
      await reloadRouteFinal();
      const { preferred } = await reloadSets();
      const sid = viewSetId ?? preferred;
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
    if (!menuRuleId) return;
    function onDocPointerDown(e: PointerEvent) {
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("[data-rule-menu]")) return;
      setMenuRuleId(null);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setMenuRuleId(null);
    }
    document.addEventListener("pointerdown", onDocPointerDown, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDocPointerDown, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuRuleId]);

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
    setTarget("proxy");
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
    if (target === "node" && !pinNodeId.trim()) {
      setError(t("rules.needNode"));
      return;
    }
    if (target === "smart" && smartKeywordOverlap.length > 0) {
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
        target,
        ord: editRule?.ord ?? null,
        enabled,
        nodeId: target === "node" ? pinNodeId : null,
        smartInclude: target === "smart" ? parseKeywords(smartInclude) : null,
        smartExclude: target === "smart" ? parseKeywords(smartExclude) : null,
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

  async function onToggleSet(id: string, enabled: boolean) {
    try {
      await setRuleSetEnabled(id, enabled);
      await reloadSets();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    }
  }

  function openNewSet() {
    setNewSetName("自定义规则集");
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
      const set = await createRuleSet(name);
      const list = await listRuleSets();
      setSets(list);
      setViewSetId(set.id);
      setRules([]);
      setNewSetOpen(false);
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setNewSetBusy(false);
    }
  }

  async function onDeleteSet() {
    if (!viewSetId || viewSet?.builtin) return;
    if (!confirm(`删除规则集「${viewSet?.name}」？`)) return;
    try {
      await deleteRuleSet(viewSetId);
      setViewSetId(null);
      await reload();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    }
  }

  function isFactorySet(s: RuleSetSummary | undefined | null) {
    if (!s) return false;
    return s.builtin || s.id === "general-rules" || s.id.startsWith("builtin-");
  }

  async function onResetFactory() {
    if (!viewSetId || !viewSet || !isFactorySet(viewSet)) return;
    const name = viewSet.name;
    if (
      !confirm(
        `将「${name}」恢复为出厂配置文件？\n当前对该集的编辑会丢失。\n（重启程序不会自动重置，只会保留你的修改。）`,
      )
    ) {
      return;
    }
    try {
      await resetRuleSet(viewSetId);
      await reloadRules(viewSetId);
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
        <aside className="card ruleset-list">
          <GlassButton
            icon="+"
            onClick={openNewSet}
            title={t("rules.newSetTitle")}
          >
            {t("rules.newSet")}
          </GlassButton>
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
                if (e.key === "Enter" || e.key === " ") setViewSetId(s.id);
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
                <button
                  type="button"
                  role="switch"
                  aria-checked={s.enabled}
                  className={`switch small ${s.enabled ? "on" : ""}`}
                  title={s.enabled ? "关闭规则集" : "启用规则集"}
                  onClick={(e) => {
                    e.stopPropagation();
                    void onToggleSet(s.id, !s.enabled);
                  }}
                >
                  <span className="switch-thumb" />
                </button>
              </div>
              <div className="muted" style={{ fontSize: 12 }}>
                {t("rules.rulesCount", { n: s.rule_count })}
                {s.builtin ? ` · ${t("rules.builtin")}` : ""}
                {s.enabled
                  ? ` · ${t("rules.setOn")}`
                  : ` · ${t("rules.setOff")}`}
              </div>
            </div>
          ))}
        </aside>

        <section className="rules-main">
          <div className="rules-toolbar card">
            <div>
              <strong>{viewSet?.name ?? "—"}</strong>
              <div className="muted" style={{ fontSize: 12, marginTop: 2 }}>
                {viewSet?.enabled
                  ? "已启用：合并进路由（保存规则时自动重启内核）"
                  : "未启用：打开左侧开关即可参与路由（可多选）"}
              </div>
            </div>
            <div className="header-actions">
              <GlassButton
                variant="primary"
                icon="+"
                onClick={openCreate}
                disabled={!viewSetId}
                title={t("rules.addRuleTitle")}
              >
                {t("rules.addRule")}
              </GlassButton>
              {isFactorySet(viewSet) && (
                <GlassButton
                  icon="↺"
                  onClick={() => void onResetFactory()}
                  title={t("rules.resetFactoryHint")}
                >
                  {t("rules.resetFactory")}
                </GlassButton>
              )}
              {viewSet &&
                !viewSet.builtin &&
                viewSet.id !== "general-rules" &&
                !viewSet.id.startsWith("builtin-") && (
                <GlassButton
                  variant="danger"
                  icon="⌫"
                  onClick={() => void onDeleteSet()}
                  title={t("rules.deleteSet")}
                >
                  {t("rules.deleteSet")}
                </GlassButton>
              )}
              <input
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
                className="search"
                placeholder="过滤规则…"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
              />
            </div>
          </div>

          {loading ? (
            <div className="empty">加载中…</div>
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
                        <button
                          type="button"
                          role="switch"
                          aria-checked={r.enabled}
                          className={`switch small ${r.enabled ? "on" : ""}`}
                          onClick={() => void onToggle(r)}
                        >
                          <span className="switch-thumb" />
                        </button>
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
              <div className="field">
                <span>{t("rules.outbound")}</span>
                <SolidSelect
                  value={target}
                  options={targetOpts}
                  onChange={(v) => setTarget(v as RuleTarget)}
                  aria-label={t("rules.outbound")}
                />
              </div>
              {target === "node" && (
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
              {target === "smart" && (
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
                <button
                  type="button"
                  role="switch"
                  className={`switch ${enabled ? "on" : ""}`}
                  onClick={() => setEnabled((v) => !v)}
                >
                  <span className="switch-thumb" />
                </button>
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
                    (target === "node" && !pinNodeId.trim()) ||
                    (target === "smart" &&
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
              <p className="muted" style={{ fontSize: 12, margin: 0 }}>
                创建后默认为启用；可在左侧开关控制是否参与路由。
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
                <button type="submit" disabled={newSetBusy || !newSetName.trim()}>
                  {newSetBusy ? "创建中…" : "创建"}
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
