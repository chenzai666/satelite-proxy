import { useEffect, useMemo, useState } from "react";
import {
  createChain,
  createPool,
  deleteChain,
  deletePool,
  diagnoseChain,
  listAllNodes,
  listChainUsage,
  listChains,
  listPools,
  updateChain,
  updatePool,
} from "../api";
import { GlassButton } from "../components/GlassButton";
import { SolidSelect } from "../components/SolidSelect";
import { useI18n } from "../i18n";
import type {
  ChainDiagnosis,
  ChainHop,
  NodePool,
  PoolMode,
  ProxyChain,
  ProxyNode,
} from "../types";

function words(value: string): string[] {
  return value
    .split(/\s+/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function hopText(hop: ChainHop, nodes: ProxyNode[], pools: NodePool[]): string {
  if (hop.kind === "node") {
    return nodes.find((node) => node.id === hop.node_id)?.name ?? hop.node_id;
  }
  return `◫ ${pools.find((pool) => pool.id === hop.pool_id)?.name ?? hop.pool_id}`;
}

interface Props {
  embedded?: boolean;
}

/**
 * A compact, keyboard-friendly editor for the upstream node-pool and
 * multi-hop-chain backend. The chain order shown here is the wire order:
 * left/top is the client-side entry hop; the last hop is the internet exit.
 */
export function ChainPage({ embedded = false }: Props) {
  const { t } = useI18n();
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
  const [pools, setPools] = useState<NodePool[]>([]);
  const [chains, setChains] = useState<ProxyChain[]>([]);
  const [usage, setUsage] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [poolEdit, setPoolEdit] = useState<NodePool | null>(null);
  const [poolOpen, setPoolOpen] = useState(false);
  const [poolName, setPoolName] = useState("");
  const [poolMode, setPoolMode] = useState<"explicit" | "keyword">("explicit");
  const [poolNodes, setPoolNodes] = useState<Set<string>>(new Set());
  const [poolInclude, setPoolInclude] = useState("");
  const [poolExclude, setPoolExclude] = useState("");

  const [chainEdit, setChainEdit] = useState<ProxyChain | null>(null);
  const [chainOpen, setChainOpen] = useState(false);
  const [chainName, setChainName] = useState("");
  const [chainHops, setChainHops] = useState<ChainHop[]>([]);
  const [hopCandidate, setHopCandidate] = useState("");
  const [diagnosis, setDiagnosis] = useState<{ id: string; data: ChainDiagnosis } | null>(null);
  const [diagnosingId, setDiagnosingId] = useState<string | null>(null);

  async function reload() {
    const [nodeList, poolList, chainList, usageList] = await Promise.all([
      listAllNodes(),
      listPools(),
      listChains(),
      listChainUsage(),
    ]);
    setNodes(nodeList);
    setPools(poolList);
    setChains(chainList);
    setUsage(usageList);
  }

  useEffect(() => {
    void reload().catch((reason) => setError(String(reason)));
  }, []);

  const candidateOptions = useMemo(
    () => [
      { value: "", label: t("chain.needHop") },
      ...nodes.map((node) => ({ value: `node:${node.id}`, label: node.name })),
      ...pools.map((pool) => ({ value: `pool:${pool.id}`, label: `◫ ${pool.name}` })),
    ],
    [nodes, pools, t],
  );

  function beginPool(pool?: NodePool) {
    setPoolEdit(pool ?? null);
    setPoolName(pool?.name ?? "");
    const mode = pool?.mode;
    if (mode?.mode === "keyword") {
      setPoolMode("keyword");
      setPoolNodes(new Set());
      setPoolInclude(mode.include.join(" "));
      setPoolExclude(mode.exclude.join(" "));
    } else {
      setPoolMode("explicit");
      setPoolNodes(new Set(mode?.mode === "explicit" ? mode.node_ids : []));
      setPoolInclude("");
      setPoolExclude("");
    }
    setError(null);
    setPoolOpen(true);
  }

  function beginChain(chain?: ProxyChain) {
    setChainEdit(chain ?? null);
    setChainName(chain?.name ?? "");
    setChainHops(chain?.hops ?? []);
    setHopCandidate("");
    setError(null);
    setChainOpen(true);
  }

  async function savePool() {
    const name = poolName.trim();
    if (!name) {
      setError(t("chain.needName"));
      return;
    }
    const mode: PoolMode =
      poolMode === "explicit"
        ? { mode: "explicit", node_ids: [...poolNodes] }
        : { mode: "keyword", include: words(poolInclude), exclude: words(poolExclude) };
    setBusy(true);
    setError(null);
    try {
      if (poolEdit) await updatePool(poolEdit.id, name, mode);
      else await createPool(name, mode);
      setPoolEdit(null);
      setPoolOpen(false);
      await reload();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function removePool(pool: NodePool) {
    if (!confirm(`${t("common.delete")}「${pool.name}」？`)) return;
    setBusy(true);
    setError(null);
    try {
      await deletePool(pool.id);
      if (poolEdit?.id === pool.id) setPoolEdit(null);
      await reload();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function addHop() {
    const [kind, id] = hopCandidate.split(":", 2);
    if (!id || (kind !== "node" && kind !== "pool")) {
      setError(t("chain.needHop"));
      return;
    }
    setChainHops((current) => [
      ...current,
      kind === "node" ? { kind, node_id: id } : { kind, pool_id: id },
    ]);
    setHopCandidate("");
  }

  function moveHop(index: number, direction: -1 | 1) {
    setChainHops((current) => {
      const next = index + direction;
      if (next < 0 || next >= current.length) return current;
      const copy = [...current];
      [copy[index], copy[next]] = [copy[next], copy[index]];
      return copy;
    });
  }

  async function saveChain() {
    const name = chainName.trim();
    if (!name) {
      setError(t("chain.needName"));
      return;
    }
    if (chainHops.length < 2) {
      setError(t("chain.needTwoHops"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (chainEdit) await updateChain(chainEdit.id, name, chainHops);
      else await createChain(name, chainHops);
      setChainEdit(null);
      setChainOpen(false);
      await reload();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function removeChain(chain: ProxyChain) {
    if (!confirm(`${t("common.delete")}「${chain.name}」？`)) return;
    setBusy(true);
    setError(null);
    try {
      await deleteChain(chain.id);
      if (chainEdit?.id === chain.id) setChainEdit(null);
      if (diagnosis?.id === chain.id) setDiagnosis(null);
      await reload();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function runDiagnosis(chain: ProxyChain) {
    setDiagnosingId(chain.id);
    setError(null);
    try {
      setDiagnosis({ id: chain.id, data: await diagnoseChain(chain.id) });
    } catch (reason) {
      setError(String(reason));
    } finally {
      setDiagnosingId(null);
    }
  }

  return (
    <div className={embedded ? "settings-embed" : "page"}>
      {!embedded && (
        <header className="page-header">
          <div>
            <h1>{t("chain.title")}</h1>
            <p className="page-desc">{t("chain.desc")}</p>
          </div>
        </header>
      )}
      {error && <p className="banner error">{error}</p>}
      <p className="muted" style={{ fontSize: 12 }}>{t("chain.saveHint")}</p>

      <section className="card" style={{ marginTop: 12, padding: 16 }}>
        <div className="header-actions" style={{ justifyContent: "space-between" }}>
          <h2 style={{ margin: 0 }}>{t("chain.pools")}</h2>
          <GlassButton variant="primary" onClick={() => beginPool()}>{t("chain.newPool")}</GlassButton>
        </div>
        {pools.length === 0 ? <p className="muted">{t("chain.noPools")}</p> : (
          <div className="settings-list" style={{ marginTop: 12 }}>
            {pools.map((pool) => (
              <div key={pool.id} className="settings-row">
                <div>
                  <strong>◫ {pool.name}</strong>
                  <p className="muted" style={{ margin: "4px 0 0", fontSize: 12 }}>
                    {pool.mode.mode === "explicit"
                      ? `${t("chain.modeExplicit")} · ${pool.mode.node_ids.length}`
                      : `${t("chain.modeKeyword")} · ${[...pool.mode.include, ...pool.mode.exclude.map((item) => `-${item}`)].join(" ") || "*"}`}
                  </p>
                </div>
                <div className="header-actions">
                  <GlassButton onClick={() => beginPool(pool)}>{t("common.edit")}</GlassButton>
                  <GlassButton variant="danger" onClick={() => void removePool(pool)}>{t("common.delete")}</GlassButton>
                </div>
              </div>
            ))}
          </div>
        )}
        {poolOpen && (
          <div className="settings-panel" style={{ marginTop: 16 }}>
            <label className="field">
              <span>{t("chain.name")}</span>
              <input value={poolName} onChange={(event) => setPoolName(event.target.value)} maxLength={64} />
            </label>
            <label className="field">
              <span>{t("chain.mode")}</span>
              <SolidSelect
                value={poolMode}
                onChange={(value) => setPoolMode(value as "explicit" | "keyword")}
                options={[
                  { value: "explicit", label: t("chain.modeExplicit") },
                  { value: "keyword", label: t("chain.modeKeyword") },
                ]}
              />
            </label>
            {poolMode === "explicit" ? (
              <div className="field">
                <span>{t("rules.pickNode")}</span>
                <div className="settings-list">
                  {nodes.map((node) => (
                    <label key={node.id} className="settings-row" style={{ cursor: "pointer" }}>
                      <span>{node.name}</span>
                      <input
                        type="checkbox"
                        checked={poolNodes.has(node.id)}
                        onChange={() => setPoolNodes((current) => {
                          const next = new Set(current);
                          next.has(node.id) ? next.delete(node.id) : next.add(node.id);
                          return next;
                        })}
                      />
                    </label>
                  ))}
                </div>
              </div>
            ) : (
              <>
                <label className="field">
                  <span>{t("chain.include")}</span>
                  <input value={poolInclude} onChange={(event) => setPoolInclude(event.target.value)} />
                </label>
                <label className="field">
                  <span>{t("chain.exclude")}</span>
                  <input value={poolExclude} onChange={(event) => setPoolExclude(event.target.value)} />
                </label>
              </>
            )}
            <div className="modal-footer">
              <GlassButton onClick={() => { setPoolEdit(null); setPoolName(""); setPoolOpen(false); }}>{t("common.cancel")}</GlassButton>
              <GlassButton variant="primary" disabled={busy} onClick={() => void savePool()}>{t("common.save")}</GlassButton>
            </div>
          </div>
        )}
      </section>

      <section className="card" style={{ marginTop: 16, padding: 16 }}>
        <div className="header-actions" style={{ justifyContent: "space-between" }}>
          <h2 style={{ margin: 0 }}>{t("chain.chains")}</h2>
          <GlassButton variant="primary" onClick={() => beginChain()}>{t("chain.newChain")}</GlassButton>
        </div>
        {chains.length === 0 ? <p className="muted">{t("chain.noChains")}</p> : (
          <div className="settings-list" style={{ marginTop: 12 }}>
            {chains.map((chain) => (
              <div key={chain.id} className="settings-row">
                <div>
                  <strong>{chain.name}</strong>
                  <p className="muted" style={{ margin: "4px 0 0", fontSize: 12 }}>
                    {chain.hops.map((hop) => hopText(hop, nodes, pools)).join(" → ")}
                    {usage[chain.id]?.length ? ` · ${usage[chain.id].join("、")}` : ""}
                  </p>
                </div>
                <div className="header-actions">
                  <GlassButton disabled={diagnosingId === chain.id} onClick={() => void runDiagnosis(chain)}>
                    {diagnosingId === chain.id ? t("chain.diagnosing") : t("chain.diagnose")}
                  </GlassButton>
                  <GlassButton onClick={() => beginChain(chain)}>{t("common.edit")}</GlassButton>
                  <GlassButton variant="danger" onClick={() => void removeChain(chain)}>{t("common.delete")}</GlassButton>
                </div>
              </div>
            ))}
          </div>
        )}
        {chainOpen && (
          <div className="settings-panel" style={{ marginTop: 16 }}>
            <label className="field">
              <span>{t("chain.name")}</span>
              <input value={chainName} onChange={(event) => setChainName(event.target.value)} maxLength={64} />
            </label>
            <div className="field">
              <span>{t("chain.addHop")}</span>
              <div className="header-actions">
                <SolidSelect value={hopCandidate} onChange={setHopCandidate} options={candidateOptions} />
                <GlassButton onClick={addHop}>{t("common.add")}</GlassButton>
              </div>
            </div>
            <div className="field">
              <span>{t("chain.hops")}</span>
              <div className="settings-list">
                {chainHops.map((hop, index) => (
                  <div key={`${hop.kind}-${index}-${hopText(hop, nodes, pools)}`} className="settings-row">
                    <span>{index + 1}. {hopText(hop, nodes, pools)}</span>
                    <div className="header-actions">
                      <GlassButton disabled={index === 0} onClick={() => moveHop(index, -1)}>↑</GlassButton>
                      <GlassButton disabled={index === chainHops.length - 1} onClick={() => moveHop(index, 1)}>↓</GlassButton>
                      <GlassButton variant="danger" onClick={() => setChainHops((current) => current.filter((_, item) => item !== index))}>×</GlassButton>
                    </div>
                  </div>
                ))}
              </div>
            </div>
            <div className="modal-footer">
              <GlassButton onClick={() => { setChainEdit(null); setChainName(""); setChainHops([]); setChainOpen(false); }}>{t("common.cancel")}</GlassButton>
              <GlassButton variant="primary" disabled={busy} onClick={() => void saveChain()}>{t("common.save")}</GlassButton>
            </div>
          </div>
        )}
        {diagnosis && (
          <div className="settings-panel" style={{ marginTop: 16 }}>
            <strong>{t("chain.diagnose")}</strong>
            {diagnosis.data.hops.map((hop, index) => (
              <p key={`${hop.label}-${index}`} className="muted" style={{ margin: "8px 0 0" }}>
                {index + 1}. {hop.label} · {hop.soloMs ?? "—"} ms → {hop.chainedMs ?? "—"} ms
                {hop.soloError ? ` · ${hop.soloError}` : ""}
                {hop.chainedError ? ` · ${hop.chainedError}` : ""}
              </p>
            ))}
            <p className="muted" style={{ margin: "10px 0 0" }}>
              {t("chain.exit")}: {diagnosis.data.exit.geo?.ip ?? diagnosis.data.exit.ipError ?? "—"}
              {diagnosis.data.exit.ipSbMs != null ? ` · ${diagnosis.data.exit.ipSbMs} ms` : ""}
            </p>
          </div>
        )}
      </section>
    </div>
  );
}
