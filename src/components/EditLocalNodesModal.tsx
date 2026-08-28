import { useEffect, useMemo, useState, type FormEvent } from "react";
import {
  deleteNode,
  getNodeDraft,
  listSubscriptionNodes,
  updateNode,
} from "../api";
import { useI18n } from "../i18n";
import type { ManualNodeDraft, ProxyNode } from "../types";
import { ErrorModal } from "./ErrorModal";
import { GlassButton } from "./GlassButton";
import { NodeDraftFields, nodeDraftReady } from "./NodeDraftFields";

interface Props {
  open: boolean;
  profileId: string | null;
  profileName: string;
  /** Select this node first when opened from its context menu. */
  initialNodeId?: string | null;
  onClose: () => void;
  onNodesChanged?: () => void;
}

function errorText(error: unknown) {
  return typeof error === "string" ? error : String(error);
}

export function EditLocalNodesModal({
  open,
  profileId,
  profileName,
  initialNodeId,
  onClose,
  onNodesChanged,
}: Props) {
  const { t } = useI18n();
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<ManualNodeDraft | null>(null);
  const [baseline, setBaseline] = useState("");
  const [loading, setLoading] = useState(false);
  const [loadingDraft, setLoadingDraft] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<"saved" | "deleted" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const serializedDraft = useMemo(() => JSON.stringify(draft), [draft]);
  const dirty = !!draft && baseline !== "" && serializedDraft !== baseline;

  useEffect(() => {
    if (!open || !profileId) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    setStatus(null);
    setDraft(null);
    setBaseline("");
    void listSubscriptionNodes(profileId)
      .then(async (list) => {
        if (cancelled) return;
        setNodes(list);
        const initial = list.find((node) => node.id === initialNodeId) ?? list[0];
        setSelectedId(initial?.id ?? null);
        if (!initial) return;
        const next = await getNodeDraft(initial.id);
        if (cancelled) return;
        setDraft(next);
        setBaseline(JSON.stringify(next));
      })
      .catch((e) => {
        if (!cancelled) setError(errorText(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, profileId, initialNodeId]);

  if (!open) return null;

  async function selectNode(id: string) {
    if (id === selectedId || busy || loadingDraft) return;
    if (dirty && !window.confirm(t("nodes.unsavedConfirm"))) return;
    setSelectedId(id);
    setLoadingDraft(true);
    setStatus(null);
    setError(null);
    try {
      const next = await getNodeDraft(id);
      setDraft(next);
      setBaseline(JSON.stringify(next));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoadingDraft(false);
    }
  }

  function closeModal() {
    if (dirty && !window.confirm(t("nodes.unsavedConfirm"))) return;
    onClose();
  }

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!selectedId || !draft || busy) return;
    const name = (draft.name ?? "").trim();
    if (!name || !nodeDraftReady(draft)) {
      setError(t("nodes.invalidDraft"));
      return;
    }
    const nextDraft = { ...draft, name };
    setBusy(true);
    setStatus(null);
    setError(null);
    try {
      const updated = await updateNode(selectedId, nextDraft);
      setNodes((current) =>
        current.map((node) => (node.id === selectedId ? updated : node)),
      );
      setDraft(nextDraft);
      setBaseline(JSON.stringify(nextDraft));
      setStatus("saved");
      onNodesChanged?.();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete() {
    if (!selectedId || busy) return;
    const index = nodes.findIndex((node) => node.id === selectedId);
    const selected = nodes[index];
    if (!selected || !window.confirm(t("nodes.deleteConfirm", { name: selected.name }))) {
      return;
    }
    setBusy(true);
    setStatus(null);
    setError(null);
    try {
      await deleteNode(selectedId);
      const remaining = nodes.filter((node) => node.id !== selectedId);
      const next = remaining[Math.min(index, remaining.length - 1)];
      setNodes(remaining);
      setSelectedId(next?.id ?? null);
      setDraft(null);
      setBaseline("");
      setStatus("deleted");
      onNodesChanged?.();
      if (next) {
        setLoadingDraft(true);
        try {
          const nextDraft = await getNodeDraft(next.id);
          setDraft(nextDraft);
          setBaseline(JSON.stringify(nextDraft));
        } finally {
          setLoadingDraft(false);
        }
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop">
      <div
        className="modal node-editor-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="edit-local-nodes-title"
      >
        <header className="modal-header">
          <div>
            <h2 id="edit-local-nodes-title">{t("nodes.renameTitle")}</h2>
            <p className="hint node-editor-profile">{profileName}</p>
          </div>
          <button
            type="button"
            className="icon-btn"
            onClick={closeModal}
            disabled={busy}
            aria-label={t("common.close")}
          >
            ×
          </button>
        </header>
        <form
          className="modal-body node-editor-body"
          onSubmit={(event) => void handleSubmit(event)}
        >
          <p className="hint node-editor-hint">{t("nodes.localOverrideHint")}</p>
          {loading ? (
            <div className="muted">{t("common.loading")}</div>
          ) : nodes.length === 0 ? (
            <div className="muted">{t("nodes.empty")}</div>
          ) : (
            <div className="node-editor-layout">
              <div className="node-editor-list" role="listbox">
                {nodes.map((node) => (
                  <button
                    key={node.id}
                    type="button"
                    role="option"
                    aria-selected={node.id === selectedId}
                    className={`node-editor-item${node.id === selectedId ? " active" : ""}`}
                    onClick={() => void selectNode(node.id)}
                    disabled={busy || loadingDraft}
                  >
                    <strong>{node.name}</strong>
                    <span>
                      {node.protocol} · {node.server}:{node.port}
                    </span>
                  </button>
                ))}
              </div>
              <div className="node-editor-form">
                {loadingDraft || !draft ? (
                  <div className="muted">{t("common.loading")}</div>
                ) : (
                  <>
                    <label className="field">
                      <span>{t("nodes.renamePlaceholder")}</span>
                      <input
                        autoCapitalize="off"
                        autoCorrect="off"
                        spellCheck={false}
                        value={draft.name ?? ""}
                        onChange={(event) => {
                          setStatus(null);
                          setDraft({ ...draft, name: event.target.value });
                        }}
                        disabled={busy}
                      />
                    </label>
                    <NodeDraftFields
                      value={draft}
                      disabled={busy}
                      onChange={(next) => {
                        setStatus(null);
                        setDraft(next);
                      }}
                    />
                  </>
                )}
              </div>
            </div>
          )}
          {status && (
            <div className="node-editor-saved">
              {t(status === "saved" ? "nodes.saveSuccess" : "nodes.deleteSuccess")}
            </div>
          )}
          {error && <ErrorModal message={error} onClose={() => setError(null)} />}
          <footer className="modal-footer node-editor-footer">
            <GlassButton
              variant="danger"
              className="node-editor-delete"
              onClick={() => void handleDelete()}
              disabled={busy || loading || loadingDraft || !selectedId}
            >
              {t("common.delete")}
            </GlassButton>
            <GlassButton onClick={closeModal} disabled={busy}>
              {t("common.close")}
            </GlassButton>
            <GlassButton
              type="submit"
              variant="primary"
              disabled={busy || loading || loadingDraft || !draft || !dirty}
            >
              {busy ? t("common.saving") : t("nodes.renameSave")}
            </GlassButton>
          </footer>
        </form>
      </div>
    </div>
  );
}
