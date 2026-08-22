import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type FormEvent,
} from "react";
import {
  applyAccentToDom,
  hexToRgb,
  hsvToRgb,
  rgbToHex,
  rgbToHsv,
} from "../theme/accents";
import { useTheme } from "../theme/ThemeContext";
import { GlassButton } from "./GlassButton";

interface Props {
  /** Starting color (any `#rrggbb`). */
  current: string;
  /** Persist `#rrggbb` as the custom accent. */
  onApply: (hex: string) => void;
  onClose: () => void;
  title: string;
  applyLabel: string;
  cancelLabel: string;
}

/**
 * Free-form accent picker: saturation/value square + hue strip + hex field.
 * Dragging live-previews the accent on the whole UI (applyAccentToDom);
 * closing without applying restores the accent that was active on mount.
 */
export function AccentColorPickerModal({
  current,
  onApply,
  onClose,
  title,
  applyLabel,
  cancelLabel,
}: Props) {
  const { theme, accent } = useTheme();
  /** The persisted accent to restore when the user cancels. */
  const originalAccentRef = useRef(accent);
  /** Set once onApply runs, so the unmount restore skips after a save. */
  const appliedRef = useRef(false);

  const start = hexToRgb(current) ?? { r: 31, g: 154, b: 114 };
  const [hsv, setHsv] = useState(() => rgbToHsv(start.r, start.g, start.b));
  const [hexText, setHexText] = useState(() => rgbToHex(start.r, start.g, start.b));

  const hex = useMemo(() => {
    const { r, g, b } = hsvToRgb(hsv.h, hsv.s, hsv.v);
    return rgbToHex(r, g, b);
  }, [hsv]);

  // Pointer picks drive hsv → hex; mirror that into the editable hex field so
  // submit (which reads the field first) saves what the user actually picked.
  // Typing still wins while editing: invalid text leaves hsv — and therefore
  // hex — untouched, so the field is not clobbered mid-edit.
  useEffect(() => {
    setHexText(hex);
  }, [hex]);

  // Live preview: the whole UI re-skins while dragging.
  useEffect(() => {
    applyAccentToDom(hex, theme);
  }, [hex, theme]);

  // Restore the pre-picker accent on close-without-save.
  useEffect(
    () => () => {
      if (!appliedRef.current) {
        applyAccentToDom(originalAccentRef.current, theme);
      }
    },
    [theme],
  );

  const svRef = useRef<HTMLDivElement>(null);
  const hueRef = useRef<HTMLDivElement>(null);

  function pickFromEvent(
    e: ReactPointerEvent<HTMLDivElement>,
    axis: "sv" | "hue",
  ) {
    const el = axis === "sv" ? svRef.current : hueRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const x = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const y = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height));
    if (axis === "sv") {
      setHsv((p) => ({ ...p, s: x, v: 1 - y }));
    } else {
      setHsv((p) => ({ ...p, h: x * 360 }));
    }
  }

  function onHexInput(v: string) {
    setHexText(v);
    const rgb = hexToRgb(v);
    if (rgb) setHsv(rgbToHsv(rgb.r, rgb.g, rgb.b));
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    const rgb = hexToRgb(hexText) ?? hexToRgb(hex);
    const finalHex = rgb ? rgbToHex(rgb.r, rgb.g, rgb.b) : hex;
    appliedRef.current = true;
    onApply(finalHex);
  }

  return (
    <div className="modal-backdrop">
      <div className="modal accent-picker-modal">
        <header className="modal-header">
          <h2>{title}</h2>
          <button type="button" className="icon-btn" onClick={onClose}>
            ×
          </button>
        </header>
        <form className="modal-body" onSubmit={submit}>
          <div
            ref={svRef}
            className="accent-sv"
            style={{
              background: `linear-gradient(to top, #000, transparent),
                linear-gradient(to right, #fff, transparent),
                hsl(${hsv.h} 100% 50%)`,
            }}
            onPointerDown={(e) => {
              e.currentTarget.setPointerCapture(e.pointerId);
              pickFromEvent(e, "sv");
            }}
            onPointerMove={(e) => {
              if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                pickFromEvent(e, "sv");
              }
            }}
          >
            <span
              className="accent-sv-dot"
              style={{
                left: `${hsv.s * 100}%`,
                top: `${(1 - hsv.v) * 100}%`,
                background: hex,
              }}
            />
          </div>
          <div className="accent-hue-row">
            <div
              ref={hueRef}
              className="accent-hue"
              onPointerDown={(e) => {
                e.currentTarget.setPointerCapture(e.pointerId);
                pickFromEvent(e, "hue");
              }}
              onPointerMove={(e) => {
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  pickFromEvent(e, "hue");
                }
              }}
            >
              <span
                className="accent-hue-thumb"
                style={{ left: `${(hsv.h / 360) * 100}%` }}
              />
            </div>
            <span
              className="accent-preview"
              style={{ background: hex }}
              aria-hidden
            />
          </div>
          <label className="field accent-hex-field">
            <span>HEX</span>
            <input
              value={hexText}
              onChange={(e) => onHexInput(e.target.value)}
              placeholder="#rrggbb"
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
            />
          </label>
          <footer className="modal-footer">
            <GlassButton onClick={onClose}>{cancelLabel}</GlassButton>
            <GlassButton variant="primary" type="submit">
              {applyLabel}
            </GlassButton>
          </footer>
        </form>
      </div>
    </div>
  );
}
