import React, { useEffect, useMemo, useRef, useState, useCallback, useLayoutEffect } from "react";
import { createRoot } from "react-dom/client";
import {
  MdCenterFocusWeak,
  MdChevronLeft,
  MdChevronRight,
  MdClose,
  MdRefresh,
  MdSwapVert,
  MdPlayArrow,
  MdBookmarkAdd,
  MdDelete,
  MdSettings,
  MdLink,
  MdLinkOff,
  MdKeyboardArrowUp,
  MdKeyboardArrowDown,
} from "react-icons/md";
import "./style.css";

const DEFAULT_ELASTIC = { amplitude: 0.5, frequency: 2, decay: 5, reversed: false };
const DEFAULT_BOUNCE = { cor: 0.5, period: 0.5, reversed: false };
const clone = (v) => JSON.parse(JSON.stringify(v));
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));

const CURVE_DATA_KEY = {
  bezier: "bezierData",
  elastic: "elasticData",
  bounce: "bounceData",
};

function curveDataFromState(state) {
  return state.mode === "bezier"
    ? state.bezierData
    : state.mode === "elastic"
      ? state.elasticData
      : state.bounceData;
}

const isFiniteNumber = (value) =>
  typeof value === "number" && Number.isFinite(value);
const isPoint = (value) =>
  value &&
  typeof value === "object" &&
  [value.anchor, value.inHandle, value.outHandle].every(
    (pair) =>
      Array.isArray(pair) &&
      pair.length === 2 &&
      pair.every(isFiniteNumber)
  ) &&
  (value.handlesLinked == null ||
    typeof value.handlesLinked === "boolean");

function isValidCurveData(mode, data) {
  if (!data || typeof data !== "object") return false;
  if (mode === "bezier") {
    return (
      Array.isArray(data.points) &&
      data.points.length >= 2 &&
      data.points.length <= 256 &&
      data.points.every(isPoint)
    );
  }
  if (mode === "elastic") {
    return (
      ["amplitude", "frequency", "decay"].every((key) =>
        isFiniteNumber(data[key])
      ) &&
      (data.reversed === undefined || typeof data.reversed === "boolean")
    );
  }
  if (mode === "bounce") {
    return (
      ["cor", "period"].every((key) => isFiniteNumber(data[key])) &&
      (data.reversed === undefined || typeof data.reversed === "boolean")
    );
  }
  return false;
}

function handlesAreLinked(point) {
  if (point.handlesLinked === true) return true;
  if (point.handlesLinked === false) return false;
  // 古いプリセットなど、フラグがないデータは実際の形状から推測する。
  // 統合は「向きが反対」であることだけを意味し、長さは一致しなくてよい。
  const [anchorX, anchorY] = point.anchor;
  const inX = point.inHandle[0] - anchorX;
  const inY = point.inHandle[1] - anchorY;
  const outX = point.outHandle[0] - anchorX;
  const outY = point.outHandle[1] - anchorY;
  const inLen = Math.hypot(inX, inY);
  const outLen = Math.hypot(outX, outY);
  if (inLen < 1e-9 || outLen < 1e-9) return false;
  const cross = inX * outY - inY * outX;
  const dot = inX * outX + inY * outY;
  return Math.abs(cross) < 1e-6 * inLen * outLen && dot < 0;
}

function isEditableTarget(target) {
  const element = target instanceof Element ? target : null;
  return Boolean(
    element &&
    (element.isContentEditable ||
      element.closest("input, textarea, select, [contenteditable='true']"))
  );
}

function isSpaceShortcutTarget(target) {
  // 入力欄以外では Space を AviUtl2 本体へ渡す。ボタンにフォーカスが
  // 残っていても「適用」の再実行ではなく再生/停止になる。
  return !isEditableTarget(target);
}

function stateWithCurve(state, curve) {
  const key = CURVE_DATA_KEY[curve.mode] || CURVE_DATA_KEY.bezier;
  return {
    ...state,
    mode: CURVE_DATA_KEY[curve.mode] ? curve.mode : "bezier",
    [key]: clone(curve.data),
  };
}

function readJson(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    const value = JSON.parse(raw);
    return value == null ? fallback : value;
  } catch {
    return fallback;
  }
}

function writeJson(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // localStorage が使えない環境でも動作を継続する。
  }
}

function sanitizeSettings(value) {
  const theme = value && typeof value.theme === "object" ? value.theme : {};
  return {
    presetPanelLocation: ["right", "left", "top", "bottom"].includes(
      value?.presetPanelLocation
    )
      ? value.presetPanelLocation
      : "right",
    presetPanelSize: clamp(Number(value?.presetPanelSize) || 200, 60, 2000),
    presetBoxSize: clamp(Number(value?.presetBoxSize) || 78, 40, 140),
    showPresetPanel: value?.showPresetPanel !== false,
    dragDropEnabled: value?.dragDropEnabled !== false,
    groupApplyOnDrop: value?.groupApplyOnDrop === true,
    compactMode: true,
    gridLinesX: clamp(Number(value?.gridLinesX) || 4, 1, 50),
    gridLinesY: clamp(Number(value?.gridLinesY) || 4, 1, 50),
    precision: clamp(Number(value?.precision) || 150, 50, 500),
    theme: {
      bgColor: theme.bgColor || "#1d1d1d",
      gridColor: theme.gridColor || "#282828",
      mainGridColor: theme.mainGridColor || "#3d3d3d",
      curveColor: "#3b82f6",
      handleColor: theme.handleColor || "#888888",
      anchorColor: theme.anchorColor || "#ffffff",
    },
  };
}

// プリセット・IDパネル用のサムネイル。0.8倍で描画する。
function makeThumb(mode, data, theme) {
  const c = document.createElement("canvas");
  c.width = 90;
  c.height = 90;
  const x = c.getContext("2d");
  if (!x) return "";
  x.fillStyle = theme.bgColor;
  x.fillRect(0, 0, 90, 90);
  x.strokeStyle = theme.curveColor;
  x.lineWidth = 2;
  x.beginPath();

  const margin = 0.1;
  const scale = 0.8;

  for (let i = 0; i <= 40; i++) {
    const t = i / 40;
    let y;
    try {
      y =
        mode === "bezier"
          ? evalBezier(data, t)
          : mode === "elastic"
            ? elastic(
              t,
              data.amplitude,
              data.frequency,
              data.decay,
              data.reversed
            )
            : bounce(t, data.cor, data.period, data.reversed);
    } catch {
      y = t;
    }
    if (!Number.isFinite(y)) y = t;
    const px = (margin + t * scale) * 90;
    const py = 90 - (margin + y * scale) * 90;
    if (!i) x.moveTo(px, py);
    else x.lineTo(px, py);
  }
  x.stroke();
  return c.toDataURL();
}



function defaultBezier() {
  return {
    points: [
      {
        anchor: [0, 0],
        inHandle: [0, 0],
        outHandle: [0.33, 0],
        outType: "bezier",
        outElasticParams: clone(DEFAULT_ELASTIC),
        outBounceParams: clone(DEFAULT_BOUNCE),
        handlesLinked: true,
      },
      {
        anchor: [1, 1],
        inHandle: [0.67, 1],
        outHandle: [1, 1],
        outType: "bezier",
        handlesLinked: true,
      },
    ],
  };
}

function elastic(p, a, f, d, reversed) {
  if (p <= 0) return 0;
  if (p >= 1) return 1;
  if (reversed) p = 1 - p;
  a = clamp(Number.isFinite(a) ? a : 0.5, 0, 1);
  f = clamp(Number.isFinite(f) ? f : 2, 0.001, 1000);
  d = clamp(Number.isFinite(d) ? d : 5, 0, 100);
  let et = (0.5 - Math.sqrt(Math.max(0, d / f)) * 0.05) / f;
  const d1 = (x) => {
    const w = 2 * Math.PI * f;
    const ek = Math.exp(-d);
    const an = w * x;
    const cs = Math.cos(an);
    const sn = Math.sin(an);
    if (!d) return cs + w * (1 - x) * sn;
    const e = Math.pow(ek, x);
    return d * e * cs + w * (e - ek) * sn;
  };
  const d2 = (x) => {
    const w = 2 * Math.PI * f;
    const ek = Math.exp(-d);
    const an = w * x;
    const cs = Math.cos(an);
    const sn = Math.sin(an);
    const e = Math.pow(ek, x);
    const w2 = w * w;
    if (!d) return w2 * (1 - x) * cs - 2 * w * sn;
    return ((w2 - d * d) * e - w2 * ek) * cs - 2 * w * d * e * sn;
  };
  for (let i = 0; i < 3; i++) {
    const dd = d2(et);
    if (Math.abs(dd) < 1e-8) break;
    et -= d1(et) / dd;
  }
  const fn = (x) => {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    const c = d
      ? (Math.pow(Math.exp(-d), x) - Math.exp(-d)) / (1 - Math.exp(-d))
      : 1 - x;
    return 1 - c * Math.cos(2 * Math.PI * f * x);
  };
  const ex = fn(et);
  const tmp = fn(p);
  const r = p < et ? ((a * (ex - 1) + 1) / ex) * tmp : a * (tmp - 1) + 1;
  return reversed ? 1 - r : r;
}

function bounce(p, cor, period, reversed) {
  if (p <= 0) return 0;
  if (p >= 1) return 1;
  const c = clamp(Number.isFinite(cor) ? cor : 0.5, 0.000001, 0.999999);
  const d = clamp(Number.isFinite(period) ? period : 0.5, 0.0001, 100);
  if (reversed) p = 1 - p;
  const limit = d * (1 / (1 - c) - 0.5);
  const f1 = (x) => Math.floor(Math.log((c - 1) * x + 1) / Math.log(c));
  const f2 = (x) =>
    x + 0.5 + 1 / (c - 1) - ((c + 1) * Math.pow(c, f1(x + 0.5))) / (2 * c - 2);
  const q = 4 * f2(p / d) ** 2 - Math.pow(c, 2 * f1(p / d + 0.5));
  let r;
  if (limit > 1) {
    const n = Math.floor(
      Math.log(1 + (c - 1) * (1 / d + 0.5)) / Math.log(c)
    );
    r =
      p < d * ((Math.pow(c, n) - 1) / (c - 1) - 0.5)
        ? q
        : 0;
  } else r = p < limit ? q : 0;
  return reversed ? 1 - (1 + r) : 1 + r;
}

function cubic(t, p0, p1, p2, p3) {
  const u = 1 - t;
  return u ** 3 * p0 + 3 * u ** 2 * t * p1 + 3 * u * t ** 2 * p2 + t ** 3 * p3;
}

function solveX(x, c1, c2) {
  let t = x;
  for (let i = 0; i < 8; i++) {
    const e = cubic(t, 0, c1, c2, 1) - x;
    if (Math.abs(e) < 1e-6) return clamp(t, 0, 1);
    const d =
      (3 * c1 - 6 * c2 + 3) * t * t + (-6 * c1 + 6 * c2) * t + 3 * c1;
    if (Math.abs(d) < 1e-6) break;
    t -= e / d;
  }
  if (Math.abs(cubic(t, 0, c1, c2, 1) - x) < 1e-6) return clamp(t, 0, 1);
  // ニュートン法が収束しない場合(ハンドルが端点近くに固まっている場合など)は二分法で厳密に解く
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 60; i++) {
    const mid = (lo + hi) / 2;
    if (cubic(mid, 0, c1, c2, 1) < x) lo = mid;
    else hi = mid;
  }
  return (lo + hi) / 2;
}

function evalBezier(data, x) {
  const pts = data.points;
  if (!pts.length) return x;
  if (x <= pts[0].anchor[0]) return pts[0].anchor[1];
  if (x >= pts[pts.length - 1].anchor[0]) return pts[pts.length - 1].anchor[1];
  for (let i = 0; i < pts.length - 1; i++) {
    const a = pts[i];
    const b = pts[i + 1];
    if (x < a.anchor[0] || x > b.anchor[0]) continue;
    const w = b.anchor[0] - a.anchor[0];
    if (w < 1e-6) return a.anchor[1];
    const local = (x - a.anchor[0]) / w;
    if (a.outType === "elastic") {
      const p = a.outElasticParams || DEFAULT_ELASTIC;
      return (
        a.anchor[1] +
        elastic(local, p.amplitude, p.frequency, p.decay, p.reversed) *
        (b.anchor[1] - a.anchor[1])
      );
    }
    if (a.outType === "bounce") {
      const p = a.outBounceParams || DEFAULT_BOUNCE;
      return (
        a.anchor[1] +
        bounce(local, p.cor, p.period, p.reversed) *
        (b.anchor[1] - a.anchor[1])
      );
    }
    return cubic(
      solveX(
        local,
        (a.outHandle[0] - a.anchor[0]) / w,
        (b.inHandle[0] - a.anchor[0]) / w
      ),
      a.anchor[1],
      a.outHandle[1],
      b.inHandle[1],
      b.anchor[1]
    );
  }
  return pts[pts.length - 1].anchor[1];
}

function clampHandles(points) {
  const lastIndex = points.length - 1;
  for (let i = 0; i < points.length; i++) {
    const p = points[i];
    const prevX = i > 0 ? points[i - 1].anchor[0] : 0;
    const nextX = i < lastIndex ? points[i + 1].anchor[0] : 1;
    const [anchorX, anchorY] = p.anchor;

    if (i > 0 && i < lastIndex && handlesAreLinked(p)) {
      // 統合時は向きだけを反対に揃える。長さは左右で独立して保持し、
      // 前後のアンカー範囲を超えた側だけを短縮する。
      const outX = p.outHandle[0] - anchorX;
      const outY = p.outHandle[1] - anchorY;
      const inX = p.inHandle[0] - anchorX;
      const inY = p.inHandle[1] - anchorY;
      let outLen = Math.hypot(outX, outY);
      let inLen = Math.hypot(inX, inY);
      let ux = 0;
      let uy = 0;

      if (outLen > 1e-9 && outX >= -1e-9) {
        ux = outX / outLen;
        uy = outY / outLen;
      } else if (inLen > 1e-9 && inX <= 1e-9) {
        ux = -inX / inLen;
        uy = -inY / inLen;
      } else {
        outLen = 0;
        inLen = 0;
      }

      if (outLen === 0 && inLen === 0) {
        p.outHandle = [anchorX, anchorY];
        p.inHandle = [anchorX, anchorY];
      } else {
        if (ux > 1e-9) {
          outLen = Math.min(outLen, Math.max(0, (nextX - anchorX) / ux));
          inLen = Math.min(inLen, Math.max(0, (anchorX - prevX) / ux));
        }
        if (Math.abs(uy) > 1e-9) {
          const maxY = Math.max(0, 10 - Math.abs(anchorY)) / Math.abs(uy);
          outLen = Math.min(outLen, maxY);
          inLen = Math.min(inLen, maxY);
        }
        outLen = Math.max(0, outLen);
        inLen = Math.max(0, inLen);
        p.outHandle = [anchorX + ux * outLen, anchorY + uy * outLen];
        p.inHandle = [anchorX - ux * inLen, anchorY - uy * inLen];
      }
    } else {
      if (p.inHandle) {
        p.inHandle[0] = clamp(p.inHandle[0], prevX, anchorX);
      }
      if (p.outHandle) {
        p.outHandle[0] = clamp(p.outHandle[0], anchorX, nextX);
      }
    }
  }
}

// ポップアップメニューの位置をウィンドウ領域内に収める
function ClampedMenu({ x, y, className, children, onMouseDown, onClick }) {
  const ref = useRef(null);
  const [pos, setPos] = useState({ left: x, top: y });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) {
      setPos({ left: x, top: y });
      return;
    }
    const rect = el.getBoundingClientRect();
    const margin = 4;
    const maxLeft = window.innerWidth - rect.width - margin;
    const maxTop = window.innerHeight - rect.height - margin;
    setPos({
      left: Math.max(margin, Math.min(x, maxLeft)),
      top: Math.max(margin, Math.min(y, maxTop)),
    });
  }, [x, y]);
  return (
    <div
      ref={ref}
      className={className}
      style={{ left: pos.left, top: pos.top }}
      onMouseDown={onMouseDown}
      onClick={onClick}
    >
      {children}
    </div>
  );
}

function IdInput({ value, onCommit }) {
  const [text, setText] = useState(value == null ? "" : String(value));
  const textRef = useRef(text);
  const drag = useRef(null);
  const lastCommitted = useRef(value);

  const updateText = useCallback((next) => {
    textRef.current = next;
    setText(next);
  }, []);

  useEffect(() => {
    const next = value == null ? "" : String(value);
    textRef.current = next;
    setText(next);
    lastCommitted.current = value;
  }, [value]);

  const commit = useCallback(
    (next) => {
      const parsed = clamp(Math.round(Number(next) || 1), 1, 999999);
      updateText(String(parsed));
      if (parsed !== lastCommitted.current) {
        lastCommitted.current = parsed;
        onCommit(parsed);
      }
    },
    [onCommit, updateText]
  );

  // 入力中は少し待ってからIDを切り替える。ドラッグ中は即時反映。
  useEffect(() => {
    if (text === "") return;
    const timer = setTimeout(() => {
      if (!drag.current) commit(text);
    }, 300);
    return () => clearTimeout(timer);
  }, [text, commit]);

  const step = (delta) => {
    const current = Number(textRef.current) || value || 1;
    commit(current + delta);
  };

  return (
    <div className="id-control">
      <button
        type="button"
        className="id-step"
        title="カーブIDを1減らす"
        aria-label="カーブIDを1減らす"
        onClick={() => step(-1)}
      >
        <MdChevronLeft />
      </button>
      <input
        className="id-input"
        type="number"
        min="1"
        max="999999"
        step="1"
        value={text}
        title="カーブID（数値入力、左右ドラッグ、矢印ボタンで変更）"
        onChange={(e) => updateText(e.target.value)}
        onBlur={() => commit(textRef.current)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            commit(textRef.current);
            e.currentTarget.blur();
          }
        }}
        onPointerDown={(e) => {
          drag.current = {
            x: e.clientX,
            value: Number(text) || value || 1,
          };
          e.currentTarget.setPointerCapture(e.pointerId);
        }}
        onPointerMove={(e) => {
          if (!drag.current) return;
          const delta = e.clientX - drag.current.x;
          if (Math.abs(delta) < 4) return;
          e.preventDefault();
          updateText(
            String(
              clamp(
                drag.current.value + Math.round(delta / 4),
                1,
                999999
              )
            )
          );
        }}
        onPointerUp={() => {
          if (!drag.current) return;
          drag.current = null;
          commit(textRef.current);
        }}
        onPointerCancel={() => {
          drag.current = null;
          updateText(value == null ? "" : String(value));
        }}
      />
      <button
        type="button"
        className="id-step"
        title="カーブIDを1増やす"
        aria-label="カーブIDを1増やす"
        onClick={() => step(1)}
      >
        <MdChevronRight />
      </button>
    </div>
  );
}

function Graph({ state, setState, settings, onCommitEdit }) {
  const ref = useRef(null);
  const view = useRef({ x: 0.5, y: 0.5, zoom: 1.2 }); // 0.75倍の初期ズーム表示
  const drag = useRef(null);
  const keys = useRef({ shift: false, alt: false });
  const [contextMenu, setContextMenu] = useState(null);
  const [isModifiedView, setIsModifiedView] = useState(false);

  const checkViewModified = useCallback(() => {
    const modified =
      Math.abs(view.current.x - 0.5) > 0.01 ||
      Math.abs(view.current.y - 0.5) > 0.01 ||
      Math.abs(view.current.zoom - 1.2) > 0.01;
    setIsModifiedView(modified);
  }, []);

  const resetView = () => {
    view.current = { x: 0.5, y: 0.5, zoom: 1.2 };
    checkViewModified();
    draw();
  };

  // 短辺(Math.min(w, h))を基準にし、1:1正方形グリッドのアスペクト比を完全保持する座標変換
  const tr = useCallback(() => {
    const c = ref.current;
    if (!c) return { toC: () => [0, 0], toG: () => [0, 0] };
    const w = c.width;
    const h = c.height;
    const minDim = Math.min(w, h) || 1;
    const baseRange = 2.0 / view.current.zoom;
    const rangeX = baseRange * (w / minDim);
    const rangeY = baseRange * (h / minDim);
    const minX = view.current.x - rangeX / 2;
    const minY = view.current.y - rangeY / 2;
    return {
      toC: (x, y) => [
        ((x - minX) / rangeX) * w,
        (1 - (y - minY) / rangeY) * h,
      ],
      toG: (x, y) => [
        minX + (x / w) * rangeX,
        minY + (1 - y / h) * rangeY,
      ],
    };
  }, []);

  const drawGrid = (ctx, transform) => {
    const c = ref.current;
    if (!c) return;
    const sx = 1 / Math.max(1, settings.gridLinesX);
    const sy = 1 / Math.max(1, settings.gridLinesY);

    for (
      let x = Math.floor(transform.toG(0, 0)[0] / sx) * sx;
      x <= transform.toG(c.width, 0)[0] + sx;
      x += sx
    ) {
      const px = transform.toC(x, 0)[0];
      ctx.strokeStyle =
        Math.abs(x - Math.round(x)) < 1e-7
          ? settings.theme.mainGridColor
          : settings.theme.gridColor;
      ctx.beginPath();
      ctx.moveTo(px, 0);
      ctx.lineTo(px, c.height);
      ctx.stroke();
    }
    for (
      let y = Math.floor(transform.toG(0, c.height)[1] / sy) * sy;
      y <= transform.toG(0, 0)[1] + sy;
      y += sy
    ) {
      const py = transform.toC(0, y)[1];
      ctx.strokeStyle =
        Math.abs(y - Math.round(y)) < 1e-7
          ? settings.theme.mainGridColor
          : settings.theme.gridColor;
      ctx.beginPath();
      ctx.moveTo(0, py);
      ctx.lineTo(c.width, py);
      ctx.stroke();
    }
  };

  const point = (transform, p) => transform.toC(p[0], p[1]);

  const drawHandle = (ctx, transform, a, h) => {
    const p = point(transform, a);
    const q = point(transform, h);
    ctx.strokeStyle = settings.theme.handleColor;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(...p);
    ctx.lineTo(...q);
    ctx.stroke();
    ctx.fillStyle = settings.theme.anchorColor;
    ctx.beginPath();
    ctx.arc(q[0], q[1], 4, 0, Math.PI * 2);
    ctx.fill();
  };

  const segmentHandle = (ctx, transform, a, b, type, params) => {
    const dx = b.anchor[0] - a.anchor[0];
    const dy = b.anchor[1] - a.anchor[1];
    if (type === "bounce") {
      let x = params.period * (params.cor + 1) * 0.5;
      let y = 1 - params.cor ** 2;
      if (params.reversed) {
        x = 1 - x;
        y = 1 - y;
      }
      const q = point(transform, [a.anchor[0] + dx * x, a.anchor[1] + dy * y]);
      const root = point(transform, [
        a.anchor[0] + dx * x,
        params.reversed ? a.anchor[1] : b.anchor[1],
      ]);
      ctx.strokeStyle = settings.theme.handleColor;
      ctx.beginPath();
      ctx.moveTo(...root);
      ctx.lineTo(...q);
      ctx.stroke();
      ctx.fillStyle = settings.theme.anchorColor;
      ctx.beginPath();
      ctx.arc(...q, 5, 0, Math.PI * 2);
      ctx.fill();
    } else {
      const x = params.reversed
        ? 1 - 0.5 / params.frequency
        : 0.5 / params.frequency;
      const y = params.reversed
        ? params.amplitude
        : 1 - params.amplitude;
      const decayX = params.reversed ? a.anchor[0] : b.anchor[0];
      const decayY = params.reversed
        ? 1 - (params.decay - 1) / 19
        : (params.decay - 1) / 19;
      drawHandle(
        ctx,
        transform,
        [
          a.anchor[0] + dx * x,
          params.reversed ? a.anchor[1] : b.anchor[1],
        ],
        [a.anchor[0] + dx * x, a.anchor[1] + dy * y]
      );
      drawHandle(
        ctx,
        transform,
        [decayX, params.reversed ? a.anchor[1] : b.anchor[1]],
        [decayX, a.anchor[1] + dy * decayY]
      );
    }
  };

  const draw = useCallback(() => {
    const c = ref.current;
    if (!c) return;
    const ctx = c.getContext("2d");
    const transform = tr();
    ctx.fillStyle = settings.theme.bgColor;
    ctx.fillRect(0, 0, c.width, c.height);
    ctx.lineWidth = 1;
    drawGrid(ctx, transform);
    ctx.strokeStyle = settings.theme.curveColor;
    ctx.lineWidth = 2;
    ctx.beginPath();

    const fn =
      state.mode === "bezier"
        ? (x) => evalBezier(state.bezierData, x)
        : state.mode === "elastic"
          ? (x) =>
            elastic(
              x,
              state.elasticData.amplitude,
              state.elasticData.frequency,
              state.elasticData.decay,
              state.elasticData.reversed
            )
          : (x) =>
            bounce(
              x,
              state.bounceData.cor,
              state.bounceData.period,
              state.bounceData.reversed
            );

    for (let i = 0; i <= settings.precision; i++) {
      const x = i / settings.precision;
      const p = point(transform, [x, fn(x)]);
      if (!i) ctx.moveTo(...p);
      else ctx.lineTo(...p);
    }
    ctx.stroke();

    if (state.mode === "bezier") {
      state.bezierData.points.forEach((p, i) => {
        if (i > 0 && state.bezierData.points[i - 1].outType === "bezier")
          drawHandle(ctx, transform, p.anchor, p.inHandle);
        if (i < state.bezierData.points.length - 1) {
          if (p.outType === "bezier")
            drawHandle(ctx, transform, p.anchor, p.outHandle);
          else
            segmentHandle(
              ctx,
              transform,
              p,
              state.bezierData.points[i + 1],
              p.outType,
              p.outType === "elastic"
                ? p.outElasticParams || DEFAULT_ELASTIC
                : p.outBounceParams || DEFAULT_BOUNCE
            );
        }
        const q = point(transform, p.anchor);
        ctx.fillStyle = settings.theme.anchorColor;
        ctx.fillRect(q[0] - 4, q[1] - 4, 8, 8);
      });
    } else if (state.mode === "elastic") {
      const p = state.elasticData;
      const x = p.reversed ? 1 - 0.5 / p.frequency : 0.5 / p.frequency;
      const y = p.reversed ? p.amplitude : 1 - p.amplitude;
      const dx = p.reversed ? 0 : 1;
      const dy = p.reversed ? 1 - (p.decay - 1) / 19 : (p.decay - 1) / 19;
      drawHandle(ctx, transform, [x, p.reversed ? 0 : 1], [x, y]);
      drawHandle(ctx, transform, [dx, p.reversed ? 0 : 1], [dx, dy]);
    } else {
      const p = state.bounceData;
      let x = p.period * (p.cor + 1) * 0.5;
      let y = 1 - p.cor ** 2;
      if (p.reversed) {
        x = 1 - x;
        y = 1 - y;
      }
      const q = point(transform, [x, y]);
      ctx.fillStyle = settings.theme.anchorColor;
      ctx.beginPath();
      ctx.arc(...q, 5, 0, Math.PI * 2);
      ctx.fill();
    }
  }, [state, settings, tr]);

  const drawRef = useRef(null);
  drawRef.current = draw;

  const resize = useCallback(() => {
    const c = ref.current;
    if (!c) return;
    const r = c.parentElement.getBoundingClientRect();
    c.width = Math.max(1, Math.floor(r.width));
    c.height = Math.max(1, Math.floor(r.height));
    drawRef.current?.();
  }, []);

  useEffect(() => {
    resize();
    const c = ref.current;
    if (!c) return;
    const observer = new ResizeObserver(() => {
      resize();
    });
    if (c.parentElement) observer.observe(c.parentElement);

    window.addEventListener("resize", resize);
    const down = (e) => {
      if (e.key === "Shift") keys.current.shift = true;
      if (e.key === "Control") keys.current.ctrl = true;
      if (e.key === "Alt") keys.current.alt = true;
    };
    const up = (e) => {
      if (e.key === "Shift") keys.current.shift = false;
      if (e.key === "Control") keys.current.ctrl = false;
      if (e.key === "Alt") keys.current.alt = false;
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", resize);
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, [resize]);

  useEffect(() => {
    draw();
  }, [state, settings, draw]);

  const hit = (x, y) => {
    const transform = tr();
    const near = (p) => {
      const q = point(transform, p);
      return (q[0] - x) ** 2 + (q[1] - y) ** 2 < 14 ** 2;
    };
    if (state.mode === "bezier") {
      const pts = state.bezierData.points;
      for (let i = 0; i < pts.length; i++) {
        if (i > 0 && pts[i - 1].outType === "bezier" && near(pts[i].inHandle))
          return { kind: "in", index: i };
        if (
          i < pts.length - 1 &&
          pts[i].outType === "bezier" &&
          near(pts[i].outHandle)
        )
          return { kind: "out", index: i };
      }

      for (let i = 0; i < pts.length - 1; i++) {
        const p_start = pts[i];
        const p_end = pts[i + 1];
        const [x1, y1] = p_start.anchor;
        const [x2, y2] = p_end.anchor;
        const dx = x2 - x1;
        const dy = y2 - y1;

        if (p_start.outType === "elastic" && p_start.outElasticParams) {
          const p = p_start.outElasticParams;
          const xRel = p.reversed ? 1 - 0.5 / p.frequency : 0.5 / p.frequency;
          const yRel = p.reversed ? p.amplitude : 1 - p.amplitude;
          const decayX = p.reversed ? x1 : x2;
          const decayYRel = p.reversed
            ? 1 - (p.decay - 1) / 19
            : (p.decay - 1) / 19;

          if (near([x1 + dx * xRel, y1 + dy * yRel]))
            return { kind: "seg_elastic_param", index: i };
          if (near([decayX, y1 + dy * decayYRel]))
            return { kind: "seg_elastic_decay", index: i };
        } else if (p_start.outType === "bounce" && p_start.outBounceParams) {
          const p = p_start.outBounceParams;
          let xRel = p.period * (p.cor + 1) * 0.5;
          let yRel = 1 - p.cor ** 2;
          if (p.reversed) {
            xRel = 1 - xRel;
            yRel = 1 - yRel;
          }
          if (near([x1 + dx * xRel, y1 + dy * yRel]))
            return { kind: "seg_bounce_param", index: i };
        }
      }

      for (let i = 0; i < pts.length; i++)
        if (near(pts[i].anchor)) return { kind: "anchor", index: i };
    } else {
      let p = state.mode === "elastic" ? state.elasticData : state.bounceData;
      let xh =
        state.mode === "elastic"
          ? p.reversed
            ? 1 - 0.5 / p.frequency
            : 0.5 / p.frequency
          : p.period * (p.cor + 1) * 0.5;
      let yh =
        state.mode === "elastic"
          ? p.reversed
            ? p.amplitude
            : 1 - p.amplitude
          : 1 - p.cor ** 2;
      if (near([xh, yh])) return { kind: "param" };
      if (state.mode === "elastic") {
        const yd = p.reversed
          ? 1 - (p.decay - 1) / 19
          : (p.decay - 1) / 19;
        if (near([p.reversed ? 0 : 1, yd])) return { kind: "decay" };
      }
    }
    return null;
  };

  const moveAnchor = (index, value) => {
    const pts = clone(state.bezierData.points);
    const old = pts[index].anchor;
    const first = index === 0;
    const last = index === pts.length - 1;

    // 端点はXを固定(0/1)してYのみ移動、中間点はXも移動可能
    let x = old[0];
    if (!first && !last) {
      const minGap = 0.02;
      const min = pts[index - 1].anchor[0] + minGap;
      const max = pts[index + 1].anchor[0] - minGap;
      x = clamp(value[0], min, max);
    }
    // XのスナップはCtrl(Ctrl→X)のみ。ShiftはY軸スナップ専用
    const y = value[1];

    const origHandles = drag.current?.originalHandles;
    for (let i = 0; i < pts.length; i++) {
      if (origHandles && origHandles[i]) {
        const o = origHandles[i];
        if (pts[i].inHandle) {
          pts[i].inHandle = [pts[i].anchor[0] + o.inX, pts[i].anchor[1] + o.inY];
        }
        if (pts[i].outHandle) {
          pts[i].outHandle = [pts[i].anchor[0] + o.outX, pts[i].anchor[1] + o.outY];
        }
      }
    }

    const move = (i, dx, dy) => {
      const p = pts[i];
      p.anchor[0] += dx;
      p.anchor[1] += dy;
      if (p.inHandle) {
        p.inHandle[0] += dx;
        p.inHandle[1] += dy;
      }
      if (p.outHandle) {
        p.outHandle[0] += dx;
        p.outHandle[1] += dy;
      }
    };

    move(index, x - old[0], value[1] - old[1]);
    clampHandles(pts);
    setState((s) => ({ ...s, bezierData: { points: pts } }));
  };

  const onDown = (e) => {
    // グラフ操作後も Space / Ctrl+Z を WebView 側で受け取れるようにする。
    ref.current?.focus({ preventScroll: true });
    if (e.button === 2) return;
    const r = ref.current.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    if (e.button === 1) {
      drag.current = { kind: "pan", x, y, view: clone(view.current) };
      return;
    }
    if (e.button === 0) {
      const h = hit(x, y);
      if (h) {
        // ハンドル・アンカーを掴んだら、そちらを優先して操作する。
        // 分離/統合は右クリックメニューで切り替える。
        let originalHandles = null;
        if (state.mode === "bezier") {
          originalHandles = state.bezierData.points.map((p) => ({
            inX: p.inHandle ? p.inHandle[0] - p.anchor[0] : 0,
            inY: p.inHandle ? p.inHandle[1] - p.anchor[1] : 0,
            outX: p.outHandle ? p.outHandle[0] - p.anchor[0] : 0,
            outY: p.outHandle ? p.outHandle[1] - p.anchor[1] : 0,
          }));
        }
        drag.current = {
          ...h,
          originalHandles,
          startState: clone(state),
        };
      } else if (keys.current.alt) {
        // 何も掴んでいない場合のみ Altドラッグでパン
        drag.current = { kind: "pan", x, y, view: clone(view.current) };
      }
    }
  };

  const onMove = (e) => {
    if (!drag.current) return;
    const r = ref.current.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    const transform = tr();

    if (drag.current.kind === "pan") {
      view.current.x =
        drag.current.view.x -
        (((x - drag.current.x) / ref.current.width) * 2) / view.current.zoom;
      view.current.y =
        drag.current.view.y +
        (((y - drag.current.y) / ref.current.height) * 2) / view.current.zoom;
      checkViewModified();
      draw();
      return;
    }

    const value = transform.toG(x, y);

    // CtrlキーでX軸スナップ、ShiftキーでY軸スナップ
    if (keys.current.ctrl) {
      value[0] = Math.round(value[0] * settings.gridLinesX) / settings.gridLinesX;
    }
    if (keys.current.shift) {
      value[1] = Math.round(value[1] * settings.gridLinesY) / settings.gridLinesY;
    }

    if (drag.current.kind === "anchor") {
      moveAnchor(drag.current.index, value);
    } else if (
      state.mode === "bezier" &&
      (drag.current.kind === "in" || drag.current.kind === "out")
    ) {
      const pts = clone(state.bezierData.points);
      const p = pts[drag.current.index];
      const linked = handlesAreLinked(p);
      const isIn = drag.current.kind === "in";
      const handleValue = [
        isIn
          ? Math.min(p.anchor[0], value[0])
          : Math.max(p.anchor[0], value[0]),
        value[1],
      ];
      p[isIn ? "inHandle" : "outHandle"] = handleValue;
      if (linked) {
        const other = isIn ? "outHandle" : "inHandle";
        const dx = handleValue[0] - p.anchor[0];
        const dy = handleValue[1] - p.anchor[1];
        const length = Math.hypot(dx, dy);
        const otherLength = Math.hypot(
          p[other][0] - p.anchor[0],
          p[other][1] - p.anchor[1]
        );
        if (length > 1e-9) {
          p[other] = [
            p.anchor[0] - (dx / length) * otherLength,
            p.anchor[1] - (dy / length) * otherLength,
          ];
        }
        // 掴んでいるハンドルがアンカー上にあるときは、反対側を
        // アンカーへ巻き込まない。方向が決まらないだけで、長さは独立。
      }
      clampHandles(pts);
      setState((s) => ({ ...s, bezierData: { points: pts } }));
    } else if (
      state.mode === "bezier" &&
      (drag.current.kind === "seg_elastic_param" ||
        drag.current.kind === "seg_elastic_decay" ||
        drag.current.kind === "seg_bounce_param")
    ) {
      const pts = clone(state.bezierData.points);
      const pIndex = drag.current.index;
      const pStart = pts[pIndex];
      const pEnd = pts[pIndex + 1];
      const dx = pEnd.anchor[0] - pStart.anchor[0];
      const dy = pEnd.anchor[1] - pStart.anchor[1];

      if (Math.abs(dx) > 1e-6 && Math.abs(dy) > 1e-6) {
        let relX = (value[0] - pStart.anchor[0]) / dx;
        let relY = (value[1] - pStart.anchor[1]) / dy;

        if (
          drag.current.kind === "seg_elastic_param" &&
          pStart.outElasticParams
        ) {
          const params = pStart.outElasticParams;
          if (params.reversed) {
            relX = 1 - relX;
            relY = 1 - relY;
          }
          const newX = clamp(relX, 0.001, 1);
          const newY = clamp(relY, 0, 1);
          params.frequency = 0.5 / newX;
          params.amplitude = 1 - newY;
        } else if (
          drag.current.kind === "seg_elastic_decay" &&
          pStart.outElasticParams
        ) {
          const params = pStart.outElasticParams;
          if (params.reversed) relY = 1 - relY;
          params.decay = 19 * clamp(relY, 0, 1) + 1;
        } else if (
          drag.current.kind === "seg_bounce_param" &&
          pStart.outBounceParams
        ) {
          const params = pStart.outBounceParams;
          if (params.reversed) {
            relX = 1 - relX;
            relY = 1 - relY;
          }
          const cor = Math.sqrt(Math.max(0, 1 - clamp(relY, 0.001, 1)));
          params.cor = cor;
          params.period = (2 * clamp(relX, 0.001, 1)) / (cor + 1);
        }
      }
      setState((s) => ({ ...s, bezierData: { points: pts } }));
    } else if (state.mode === "elastic") {
      const p = clone(state.elasticData);
      if (drag.current.kind === "param") {
        const xx = clamp(value[0], 0.001, 1);
        p.frequency = 0.5 / (p.reversed ? Math.max(0.001, 1 - xx) : xx);
        p.amplitude = p.reversed
          ? clamp(value[1], 0, 1)
          : 1 - clamp(value[1], 0, 1);
      } else if (drag.current.kind === "decay")
        p.decay = p.reversed
          ? 19 * (1 - clamp(value[1], 0, 1)) + 1
          : 19 * clamp(value[1], 0, 1) + 1;
      setState((s) => ({ ...s, elasticData: p }));
    } else if (state.mode === "bounce") {
      let xx = clamp(value[0], 0, 1);
      let yy = clamp(value[1], 0, 1);
      if (state.bounceData.reversed) {
        xx = 1 - xx;
        yy = 1 - yy;
      }
      const cor = Math.sqrt(Math.max(0, 1 - yy));
      setState((s) => ({
        ...s,
        bounceData: {
          ...s.bounceData,
          cor,
          period: clamp((2 * xx) / (cor + 1), 0.01, 2),
        },
      }));
    }
    draw();
  };

  const onWheel = (e) => {
    e.preventDefault();
    view.current.zoom = clamp(
      view.current.zoom * (e.deltaY < 0 ? 1.2 : 1 / 1.2),
      0.1,
      50
    );
    checkViewModified();
    draw();
  };

  const onDouble = (e) => {
    if (state.mode !== "bezier") return;
    const r = ref.current.getBoundingClientRect();
    const value = tr().toG(e.clientX - r.left, e.clientY - r.top);
    const pts = clone(state.bezierData.points);
    const h = hit(e.clientX - r.left, e.clientY - r.top);
    if (h?.kind === "anchor" && h.index > 0 && h.index < pts.length - 1) {
      pts.splice(h.index, 1);
    } else if (!h && value[0] > 0 && value[0] < 1) {
      const i = pts.findIndex(
        (p, j) =>
          j < pts.length - 1 &&
          value[0] >= p.anchor[0] &&
          value[0] <= pts[j + 1].anchor[0]
      );
      if (i >= 0) {
        const handleLen = (pts[i + 1].anchor[0] - pts[i].anchor[0]) / 4;
        pts.splice(i + 1, 0, {
          anchor: value,
          inHandle: [value[0] - handleLen, value[1]],
          outHandle: [value[0] + handleLen, value[1]],
          outType: "bezier",
          handlesLinked: true,
        });
      }
    }
    clampHandles(pts);
    onCommitEdit(state);
    setState((s) => ({ ...s, bezierData: { points: pts } }));
  };

  const handleContextMenu = (e) => {
    e.preventDefault();
    if (state.mode !== "bezier") return;
    const r = ref.current.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    const h = hit(x, y);

    if (h && (h.kind === "anchor" || h.kind === "in" || h.kind === "out")) {
      setContextMenu({
        x: e.clientX,
        y: e.clientY,
        pointIndex: h.index,
      });
    } else {
      setContextMenu(null);
    }
  };

  useEffect(() => {
    const up = () => {
      const ended = drag.current;
      drag.current = null;
      if (ended?.startState && ended.kind !== "pan") {
        onCommitEdit(ended.startState);
      }
    };
    const closeMenu = () => setContextMenu(null);
    window.addEventListener("mouseup", up);
    window.addEventListener("click", closeMenu);
    return () => {
      window.removeEventListener("mouseup", up);
      window.removeEventListener("click", closeMenu);
    };
  }, [onCommitEdit]);

  return (
    <div className="graph-wrap" style={{ position: "relative" }}>
      {isModifiedView && (
        <button
          className="icon-button reset-view-btn"
          title="視点リセット"
          onClick={resetView}
        >
          <MdCenterFocusWeak />
        </button>
      )}
      <canvas
        ref={ref}
        tabIndex={-1}
        onMouseDown={onDown}
        onMouseMove={onMove}
        onDoubleClick={onDouble}
        onWheel={onWheel}
        onContextMenu={handleContextMenu}
      />
      {contextMenu && (
        <ClampedMenu
          className="context-menu"
          x={contextMenu.x}
          y={contextMenu.y}
          onClick={(e) => e.stopPropagation()}
        >
          {["bezier", "elastic", "bounce"].map((type) => (
            <button
              key={type}
              className={`context-menu-item ${state.bezierData.points[contextMenu.pointIndex]?.outType ===
                  type
                  ? "active"
                  : ""
                }`}
              onClick={() => {
                const pts = clone(state.bezierData.points);
                if (pts[contextMenu.pointIndex]) {
                  pts[contextMenu.pointIndex].outType = type;
                  if (
                    type === "elastic" &&
                    !pts[contextMenu.pointIndex].outElasticParams
                  ) {
                    pts[contextMenu.pointIndex].outElasticParams =
                      clone(DEFAULT_ELASTIC);
                  }
                  if (
                    type === "bounce" &&
                    !pts[contextMenu.pointIndex].outBounceParams
                  ) {
                    pts[contextMenu.pointIndex].outBounceParams =
                      clone(DEFAULT_BOUNCE);
                  }
                }
                onCommitEdit(state);
                setState((s) => ({ ...s, bezierData: { points: pts } }));
                setContextMenu(null);
              }}
            >
              {type === "bezier"
                ? "ベジェ"
                : type === "elastic"
                  ? "振動"
                  : "バウンス"}
            </button>
          ))}
          {contextMenu.pointIndex > 0 &&
            contextMenu.pointIndex < state.bezierData.points.length - 1 && (
              <>
                <div className="context-menu-divider" />
                <button
                  className="context-menu-item"
                  onClick={() => {
                    const pts = clone(state.bezierData.points);
                    const point = pts[contextMenu.pointIndex];
                    if (point) {
                      point.handlesLinked = !handlesAreLinked(point);
                      clampHandles(pts);
                    }
                    onCommitEdit(state);
                    setState((s) => ({ ...s, bezierData: { points: pts } }));
                    setContextMenu(null);
                  }}
                >
                  {state.bezierData.points[contextMenu.pointIndex] &&
                  handlesAreLinked(
                    state.bezierData.points[contextMenu.pointIndex]
                  ) ? (
                    <>
                      <MdLinkOff /> ハンドルを分離
                    </>
                  ) : (
                    <>
                      <MdLink /> ハンドルを統合
                    </>
                  )}
                </button>
                <button
                  className="context-menu-item danger"
                  onClick={() => {
                    const pts = clone(state.bezierData.points);
                    pts.splice(contextMenu.pointIndex, 1);
                    clampHandles(pts);
                    onCommitEdit(state);
                    setState((s) => ({ ...s, bezierData: { points: pts } }));
                    setContextMenu(null);
                  }}
                >
                  <MdDelete /> アンカーを削除
                </button>
              </>
            )}
        </ClampedMenu>
      )}
    </div>
  );
}

function Presets({ presets, setPresets, settings, style, onLoadPreset }) {
  const [dragged, setDragged] = useState(null);
  const [presetMenu, setPresetMenu] = useState(null);

  const presetThumbs = useMemo(() => {
    const thumbs = {};
    for (const preset of presets) {
      try {
        thumbs[preset.id] = makeThumb(
          preset.mode,
          preset.data,
          settings.theme
        );
      } catch {
        thumbs[preset.id] = "";
      }
    }
    return thumbs;
  }, [presets, settings.theme]);

  const handlePresetContextMenu = (e, id) => {
    e.preventDefault();
    e.stopPropagation();
    setPresetMenu({ x: e.clientX, y: e.clientY, id });
  };

  useEffect(() => {
    const close = () => setPresetMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, []);

  const boxSize = settings.presetBoxSize || 78;
  const cardStyle = {
    width: `${boxSize}px`,
    height: `${boxSize + 18}px`,
  };

  return (
    <aside className="preset-panel" style={style}>
      <div className="preset-list">
        {presets.map((p, i) => (
          <div
            key={p.id}
            className="preset-card"
            style={cardStyle}
            draggable
            onDragStart={() => setDragged(i)}
            onDragOver={(e) => e.preventDefault()}
            onDrop={() => {
              if (dragged == null || dragged === i) return;
              const next = [...presets];
              const [entry] = next.splice(dragged, 1);
              next.splice(i, 0, entry);
              setPresets(next);
              setDragged(null);
            }}
            onContextMenu={(e) => handlePresetContextMenu(e, p.id)}
            onClick={() => onLoadPreset(p.mode, p.data)}
          >
            <img
              src={presetThumbs[p.id] || p.thumbnail || ""}
              alt={p.name}
              style={{ height: `${boxSize - 4}px` }}
            />
            <span>{p.name}</span>
          </div>
        ))}
      </div>
      {presetMenu && (
        <ClampedMenu
          className="context-menu"
          x={presetMenu.x}
          y={presetMenu.y}
          onClick={(e) => e.stopPropagation()}
        >
          <button
            className="context-menu-item danger"
            onClick={() => {
              setPresets((p) => p.filter((item) => item.id !== presetMenu.id));
              setPresetMenu(null);
            }}
          >
            <MdDelete /> プリセットの削除
          </button>
        </ClampedMenu>
      )}
    </aside>
  );
}

function SegmentBar({ segments, onSelect }) {
  const frames = Array.isArray(segments.frames) ? segments.frames : [];
  const count = frames.length;
  if (count <= 1) return null;
  const focus = segments.focus ?? 0;
  const objectEnd = segments.layerFrame?.end;
  return (
    <div className="segment-bar" title="AviUtl2のシークバー位置の区間">
      <span className="segment-label">区間</span>
      <div className="segment-list">
        {Array.from({ length: count }, (_, index) => {
          const start = frames[index];
          const end = frames[index + 1] ?? objectEnd;
          return (
            <button
              key={index}
              className={`segment-chip ${index === focus ? "active" : ""}`}
              onClick={() => onSelect(index)}
              title={
                end == null
                  ? `${start} フレーム`
                  : `${start} - ${end} フレーム`
              }
            >
              {index + 1}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function AnimatedModal({ open, onClose, children, title }) {
  const [closing, setClosing] = useState(false);
  const [visible, setVisible] = useState(open);

  useEffect(() => {
    if (open) {
      setVisible(true);
      setClosing(false);
    } else if (visible) {
      setClosing(true);
      const timer = setTimeout(() => {
        setVisible(false);
        setClosing(false);
      }, 150);
      return () => clearTimeout(timer);
    }
  }, [open, visible]);

  if (!visible) return null;

  const handleClose = () => {
    setClosing(true);
    setTimeout(() => {
      onClose();
      setVisible(false);
      setClosing(false);
    }, 150);
  };

  return (
    <div
      className={`modal-backdrop ${closing ? "closing" : ""}`}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      <section className={`modal-dialog ${closing ? "closing" : ""}`}>
        <header>
          <strong>{title}</strong>
          <button className="close-button" onClick={handleClose}>
            <MdClose />
          </button>
        </header>
        {children}
      </section>
    </div>
  );
}

function SavePresetModal({ open, onClose, onSave, defaultName }) {
  const [name, setName] = useState(defaultName || "");
  useEffect(() => {
    setName(defaultName || "");
  }, [defaultName, open]);

  const handleSubmit = (e) => {
    e.preventDefault();
    if (name.trim()) {
      onSave(name.trim());
      onClose();
    }
  };

  return (
    <AnimatedModal open={open} onClose={onClose} title="プリセットの保存">
      <form onSubmit={handleSubmit} className="modal-content">
        <label>
          プリセット名
          <input
            type="text"
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="プリセット名を入力..."
          />
        </label>
        <footer className="modal-footer">
          <button type="button" onClick={onClose}>
            キャンセル
          </button>
          <button type="submit" className="primary">
            保存
          </button>
        </footer>
      </form>
    </AnimatedModal>
  );
}

function SettingsPanelModal({ open, onClose, settings, setSettings, onDebug }) {
  const update = (key, value) => setSettings((s) => ({ ...s, [key]: value }));
  return (
    <AnimatedModal open={open} onClose={onClose} title="設定">
      <div className="modal-content settings-content">
        <label className="settings-check">
          <input
            type="checkbox"
            checked={settings.dragDropEnabled !== false}
            onChange={(e) => update("dragDropEnabled", e.target.checked)}
          />
          ドラッグ&ドロップ適用を有効にする
        </label>
        <label className="settings-check">
          <input
            type="checkbox"
            checked={settings.groupApplyOnDrop === true}
            onChange={(e) => update("groupApplyOnDrop", e.target.checked)}
          />
          X/Y/Zなどのグループ項目へまとめて適用（D&D）
        </label>
        <label>
          プリセットパネル位置
          <select
            value={settings.presetPanelLocation}
            onChange={(e) => update("presetPanelLocation", e.target.value)}
          >
            <option value="right">右</option>
            <option value="left">左</option>
            <option value="top">上</option>
            <option value="bottom">下</option>
          </select>
        </label>
        <label>
          プリセットのボックスサイズ ({settings.presetBoxSize || 78}px)
          <input
            type="range"
            min="40"
            max="140"
            value={settings.presetBoxSize || 78}
            onChange={(e) => update("presetBoxSize", Number(e.target.value))}
          />
        </label>
        <label>
          グリッド分割数 X
          <input
            type="number"
            min="1"
            max="50"
            value={settings.gridLinesX}
            onChange={(e) =>
              update("gridLinesX", clamp(Number(e.target.value) || 4, 1, 50))
            }
          />
        </label>
        <label>
          グリッド分割数 Y
          <input
            type="number"
            min="1"
            max="50"
            value={settings.gridLinesY}
            onChange={(e) =>
              update("gridLinesY", clamp(Number(e.target.value) || 4, 1, 50))
            }
          />
        </label>
        <label>
          カーブ描画精度
          <input
            type="range"
            min="50"
            max="500"
            value={settings.precision}
            onChange={(e) => update("precision", Number(e.target.value))}
          />
        </label>
        <footer className="modal-footer">
          <button type="button" onClick={onDebug}>
            ドラッグ診断をログ出力
          </button>
          <button type="button" className="primary" onClick={onClose}>
            完了
          </button>
        </footer>
      </div>
    </AnimatedModal>
  );
}

function PresetDivider({
  vertical,
  before,
  visible,
  onToggle,
  onResizeStart,
}) {
  const outward = vertical ? (
    before ? <MdKeyboardArrowUp /> : <MdKeyboardArrowDown />
  ) : before ? (
    <MdChevronLeft />
  ) : (
    <MdChevronRight />
  );
  const inward = vertical ? (
    before ? <MdKeyboardArrowDown /> : <MdKeyboardArrowUp />
  ) : before ? (
    <MdChevronRight />
  ) : (
    <MdChevronLeft />
  );

  if (!visible) {
    const edgeClass = vertical
      ? before
        ? "edge-top"
        : "edge-bottom"
      : before
        ? "edge-left"
        : "edge-right";
    return (
      <button
        type="button"
        className={`panel-edge-toggle ${edgeClass}`}
        title="プリセット欄を表示"
        aria-label="プリセット欄を表示"
        onClick={onToggle}
      >
        {inward}
      </button>
    );
  }

  return (
    <div
      className={`resizer ${vertical ? "horizontal" : "vertical"}`}
      onMouseDown={onResizeStart}
    >
      <button
        type="button"
        className="panel-toggle"
        title="プリセット欄を隠す"
        aria-label="プリセット欄を隠す"
        onMouseDown={(event) => event.stopPropagation()}
        onClick={onToggle}
      >
        {outward}
      </button>
    </div>
  );
}

function App() {
  const [state, setState] = useState({
    mode: "bezier",
    bezierData: defaultBezier(),
    elasticData: clone(DEFAULT_ELASTIC),
    bounceData: clone(DEFAULT_BOUNCE),
  });
  const [settings, setSettings] = useState(() =>
    sanitizeSettings(readJson("hl_ezing.react.settings", {}))
  );
  const [curves, setCurves] = useState([]);
  const [activeId, setActiveId] = useState(null);
  const [notice, setNotice] = useState(null);

  const [presets, setPresets] = useState(() => {
    const value = readJson("hl_ezing.react.presets", []);
    return Array.isArray(value)
      ? value.filter(
          (preset) =>
            preset &&
            preset.id &&
            preset.mode &&
            isValidCurveData(preset.mode, preset.data)
        )
      : [];
  });
  const [showSavePreset, setShowSavePreset] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [panelSize, setPanelSize] = useState(settings.presetPanelSize || 200);
  const resizing = useRef(false);
  const applyWrapRef = useRef(null);
  const [pickerMaxHeight, setPickerMaxHeight] = useState(200);
  const [trackPicker, setTrackPicker] = useState(null);
  const [segments, setSegments] = useState({ frames: [], focus: null, layerFrame: null });
  const [draggingToTrackbar, setDraggingToTrackbar] = useState(false);
  const applyDragRef = useRef(null);
  const suppressApplyClickRef = useRef(false);
  const applyDragResetTimer = useRef(null);
  const segmentsTimer = useRef(null);

  const stateRef = useRef(state);
  stateRef.current = state;
  const curvesRef = useRef(curves);
  curvesRef.current = curves;
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;

  const noticeTimer = useRef(null);
  const bootstrappedRef = useRef(false);
  const lastSentRef = useRef(null);
  const pendingUpdateRef = useRef(null);
  const tracksRequestRef = useRef(null);
  const historyRef = useRef({ undo: [], redo: [] });

  const post = useCallback((message) => {
    window.ipc?.postMessage(JSON.stringify(message));
  }, []);

  const notify = useCallback((text, kind = "info") => {
    setNotice({ text, kind });
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
    noticeTimer.current = setTimeout(() => setNotice(null), 3200);
  }, []);

  const flushPendingUpdate = useCallback(() => {
    const pending = pendingUpdateRef.current;
    if (!pending) return;
    clearTimeout(pending.timer);
    pendingUpdateRef.current = null;
    lastSentRef.current = pending.payload;
    post(pending.message);
  }, [post]);

  const recordHistory = useCallback((beforeState, id = activeIdRef.current) => {
    if (id == null || !beforeState) return;
    const history = historyRef.current;
    history.undo.push({ id, state: clone(beforeState) });
    if (history.undo.length > 100) history.undo.shift();
    history.redo = [];
  }, []);

  const commitHistory = useCallback(
    (beforeState) => {
      if (activeIdRef.current == null || !beforeState) return;
      if (JSON.stringify(beforeState) === JSON.stringify(stateRef.current)) {
        return;
      }
      recordHistory(beforeState);
    },
    [recordHistory]
  );

  const applyHistoryEntry = useCallback(
    (entry) => {
      if (!entry || !curvesRef.current.some((curve) => curve.id === entry.id)) {
        return false;
      }
      flushPendingUpdate();
      if (activeIdRef.current != null) {
        const current = stateRef.current;
        const data = clone(curveDataFromState(current));
        setCurves((list) =>
          list.map((curve) =>
            curve.id === activeIdRef.current
              ? { ...curve, mode: current.mode, data }
              : curve
          )
        );
      }
      setActiveId(entry.id);
      setState(clone(entry.state));
      setCurves((list) =>
        list.map((curve) =>
          curve.id === entry.id
            ? {
                ...curve,
                mode: entry.state.mode,
                data: clone(curveDataFromState(entry.state)),
              }
            : curve
        )
      );
      lastSentRef.current = null;
      return true;
    },
    [flushPendingUpdate]
  );

  const undo = useCallback(() => {
    const history = historyRef.current;
    const entry = history.undo.pop();
    if (!entry) return;
    const current =
      activeIdRef.current == null
        ? null
        : { id: activeIdRef.current, state: clone(stateRef.current) };
    if (!applyHistoryEntry(entry)) {
      history.undo.push(entry);
      return;
    }
    if (current) {
      history.redo.push(current);
      if (history.redo.length > 100) history.redo.shift();
    }
  }, [applyHistoryEntry]);

  const redo = useCallback(() => {
    const history = historyRef.current;
    const entry = history.redo.pop();
    if (!entry) return;
    const current =
      activeIdRef.current == null
        ? null
        : { id: activeIdRef.current, state: clone(stateRef.current) };
    if (!applyHistoryEntry(entry)) {
      history.redo.push(entry);
      return;
    }
    if (current) {
      history.undo.push(current);
      if (history.undo.length > 100) history.undo.shift();
    }
  }, [applyHistoryEntry]);

  useEffect(() => {
    writeJson("hl_ezing.react.settings", settings);
  }, [settings]);
  useEffect(() => {
    writeJson("hl_ezing.react.presets", presets);
  }, [presets]);

  const selectCurve = useCallback((id) => {
    if (id === activeIdRef.current) return;
    flushPendingUpdate();
    const curve = curvesRef.current.find((entry) => entry.id === id);
    if (!curve) return;

    // 編集中のIDは一覧側のデータを遅延更新しているため、切り替え前に同期する。
    if (activeIdRef.current != null) {
      const current = stateRef.current;
      const data = clone(curveDataFromState(current));
      const payload = JSON.stringify({ mode: current.mode, data });
      // 80ms のデバウンス待ちで切り替え時に編集が失われないよう、
      // IDを切り替える直前に保留中の内容を即時送信する。
      if (lastSentRef.current !== payload) {
        lastSentRef.current = payload;
        post({
          cmd: "update_curve",
          id: activeIdRef.current,
          mode: current.mode,
          data,
        });
      }
      setCurves((list) =>
        list.map((entry) =>
          entry.id === activeIdRef.current
            ? {
                ...entry,
                mode: current.mode,
                data,
              }
            : entry
        )
      );
    }

    setActiveId(id);
    setState((current) => stateWithCurve(current, curve));
    lastSentRef.current = null;
  }, [flushPendingUpdate, post]);

  const selectOrCreateId = useCallback(
    (id) => {
      if (id === activeIdRef.current) return;
      const existing = curvesRef.current.find((curve) => curve.id === id);
      if (existing) {
        selectCurve(id);
        return;
      }
      // 入力されたIDが未作成の場合は、現在のグラフを複製して作成する。
      const current = stateRef.current;
      post({
        cmd: "create_curve",
        id,
        mode: current.mode,
        data: clone(curveDataFromState(current)),
      });
    },
    [post, selectCurve]
  );

  const loadPreset = useCallback(
    (mode, data) => {
      if (!isValidCurveData(mode, data)) {
        notify("プリセットのデータが不正です。", "error");
        return;
      }
      if (activeIdRef.current == null) {
        notify("先にカーブIDを入力してください。", "error");
        return;
      }
      const key = CURVE_DATA_KEY[mode] || CURVE_DATA_KEY.bezier;
      recordHistory(stateRef.current);
      setState((current) => ({
        ...current,
        mode: key === CURVE_DATA_KEY.bezier ? "bezier" : mode,
        [key]: clone(data),
      }));
    },
    [notify, recordHistory]
  );

  const handleEvent = useCallback(
    (event) => {
      if (!event || !event.event) return;

      if (event.event === "object_changed") {
        setTrackPicker(null);
        post({ cmd: "get_segments" });
        return;
      }

      if (event.event === "segments_dirty") {
        // AviUtl2 側でシークバーや区間が動いたときだけ届く。
        // 連続移動に備えて少しだけまとめてから取得する。
        if (segmentsTimer.current) clearTimeout(segmentsTimer.current);
        segmentsTimer.current = setTimeout(() => {
          segmentsTimer.current = null;
          post({ cmd: "get_segments" });
        }, 120);
        return;
      }

      if (event.event === "curves") {
        const list = Array.isArray(event.curves) ? event.curves : [];
        setCurves(list);
        if (list.length === 0) {
          setActiveId(null);
          if (!bootstrappedRef.current) {
            bootstrappedRef.current = true;
            post({ cmd: "create_curve", mode: "bezier", data: defaultBezier() });
          }
          return;
        }
        bootstrappedRef.current = true;
        const preferred =
          event.last_applied ?? activeIdRef.current ?? list[0].id;
        const next =
          list.find((curve) => curve.id === preferred) || list[0];
        setActiveId(next.id);
        setState((current) => stateWithCurve(current, next));
        lastSentRef.current = null;
        return;
      }

      if (event.event === "curve_created") {
        if (!event.ok || !event.curve) {
          notify(event.message || "カーブIDの作成に失敗しました。", "error");
          return;
        }
        const curve = event.curve;
        const previousId = activeIdRef.current;
        const previousState = stateRef.current;
        setCurves((list) =>
          [
            ...list.map((entry) =>
              entry.id === previousId
                ? {
                    ...entry,
                    mode: previousState.mode,
                    data: clone(curveDataFromState(previousState)),
                  }
                : entry
            ),
            curve,
          ]
            .filter(
              (entry, index, array) =>
                array.findIndex((candidate) => candidate.id === entry.id) ===
                index
            )
            .sort((a, b) => a.id - b.id)
        );
        setActiveId(curve.id);
        setState((current) => stateWithCurve(current, curve));
        lastSentRef.current = null;
        return;
      }

      if (event.event === "curve_updated") {
        // 編集中IDの更新は毎フレーム近く届くため、ID一覧の再描画を避ける。
        // 切り替え時に selectCurve() 側で最新データを同期する。
        if (event.ok && event.curve) {
          if (event.curve.id === activeIdRef.current) {
            // Rust 側で端点Yのclampや点数制限が行われた場合、
            // プレビューと適用結果がずれないようUIにも反映する。
            const current = stateRef.current;
            const localData = curveDataFromState(current);
            if (
              event.curve.mode !== current.mode ||
              JSON.stringify(event.curve.data) !== JSON.stringify(localData)
            ) {
              setState((currentState) =>
                stateWithCurve(currentState, event.curve)
              );
              lastSentRef.current = null;
            }
          } else {
            setCurves((list) =>
              list.map((entry) =>
                entry.id === event.curve.id ? event.curve : entry
              )
            );
          }
        }
        return;
      }

      if (event.event === "curve_deleted") {
        if (!event.ok) {
          notify(event.message || "カーブIDの削除に失敗しました。", "error");
          return;
        }
        const remaining = curvesRef.current.filter(
          (curve) => curve.id !== event.id
        );
        const history = historyRef.current;
        history.undo = history.undo.filter((entry) => entry.id !== event.id);
        history.redo = history.redo.filter((entry) => entry.id !== event.id);
            setCurves(remaining);
        if (activeIdRef.current === event.id) {
          const next = remaining[0];
          if (next) {
            setActiveId(next.id);
            setState((current) => stateWithCurve(current, next));
          } else {
            setActiveId(null);
          }
          lastSentRef.current = null;
        }
        notify(`ID ${event.id} を削除しました。`);
        return;
      }

      if (event.event === "drag_started") {
        if (!event.ok) {
          setDraggingToTrackbar(false);
          notify(event.message || "ドラッグを開始できませんでした。", "error");
        }
        return;
      }

      if (event.event === "drag_cancelled") {
        setDraggingToTrackbar(false);
        notify(event.message || "ドラッグをキャンセルしました。", "error");
        return;
      }

      if (event.event === "segments") {
        if (!event.ok) {
          setSegments({ frames: [], focus: null, layerFrame: null });
          return;
        }
        setSegments({
          frames: Array.isArray(event.frames) ? event.frames : [],
          focus: event.focus ?? null,
          layerFrame: event.layer_frame || null,
        });
        return;
      }

      if (event.event === "applied") {
        setDraggingToTrackbar(false);
        if (!event.ok) {
          notify(event.message || "適用に失敗しました。", "error");
          return;
        }
        const curve =
          event.curve || {
            id: event.id,
            mode: stateRef.current.mode,
            data: clone(curveDataFromState(stateRef.current)),
          };
        setCurves((list) =>
          [...list.filter((entry) => entry.id !== curve.id), curve].sort(
            (a, b) => a.id - b.id
          )
        );
        if (event.created) setActiveId(curve.id);
        notify(`ID ${event.id} を${event.count}件のトラックバーに適用しました。`);
        post({ cmd: "get_segments" });
        return;
      }

      if (event.event === "tracks") {
        window.dispatchEvent(
          new CustomEvent("hl-ezing-tracks", { detail: event })
        );
        return;
      }

      if (event.event === "projectLoaded") {
        bootstrappedRef.current = false;
        if (pendingUpdateRef.current) {
          clearTimeout(pendingUpdateRef.current.timer);
          pendingUpdateRef.current = null;
        }
        historyRef.current = { undo: [], redo: [] };
            lastSentRef.current = null;
        setTrackPicker(null);
        setSegments({ frames: [], focus: null, layerFrame: null });
        setCurves([]);
        setActiveId(null);
        post({ cmd: "get_curves" });
        post({ cmd: "get_segments" });
      }
    },
    [notify, post]
  );

  useEffect(() => {
    window.hlEzing = { onEvent: handleEvent, post };
    post({ cmd: "hello" });
    post({ cmd: "get_curves" });
    post({ cmd: "get_segments" });
  }, [handleEvent, post]);



  // エディタの変更をアクティブIDへ反映する。少し遅延させることで
  // ドラッグ中の連続IPCを抑えつつ、トラック側は即座に追従する。
  useEffect(() => {
    if (activeId == null) return;
    const data = curveDataFromState(state);
    const payload = JSON.stringify({ mode: state.mode, data });
    if (lastSentRef.current === payload) return;
    const message = {
      cmd: "update_curve",
      id: activeId,
      mode: state.mode,
      data: clone(data),
    };
    const timer = setTimeout(() => {
      pendingUpdateRef.current = null;
      lastSentRef.current = payload;
      post(message);
    }, 80);
    pendingUpdateRef.current = { timer, payload, message };
    return () => {
      if (pendingUpdateRef.current?.timer === timer) {
        clearTimeout(timer);
        pendingUpdateRef.current = null;
      }
    };
  }, [
    activeId,
    state.mode,
    state.bezierData,
    state.elasticData,
    state.bounceData,
    post,
  ]);

  useEffect(() => {
    const handleKeyDown = (e) => {
      if (showSettings || showSavePreset) return;

      if ((e.ctrlKey || e.metaKey) && !e.altKey && !isEditableTarget(e.target)) {
        const key = e.key.toLowerCase();
        if (key === "z" && !e.shiftKey) {
          e.preventDefault();
          undo();
          return;
        }
        if (key === "y" || (key === "z" && e.shiftKey)) {
          e.preventDefault();
          redo();
          return;
        }
      }

      if (
        e.code === "Space" &&
        !e.repeat &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.metaKey &&
        !e.shiftKey &&
        isSpaceShortcutTarget(e.target)
      ) {
        e.preventDefault();
        post({ cmd: "host_space" });
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [undo, redo, post, showSettings, showSavePreset]);

  const handleResizeMove = useCallback(
    (e) => {
      if (!resizing.current) return;
      const loc = settings.presetPanelLocation || "right";
      const vertical = loc === "top" || loc === "bottom";
      const before = loc === "left" || loc === "top";
      const size = vertical
        ? before
          ? e.clientY
          : window.innerHeight - e.clientY
        : before
          ? e.clientX
          : window.innerWidth - e.clientX;
      const clamped = clamp(
        size,
        60,
        vertical ? window.innerHeight - 80 : window.innerWidth - 120
      );
      setPanelSize(clamped);
    },
    [settings.presetPanelLocation]
  );

  useEffect(() => {
    const move = (e) => handleResizeMove(e);
    const up = () => {
      if (resizing.current) {
        resizing.current = false;
        setSettings((current) => ({
          ...current,
          presetPanelSize: panelSize,
        }));
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, [handleResizeMove, panelSize]);

  // トラックピッカーの高さをウィンドウ領域内に収める
  useEffect(() => {
    if (!trackPicker) return;
    const measure = () => {
      const el = applyWrapRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const available = rect.bottom - 38 - 6;
      setPickerMaxHeight(Math.max(40, Math.min(200, available)));
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [trackPicker]);

  // トラックピッカー外部クリックで閉じる
  useEffect(() => {
    if (!trackPicker) return;
    const close = (e) => {
      if (!e.target.closest(".apply-wrap")) setTrackPicker(null);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [trackPicker]);

  const setMode = (mode) => {
    if (mode === stateRef.current.mode) return;
    recordHistory(stateRef.current);
    setState((current) => ({ ...current, mode }));
  };

  const clearTracksRequest = useCallback(() => {
    if (!tracksRequestRef.current) return;
    window.removeEventListener(
      "hl-ezing-tracks",
      tracksRequestRef.current.listener
    );
    clearTimeout(tracksRequestRef.current.timer);
    tracksRequestRef.current = null;
  }, []);

  const doApply = (itemName, effectName, effectIndex) => {
    if (activeIdRef.current == null) {
      notify("先にカーブIDを作成してください。", "error");
      return;
    }
    const current = stateRef.current;
    const msg = {
      cmd: "apply",
      curve_id: activeIdRef.current,
      mode: current.mode,
      data: clone(curveDataFromState(current)),
    };
    if (itemName) msg.item_name = itemName;
    if (effectName) msg.effect_name = effectName;
    if (effectIndex != null) msg.effect_index = effectIndex;
    post(msg);
  };

  const handleApplyClick = () => {
    if (activeIdRef.current == null) {
      notify("先にカーブIDを作成してください。", "error");
      return;
    }
    clearTracksRequest();

    const onTracks = (e) => {
      clearTracksRequest();
      const detail = e.detail || {};
      if (!detail.ok) {
        notify(detail.message || "トラック一覧の取得に失敗しました。", "error");
        return;
      }
      const tracks = detail.tracks || [];
      if (tracks.length === 0) {
        notify("適用可能なトラックバーがありません。", "error");
        return;
      }
      if (tracks.length === 1) {
        doApply(tracks[0].item, tracks[0].effect, tracks[0].effect_index);
        setTrackPicker(null);
        return;
      }
      setTrackPicker({ tracks });
    };

    const timer = setTimeout(() => {
      clearTracksRequest();
      notify("トラック一覧の取得がタイムアウトしました。", "error");
    }, 3000);
    tracksRequestRef.current = { listener: onTracks, timer };
    window.addEventListener("hl-ezing-tracks", onTracks);
    post({ cmd: "get_tracks" });
  };

  // 「適用」ボタンのクリックは従来どおりピッカーを開き、数ピクセル以上
  // ドラッグした場合だけ AviUtl2 のトラックバーへのドロップを開始する。
  const beginApplyDrag = (e) => {
    if (
      e.button !== 0 ||
      activeIdRef.current == null ||
      settings.dragDropEnabled === false
    ) {
      return;
    }
    if (applyDragResetTimer.current) {
      clearTimeout(applyDragResetTimer.current);
      applyDragResetTimer.current = null;
    }
    suppressApplyClickRef.current = false;
    // ドラッグ開始前にスキャンを温めておくと、最初のハイライトまでの
    // 待ち時間を短縮できる。
    post({ cmd: "prepare_drag" });
    applyDragRef.current = {
      x: e.clientX,
      y: e.clientY,
      pointerId: e.pointerId,
      started: false,
    };
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // ポインタキャプチャ非対応でも、WebView内のドラッグは継続できる。
    }
  };

  const moveApplyDrag = (e) => {
    if (settings.dragDropEnabled === false) return;
    const drag = applyDragRef.current;
    if (!drag || drag.started || (e.buttons & 1) === 0) return;
    if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 6) return;
    drag.started = true;
    suppressApplyClickRef.current = true;
    setTrackPicker(null);
    setDraggingToTrackbar(true);
    post({
      cmd: "start_drag",
      curve_id: activeIdRef.current,
      group_apply: settings.groupApplyOnDrop === true,
    });
  };

  const endApplyDrag = (e) => {
    const drag = applyDragRef.current;
    applyDragRef.current = null;
    try {
      if (e.currentTarget.hasPointerCapture?.(e.pointerId)) {
        e.currentTarget.releasePointerCapture(e.pointerId);
      }
    } catch {
      // 既に解放済みの場合は無視する。
    }
    if (drag?.started) {
      // ドラッグ後の click を抑止する。WebView外で離した場合は click が
      // 発生しないため、少し後にフラグを戻す。
      if (applyDragResetTimer.current) {
        clearTimeout(applyDragResetTimer.current);
      }
      applyDragResetTimer.current = setTimeout(() => {
        suppressApplyClickRef.current = false;
        applyDragResetTimer.current = null;
      }, 400);
    } else {
      suppressApplyClickRef.current = false;
    }
  };

  const handleApplyButtonClick = () => {
    if (suppressApplyClickRef.current) {
      suppressApplyClickRef.current = false;
      return;
    }
    handleApplyClick();
  };

  useEffect(
    () => () => {
      if (applyDragResetTimer.current) {
        clearTimeout(applyDragResetTimer.current);
      }
      if (segmentsTimer.current) {
        clearTimeout(segmentsTimer.current);
      }
      if (pendingUpdateRef.current) {
        clearTimeout(pendingUpdateRef.current.timer);
        pendingUpdateRef.current = null;
      }
    },
    []
  );

  const handleSavePresetConfirm = (name) => {
    const current = stateRef.current;
    setPresets((list) => [
      ...list,
      {
        id: `${Date.now()}`,
        name,
        mode: current.mode,
        data: clone(curveDataFromState(current)),
        thumbnail: null,
      },
    ]);
  };

  const reset = () => {
    recordHistory(stateRef.current);
    setState((current) => ({
      ...current,
      bezierData: defaultBezier(),
      elasticData: clone(DEFAULT_ELASTIC),
      bounceData: clone(DEFAULT_BOUNCE),
    }));
  };

  const reverse = () => {
    recordHistory(stateRef.current);
    if (state.mode === "elastic") {
      setState((current) => ({
        ...current,
        elasticData: {
          ...current.elasticData,
          reversed: !current.elasticData.reversed,
        },
      }));
      return;
    }
    if (state.mode === "bounce") {
      setState((current) => ({
        ...current,
        bounceData: {
          ...current.bounceData,
          reversed: !current.bounceData.reversed,
        },
      }));
      return;
    }

    const original = clone(state.bezierData.points);
    const points = original
      .slice()
      .reverse()
      .map((point, index) => {
        const originalIndex = original.length - 1 - index;
        const incoming =
          originalIndex > 0 ? original[originalIndex - 1] : null;
        const outType = incoming?.outType || "bezier";
        const elastic = incoming?.outElasticParams
          ? clone(incoming.outElasticParams)
          : undefined;
        const bounce = incoming?.outBounceParams
          ? clone(incoming.outBounceParams)
          : undefined;
        if (elastic) elastic.reversed = !elastic.reversed;
        if (bounce) bounce.reversed = !bounce.reversed;
        return {
          ...point,
          anchor: [1 - point.anchor[0], 1 - point.anchor[1]],
          inHandle: [1 - point.outHandle[0], 1 - point.outHandle[1]],
          outHandle: [1 - point.inHandle[0], 1 - point.inHandle[1]],
          outType,
          outElasticParams: elastic,
          outBounceParams: bounce,
        };
      });
    clampHandles(points);
    setState((current) => ({ ...current, bezierData: { points } }));
  };

  const loc = settings.presetPanelLocation || "right";
  const isVertical = loc === "top" || loc === "bottom";
  const isBefore = loc === "left" || loc === "top";
  const showPresetPanel = settings.showPresetPanel !== false;

  const presetStyle = isVertical
    ? { height: `${panelSize}px`, flexBasis: `${panelSize}px` }
    : { width: `${panelSize}px`, flexBasis: `${panelSize}px` };

  const presetsComponent = (
    <Presets
      presets={presets}
      setPresets={setPresets}
      settings={settings}
      style={presetStyle}
      onLoadPreset={loadPreset}
    />
  );

  return (
    <div
      className={`app ${isVertical ? "vertical" : "horizontal"} compact location-${loc}`}
    >
      {showPresetPanel && isBefore && presetsComponent}
      {isBefore && (
        <PresetDivider
          vertical={isVertical}
          before
          visible={showPresetPanel}
          onToggle={() =>
            setSettings((current) => ({
              ...current,
              showPresetPanel: !current.showPresetPanel,
            }))
          }
          onResizeStart={() => {
            resizing.current = true;
          }}
        />
      )}
      <main className="main">
        <header className="floating-header">
          <select
            className="mode-select"
            value={state.mode}
            onChange={(e) => setMode(e.target.value)}
            title="イージングの種類"
          >
            <option value="bezier">ベジェ</option>
            <option value="elastic">振動</option>
            <option value="bounce">バウンス</option>
          </select>
          <IdInput value={activeId} onCommit={selectOrCreateId} />
          <button
            className="icon-button"
            title="設定"
            onClick={() => setShowSettings(true)}
          >
            <MdSettings />
          </button>
        </header>
        <Graph
          state={state}
          setState={setState}
          settings={settings}
          onCommitEdit={commitHistory}
        />
        <SegmentBar
          segments={segments}
          onSelect={(index) => {
            setSegments((current) => ({ ...current, focus: index }));
            post({ cmd: "select_segment", index });
          }}
        />
        <footer className="floating-footer">
          <div className="apply-wrap" ref={applyWrapRef}>
            <button
              className={`apply-btn ${draggingToTrackbar ? "dragging" : ""}`}
              onClick={handleApplyButtonClick}
              onPointerDown={beginApplyDrag}
              onPointerMove={moveApplyDrag}
              onPointerUp={endApplyDrag}
              onPointerCancel={endApplyDrag}
              disabled={activeId == null}
              title={
                activeId == null
                  ? "カーブIDを作成してください"
                  : settings.dragDropEnabled === false
                    ? "クリックで適用先を選択（ドラッグ&ドロップは設定で無効）"
                    : "クリックで適用先を選択 / ドラッグでトラックバーへ適用"
              }
            >
              <MdPlayArrow /> {draggingToTrackbar ? "ドロップ" : "適用"}
            </button>
            {trackPicker && (
              <div
                className="track-picker"
                style={{ maxHeight: `${pickerMaxHeight}px` }}
                onMouseDown={(e) => e.stopPropagation()}
              >
                {trackPicker.tracks.map((t, i) => (
                  <button
                    key={i}
                    className="track-picker-item"
                    onMouseDown={(e) => e.stopPropagation()}
                    onClick={(e) => {
                      e.stopPropagation();
                      doApply(t.item, t.effect, t.effect_index);
                      if (!e.shiftKey) setTrackPicker(null);
                    }}
                  >
                    {t.label}
                    {t.curve_id != null ? ` [ID ${t.curve_id}]` : ""}
                  </button>
                ))}
                <div className="context-menu-divider" />
                <button
                  className="track-picker-item"
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    doApply(null);
                    if (!e.shiftKey) setTrackPicker(null);
                  }}
                >
                  全てに適用
                </button>
              </div>
            )}
          </div>
          <button className="icon-button" onClick={reset} title="リセット">
            <MdRefresh />
          </button>
          <button className="icon-button" onClick={reverse} title="反転">
            <MdSwapVert />
          </button>
          <button
            className="icon-button"
            onClick={() => setShowSavePreset(true)}
            title="プリセット保存"
          >
            <MdBookmarkAdd />
          </button>
        </footer>
      </main>
      {!isBefore && (
        <PresetDivider
          vertical={isVertical}
          before={false}
          visible={showPresetPanel}
          onToggle={() =>
            setSettings((current) => ({
              ...current,
              showPresetPanel: !current.showPresetPanel,
            }))
          }
          onResizeStart={() => {
            resizing.current = true;
          }}
        />
      )}
      {showPresetPanel && !isBefore && presetsComponent}
      {notice && (
        <div className={`status-toast ${notice.kind}`}>{notice.text}</div>
      )}
      <SavePresetModal
        open={showSavePreset}
        onClose={() => setShowSavePreset(false)}
        onSave={handleSavePresetConfirm}
        defaultName={`プリセット ${presets.length + 1}`}
      />
      <SettingsPanelModal
        open={showSettings}
        onClose={() => setShowSettings(false)}
        settings={settings}
        setSettings={setSettings}
        onDebug={() => {
          post({ cmd: "debug_trackbars" });
          notify("トラックバー診断情報をログへ出力しました。");
        }}
      />
    </div>
  );
}


createRoot(document.getElementById("root")).render(<App />);
