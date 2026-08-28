import QRCode from "qrcode";
import { useEffect, useRef, useState } from "react";
import { getNodeShareUri } from "../api";
import { useI18n } from "../i18n";
import { copyNodeShareText } from "../nodeShare";
import type { ProxyNode } from "../types";
import { GlassButton } from "./GlassButton";

interface Props {
  node: ProxyNode | null;
  onClose: () => void;
}

function errorText(error: unknown) {
  return typeof error === "string" ? error : String(error);
}

/** Renders a node URI as a local QR canvas; credentials never leave the app. */
export function NodeShareModal({ node, onClose }: Props) {
  const { t } = useI18n();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [uri, setUri] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!node) return;
    let cancelled = false;
    setUri(null);
    setError(null);
    setCopied(false);
    void getNodeShareUri(node.id)
      .then((value) => {
        if (!cancelled) setUri(value);
      })
      .catch((reason) => {
        if (!cancelled) setError(errorText(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [node]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!uri || !canvas) return;
    let cancelled = false;
    void QRCode.toCanvas(canvas, uri, {
      width: 224,
      margin: 1,
      errorCorrectionLevel: "M",
      color: { dark: "#101318", light: "#ffffff" },
    }).catch((reason) => {
      if (!cancelled) setError(errorText(reason));
    });
    return () => {
      cancelled = true;
    };
  }, [uri]);

  if (!node) return null;

  async function copy() {
    if (!uri) return;
    try {
      await copyNodeShareText(uri);
      setCopied(true);
    } catch (reason) {
      setError(errorText(reason));
    }
  }

  return (
    <div className="modal-backdrop">
      <div className="modal node-share-modal" role="dialog" aria-modal="true" aria-labelledby="node-share-title">
        <header className="modal-header">
          <div>
            <h2 id="node-share-title">{t("nodes.shareTitle")}</h2>
            <p className="hint" title={node.name}>{node.name}</p>
          </div>
          <button type="button" className="icon-btn" onClick={onClose} aria-label={t("common.close")}>×</button>
        </header>
        <div className="modal-body node-share-body">
          <p className="node-share-warning">{t("nodes.shareHint")}</p>
          {error ? (
            <div className="node-share-error">{error || t("nodes.shareUnsupported")}</div>
          ) : !uri ? (
            <div className="muted">{t("nodes.shareLoading")}</div>
          ) : (
            <>
              <div className="node-share-qr"><canvas ref={canvasRef} /></div>
              <textarea className="node-share-uri" readOnly value={uri} aria-label={t("nodes.contextCopyLink")} />
            </>
          )}
        </div>
        <footer className="modal-footer">
          <GlassButton onClick={onClose}>{t("common.close")}</GlassButton>
          <GlassButton variant="primary" disabled={!uri} onClick={() => void copy()}>
            {copied ? t("nodes.shareCopied") : t("nodes.shareCopy")}
          </GlassButton>
        </footer>
      </div>
    </div>
  );
}
