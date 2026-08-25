import { useEffect, useState, type FormEvent } from "react";
import { listSubscriptionNodes, renameNode } from "../api";
import { GlassButton } from "./GlassButton";
import { ErrorModal } from "./ErrorModal";
import { useI18n } from "../i18n";
import type { ProxyNode } from "../types";

interface Props {
  open: boolean;
  profileId: string | null;
  profileName: string;
  onClose: () => void;
}

export function EditLocalNodesModal({
  open,
  profileId,
  profileName,
  onClose,
}: Props) {
  const { t } = useI18n();
  const [nodes, setNodes] = useState<ProxyNode[]>([]);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || !profileId) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    void listSubscriptionNodes(profileId)
      .then((list) => {
        if (cancelled) return;
        setNodes(list);
        const next: Record<string, string> = {};
        for (const n of list) next[n.id] = n.name;
        setDrafts(next);
      })
      .catch((e) => {
        if (!cancelled) setError(typeof e === "string" ? e : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, profileId]);

  if (!open) return null;

  const changed = nodes.filter((n) => (drafts[n.id] ?? "").trim() !== n.name);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (busy || changed.length === 0) {
      onClose();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      for (const n of changed) {
        const name = (drafts[n.id] ?? "").trim();
        if (!name) {
          setError(t("nodes.renamePlaceholder"));
          setBusy(false);
          return;
        }
        await renameNode(n.id, name);
      }
      onClose();
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop">
      <div
        className="modal config-add-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="edit-local-nodes-title"
      >
        <header className="modal-header">
          <h2 id="edit-local-nodes-title">{t("nodes.renameTitle")}</h2>
          <button
            type="button"
            className="icon-btn"
            onClick={onClose}
            disabled={busy}
            aria-label={t("common.close")}
          >
            ×
          </button>
        </header>
        <form className="modal-body" onSubmit={(e) => void handleSubmit(e)}>
          <p className="hint" style={{ margin: 0 }}>
            {profileName}
          </p>
          {loading ? (
            <div className="muted">{t("common.loading")}</div>
          ) : nodes.length === 0 ? (
            <div className="muted">{t("nodes.empty")}</div>
          ) : (
            <div className="rename-node-list">
              {nodes.map((n) => (
                <label key={n.id} className="field">
                  <span className="rename-node-meta">
                    <code>{n.protocol}</code> {n.server}:{n.port}
                  </span>
                  <input
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                    value={drafts[n.id] ?? ""}
                    onChange={(e) =>
                      setDrafts((prev) => ({ ...prev, [n.id]: e.target.value }))
                    }
                    placeholder={t("nodes.renamePlaceholder")}
                    disabled={busy}
                  />
                </label>
              ))}
            </div>
          )}
          {error && (
            <ErrorModal message={error} onClose={() => setError(null)} />
          )}
          <footer className="modal-footer">
            <GlassButton onClick={onClose} disabled={busy}>
              {t("common.cancel")}
            </GlassButton>
            <GlassButton
              type="submit"
              variant="primary"
              disabled={busy || loading || nodes.length === 0}
            >
              {busy ? t("common.saving") : t("nodes.renameSave")}
            </GlassButton>
          </footer>
        </form>
      </div>
    </div>
  );
}
