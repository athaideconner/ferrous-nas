import { useEffect, useState, type ReactNode } from "react";

/* ---------------- toasts (tiny module-level bus) ---------------- */

type Toast = { id: number; msg: string; err: boolean };
let toastId = 0;
const listeners = new Set<(t: Toast[]) => void>();
let current: Toast[] = [];

function emit() {
  for (const l of listeners) l(current);
}
export function toast(msg: string, err = false) {
  const t = { id: ++toastId, msg, err };
  current = [...current, t];
  emit();
  setTimeout(() => {
    current = current.filter((x) => x.id !== t.id);
    emit();
  }, 3800);
}

export function Toasts() {
  const [items, setItems] = useState<Toast[]>(current);
  useEffect(() => {
    listeners.add(setItems);
    return () => {
      listeners.delete(setItems);
    };
  }, []);
  return (
    <div className="toast-wrap">
      {items.map((t) => (
        <div key={t.id} className={"toast" + (t.err ? " err" : "")}>
          {t.err ? "⚠️ " : "✓ "}
          {t.msg}
        </div>
      ))}
    </div>
  );
}

/* ---------------- primitives ---------------- */

export function Spinner() {
  return <div className="spinner" />;
}

export function Badge({ tone, children }: { tone: string; children: ReactNode }) {
  return <span className={"badge " + tone}>{children}</span>;
}

export function Meter({ pct, tone }: { pct: number; tone?: string }) {
  const auto = pct >= 90 ? "red" : pct >= 75 ? "yellow" : "";
  return (
    <div className={"meter " + (tone ?? auto)}>
      <span style={{ width: `${Math.min(100, Math.max(0, pct))}%` }} />
    </div>
  );
}

/** A circular percentage gauge drawn with SVG. */
export function Ring({ pct, size = 92, label, sub }: { pct: number; size?: number; label?: string; sub?: string }) {
  const stroke = 9;
  const r = (size - stroke) / 2;
  const c = 2 * Math.PI * r;
  const clamped = Math.min(100, Math.max(0, pct));
  const color = clamped >= 90 ? "var(--red)" : clamped >= 75 ? "var(--yellow)" : "var(--accent)";
  return (
    <div className="ring">
      <svg width={size} height={size}>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--bg-elev-2)" strokeWidth={stroke} />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke={color}
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={c}
          strokeDashoffset={c * (1 - clamped / 100)}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
          style={{ transition: "stroke-dashoffset 0.5s" }}
        />
        <text x="50%" y="47%" textAnchor="middle" dominantBaseline="middle" fontSize="19" fontWeight="700">
          {label ?? `${Math.round(clamped)}%`}
        </text>
        {sub && (
          <text x="50%" y="64%" textAnchor="middle" fontSize="10" fill="var(--text-faint)">
            {sub}
          </text>
        )}
      </svg>
    </div>
  );
}

export function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="switch">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span className="track" />
      <span className="thumb" />
    </label>
  );
}

/** A compact line chart from an array of numbers. */
export function Sparkline({ data, color = "var(--accent)", height = 44 }: { data: number[]; color?: string; height?: number }) {
  const w = 100;
  if (data.length < 2) return <svg width="100%" height={height} viewBox={`0 0 ${w} ${height}`} preserveAspectRatio="none" />;
  const max = Math.max(...data, 0.001);
  const min = Math.min(...data, 0);
  const range = max - min || 1;
  const pts = data.map((v, i) => {
    const x = (i / (data.length - 1)) * w;
    const y = height - ((v - min) / range) * (height - 6) - 3;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });
  const area = `0,${height} ${pts.join(" ")} ${w},${height}`;
  const id = "g" + color.replace(/[^a-z0-9]/gi, "");
  return (
    <svg width="100%" height={height} viewBox={`0 0 ${w} ${height}`} preserveAspectRatio="none">
      <defs>
        <linearGradient id={id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.35" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <polygon points={area} fill={`url(#${id})`} />
      <polyline points={pts.join(" ")} fill="none" stroke={color} strokeWidth="1.6" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

export function Modal({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        {children}
      </div>
    </div>
  );
}

export function StatTile({ label, value, unit, foot }: { label: string; value: ReactNode; unit?: string; foot?: ReactNode }) {
  return (
    <div className="card stat">
      <div className="label">{label}</div>
      <div className="value">
        {value}
        {unit && <small> {unit}</small>}
      </div>
      {foot && <div className="foot">{foot}</div>}
    </div>
  );
}

/** Wrap a page body; shows spinner / error consistently. */
export function Async<T>({ state, children }: { state: { data: T | null; error: string | null; loading: boolean }; children: (data: T) => ReactNode }) {
  if (state.loading && !state.data) return <Spinner />;
  if (state.error && !state.data) return <div className="empty">Failed to load: {state.error}</div>;
  if (!state.data) return <div className="empty">No data</div>;
  return <>{children(state.data)}</>;
}
