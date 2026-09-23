import { Component, type ErrorInfo, type ReactNode } from "react";
import { logFrontendEvent } from "../api";
import { GlassButton } from "./GlassButton";
import { useI18n } from "../i18n";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/**
 * Top-of-app crash net. A render exception anywhere below would otherwise
 * unmount the whole React tree and leave an opaque white window — exactly
 * the "整个 UI 白屏只能强退" failure mode. This catches it, reports the
 * message + component stack to the app log (`webview` target, Logs →
 * 应用日志), and offers an in-place remount instead. The fallback stays
 * inline-styled and reads i18n through a nested function component: the
 * provider sits above this boundary, so a crash of i18n state itself is
 * not something the fallback could survive anyway.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    const stack = (info.componentStack ?? "")
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .slice(0, 8)
      .join(" | ");
    logFrontendEvent(`render crash: ${error.message}\ncomponent stack: ${stack}`);
  }

  private retry = () => {
    this.setState({ error: null });
  };

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <CrashFallback error={error} onRetry={this.retry} />
    );
  }
}

function CrashFallback({
  error,
  onRetry,
}: {
  error: Error;
  onRetry: () => void;
}) {
  const { t } = useI18n();
  return (
    <div
      style={{
        minHeight: "100vh",
        display: "grid",
        placeItems: "center",
        padding: "1.5rem",
      }}
    >
      <div
        className="card"
        style={{
          padding: "1.4rem 1.3rem",
          display: "grid",
          gap: "0.9rem",
          maxWidth: "34rem",
          width: "100%",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "1rem" }}>{t("crash.title")}</h2>
        <pre
          className="mono"
          style={{
            margin: 0,
            whiteSpace: "pre-wrap",
            wordBreak: "break-all",
            maxHeight: "38vh",
            overflow: "auto",
            fontSize: "0.72rem",
            color: "var(--muted)",
          }}
        >
          {error.message || String(error)}
        </pre>
        <div style={{ display: "flex", gap: "0.5rem" }}>
          <GlassButton icon="↻" onClick={onRetry}>
            {t("crash.retry")}
          </GlassButton>
          <GlassButton
            variant="primary"
            icon="⟳"
            onClick={() => window.location.reload()}
          >
            {t("crash.reload")}
          </GlassButton>
        </div>
        <span style={{ fontSize: "0.68rem", color: "var(--muted)" }}>
          {t("crash.logged")}
        </span>
      </div>
    </div>
  );
}
