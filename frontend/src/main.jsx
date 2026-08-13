import React, { useEffect, useRef, useState, useCallback, useLayoutEffect } from "react";
import { createRoot } from "react-dom/client";
import {
  MdCenterFocusWeak,
  MdClose,
  MdRefresh,
  MdSwapVert,
  MdPlayArrow,
  MdBookmarkAdd,
  MdDelete,
  MdSettings,
} from "react-icons/md";
import "./style.css";

const DEFAULT_ELASTIC = { amplitude: 0.5, frequency: 2, decay: 5, reversed: false };
const DEFAULT_BOUNCE = { cor: 0.5, period: 0.5, reversed: false };
const clone = (v) => JSON.parse(JSON.stringify(v));
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));

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
      },
      {
        anchor: [1, 1],
        inHandle: [0.67, 1],
        outHandle: [1, 1],
        outType: "bezier",
      },
    ],
  };
}

function elastic(p, a, f, d, reversed) {
  if (p <= 0) return 0;
  if (p >= 1) return 1;
  if (reversed) p = 1 - p;
  f = Math.max(0.001, f);
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
  const c = clamp(cor, 0, 0.999999);
  const d = Math.max(period, 0.0001);
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
    if (Math.abs(e) < 1e-6) break;
    const d =
      (3 * c1 - 6 * c2 + 3) * t * t + (-6 * c1 + 6 * c2) * t + 3 * c1;
    if (Math.abs(d) < 1e-6) break;
    t -= e / d;
  }
  return clamp(t, 0, 1);
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
  for (let i = 0; i < points.length; i++) {
    const p = points[i];
    const prevX = i > 0 ? points[i - 1].anchor[0] : 0;
    const nextX = i < points.length - 1 ? points[i + 1].anchor[0] : 1;
    if (p.inHandle) {
      p.inHandle[0] = clamp(p.inHandle[0], prevX, p.anchor[0]);
    }
    if (p.outHandle) {
      p.outHandle[0] = clamp(p.outHandle[0], p.anchor[0], nextX);
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

function Graph({ state, setState, settings }) {
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

  const resize = useCallback(() => {
    const c = ref.current;
    if (!c) return;
    const r = c.parentElement.getBoundingClientRect();
    c.width = Math.max(1, Math.floor(r.width));
    c.height = Math.max(1, Math.floor(r.height));
    draw();
  }, [draw]);

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
        // Altを押しながらのドラッグでも、ハンドル・アンカーを掴んだら
        // そちらを優先して操作する(Altはハンドルの個別独立操作用)
        let originalHandles = null;
        if (state.mode === "bezier") {
          originalHandles = state.bezierData.points.map((p) => ({
            inX: p.inHandle ? p.inHandle[0] - p.anchor[0] : 0,
            inY: p.inHandle ? p.inHandle[1] - p.anchor[1] : 0,
            outX: p.outHandle ? p.outHandle[0] - p.anchor[0] : 0,
            outY: p.outHandle ? p.outHandle[1] - p.anchor[1] : 0,
          }));
        }
        drag.current = { ...h, originalHandles };
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
      p[drag.current.kind === "in" ? "inHandle" : "outHandle"] = value;
      if (!keys.current.alt) {
        const key = drag.current.kind === "in" ? "inHandle" : "outHandle";
        const other = key === "inHandle" ? "outHandle" : "inHandle";
        const dx = value[0] - p.anchor[0];
        const dy = value[1] - p.anchor[1];
        const len =
          Math.hypot(
            p[other][0] - p.anchor[0],
            p[other][1] - p.anchor[1]
          ) || 0.0001;
        const n = Math.hypot(dx, dy) || 1;
        p[other] = [
          p.anchor[0] - (dx / n) * len,
          p.anchor[1] - (dy / n) * len,
        ];
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
        });
      }
    }
    clampHandles(pts);
    setState((s) => ({ ...s, bezierData: { points: pts } }));
  };

  const handleContextMenu = (e) => {
    e.preventDefault();
    if (state.mode !== "bezier") return;
    const r = ref.current.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    const h = hit(x, y);

    if (h && h.kind === "anchor") {
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
      drag.current = null;
    };
    const closeMenu = () => setContextMenu(null);
    window.addEventListener("mouseup", up);
    window.addEventListener("click", closeMenu);
    return () => {
      window.removeEventListener("mouseup", up);
      window.removeEventListener("click", closeMenu);
    };
  }, []);

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
                  className="context-menu-item danger"
                  onClick={() => {
                    const pts = clone(state.bezierData.points);
                    pts.splice(contextMenu.pointIndex, 1);
                    clampHandles(pts);
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

function Presets({ presets, setPresets, setState, settings, style }) {
  const [dragged, setDragged] = useState(null);
  const [presetMenu, setPresetMenu] = useState(null);

  // プリセットサムネイルグラフの大きさを 0.8倍に縮小描画
  const makeThumb = (preset) => {
    const c = document.createElement("canvas");
    c.width = 90;
    c.height = 90;
    const x = c.getContext("2d");
    x.fillStyle = settings.theme.bgColor;
    x.fillRect(0, 0, 90, 90);
    x.strokeStyle = settings.theme.curveColor;
    x.lineWidth = 2;
    x.beginPath();

    const margin = 0.1;
    const scale = 0.8;

    for (let i = 0; i <= 40; i++) {
      const t = i / 40;
      const y =
        preset.mode === "bezier"
          ? evalBezier(preset.data, t)
          : preset.mode === "elastic"
            ? elastic(
              t,
              preset.data.amplitude,
              preset.data.frequency,
              preset.data.decay,
              preset.data.reversed
            )
            : bounce(
              t,
              preset.data.cor,
              preset.data.period,
              preset.data.reversed
            );
      const px = (margin + t * scale) * 90;
      const py = 90 - (margin + y * scale) * 90;
      if (!i) x.moveTo(px, py);
      else x.lineTo(px, py);
    }
    x.stroke();
    return c.toDataURL();
  };

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
            onClick={() =>
              setState((s) => ({
                ...s,
                mode: p.mode,
                ...(p.mode === "bezier"
                  ? { bezierData: clone(p.data) }
                  : p.mode === "elastic"
                    ? { elasticData: clone(p.data) }
                    : { bounceData: clone(p.data) }),
              }))
            }
          >
            <img
              src={p.thumbnail || makeThumb(p)}
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

function SettingsPanelModal({ open, onClose, settings, setSettings }) {
  const update = (key, value) => setSettings((s) => ({ ...s, [key]: value }));
  return (
    <AnimatedModal open={open} onClose={onClose} title="設定">
      <div className="modal-content settings-content">
        <label>
          プリセットパネル位置
          <select
            value={settings.presetPanelLocation}
            onChange={(e) => update("presetPanelLocation", e.target.value)}
          >
            <option value="right">Right (右)</option>
            <option value="left">Left (左)</option>
            <option value="top">Top (上)</option>
            <option value="bottom">Bottom (下)</option>
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
            onChange={(e) => update("gridLinesX", Number(e.target.value))}
          />
        </label>
        <label>
          グリッド分割数 Y
          <input
            type="number"
            min="1"
            max="50"
            value={settings.gridLinesY}
            onChange={(e) => update("gridLinesY", Number(e.target.value))}
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
          <button type="button" className="primary" onClick={onClose}>
            完了
          </button>
        </footer>
      </div>
    </AnimatedModal>
  );
}

function App() {
  const [state, setState] = useState({
    mode: "bezier",
    bezierData: defaultBezier(),
    elasticData: clone(DEFAULT_ELASTIC),
    bounceData: clone(DEFAULT_BOUNCE),
  });
  const [settings, setSettings] = useState(() => {
    const defaults = {
      presetPanelLocation: "right",
      presetPanelSize: 200,
      presetBoxSize: 78,
      compactMode: true,
      gridLinesX: 4,
      gridLinesY: 4,
      precision: 150,
      theme: {
        bgColor: "#1d1d1d",
        gridColor: "#282828",
        mainGridColor: "#3d3d3d",
        curveColor: "#3b82f6",
        handleColor: "#888888",
        anchorColor: "#ffffff",
      },
    };
    const saved = JSON.parse(localStorage.getItem("hl_ezing.react.settings") || "{}");
    const merged = Object.assign({}, defaults, saved);
    merged.theme = Object.assign({}, defaults.theme, saved.theme, { curveColor: "#3b82f6" });
    return merged;
  });

  const [presets, setPresets] = useState(() =>
    JSON.parse(localStorage.getItem("hl_ezing.react.presets") || "[]")
  );
  const [showSavePreset, setShowSavePreset] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [panelSize, setPanelSize] = useState(settings.presetPanelSize || 200);
  const resizing = useRef(false);
  const applyWrapRef = useRef(null);
  const [pickerMaxHeight, setPickerMaxHeight] = useState(200);

  useEffect(() => {
    localStorage.setItem("hl_ezing.react.settings", JSON.stringify(settings));
  }, [settings]);
  useEffect(() => {
    localStorage.setItem("hl_ezing.react.presets", JSON.stringify(presets));
  }, [presets]);

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
        setSettings((s) => ({ ...s, presetPanelSize: panelSize }));
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, [handleResizeMove, panelSize]);

  const [trackPicker, setTrackPicker] = useState(null); // { tracks: [...] }

  // トラックピッカーの高さをウィンドウ領域内に収める
  useEffect(() => {
    if (!trackPicker) return;
    const measure = () => {
      const el = applyWrapRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const available = rect.bottom - 38 - 6; // ピッカー下端はボタン上38px、上端余白6px
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

  useEffect(() => {
    window.hlEzing = {
      onEvent: (event) => {
        if (event.event === "applied")
          window.dispatchEvent(new CustomEvent("hl-ezing-applied", { detail: event }));
        if (event.event === "tracks") {
          window.dispatchEvent(new CustomEvent("hl-ezing-tracks", { detail: event }));
        }
      },
      post: (message) => window.ipc?.postMessage(JSON.stringify(message)),
    };
    window.hlEzing.post({ cmd: "hello" });
  }, []);

  const setMode = (mode) => setState((s) => ({ ...s, mode }));

  const handleApplyClick = () => {
    // トラック一覧を取得してドロップダウンを表示
    const onTracks = (e) => {
      window.removeEventListener("hl-ezing-tracks", onTracks);
      const tracks = e.detail?.tracks || [];
      if (tracks.length === 0) {
        // トラックが無い場合はそのまま適用
        doApply(null);
      } else if (tracks.length === 1) {
        // 1つしかない場合はそのまま適用
        doApply(tracks[0].item);
      } else {
        // 複数ある場合はピッカーを表示
        setTrackPicker({ tracks });
      }
    };
    window.addEventListener("hl-ezing-tracks", onTracks);
    window.ipc?.postMessage(JSON.stringify({ cmd: "get_tracks" }));
  };

  const doApply = (itemName) => {
    const data =
      state.mode === "bezier"
        ? state.bezierData
        : state.mode === "elastic"
          ? state.elasticData
          : state.bounceData;
    const msg = { cmd: "apply", mode: state.mode, data: clone(data) };
    if (itemName) msg.item_name = itemName;
    window.ipc?.postMessage(JSON.stringify(msg));
    setTrackPicker(null);
  };

  const handleSavePresetConfirm = (name) => {
    const data =
      state.mode === "bezier"
        ? state.bezierData
        : state.mode === "elastic"
          ? state.elasticData
          : state.bounceData;
    setPresets((p) => [
      ...p,
      {
        id: `${Date.now()}`,
        name,
        mode: state.mode,
        data: clone(data),
        thumbnail: null,
      },
    ]);
  };

  const reset = () =>
    setState((s) => ({
      ...s,
      bezierData: defaultBezier(),
      elasticData: clone(DEFAULT_ELASTIC),
      bounceData: clone(DEFAULT_BOUNCE),
    }));

  const reverse = () => {
    if (state.mode === "elastic")
      setState((s) => ({
        ...s,
        elasticData: {
          ...s.elasticData,
          reversed: !s.elasticData.reversed,
        },
      }));
    else if (state.mode === "bounce")
      setState((s) => ({
        ...s,
        bounceData: {
          ...s.bounceData,
          reversed: !s.bounceData.reversed,
        },
      }));
    else {
      const points = clone(state.bezierData.points)
        .reverse()
        .map((p) => ({
          ...p,
          anchor: [1 - p.anchor[0], 1 - p.anchor[1]],
          inHandle: [1 - p.outHandle[0], 1 - p.outHandle[1]],
          outHandle: [1 - p.inHandle[0], 1 - p.inHandle[1]],
        }));
      clampHandles(points);
      setState((s) => ({ ...s, bezierData: { points } }));
    }
  };

  const loc = settings.presetPanelLocation || "right";
  const isVertical = loc === "top" || loc === "bottom";
  const isBefore = loc === "left" || loc === "top";

  const presetStyle = isVertical
    ? { height: `${panelSize}px`, flexBasis: `${panelSize}px` }
    : { width: `${panelSize}px`, flexBasis: `${panelSize}px` };

  const presetsComponent = (
    <Presets
      presets={presets}
      setPresets={setPresets}
      setState={setState}
      settings={settings}
      style={presetStyle}
    />
  );

  return (
    <div
      className={`app ${isVertical ? "vertical" : "horizontal"} compact location-${loc}`}
    >
      {isBefore && presetsComponent}
      {isBefore && (
        <div
          className={`resizer ${isVertical ? "horizontal" : "vertical"}`}
          onMouseDown={() => {
            resizing.current = true;
          }}
        />
      )}
      <main className="main">
        <header className="floating-header">
          <div className="modes">
            {[
              ["bezier", "ベジェ"],
              ["elastic", "振動"],
              ["bounce", "バウンス"],
            ].map(([mode, label]) => (
              <button
                className={state.mode === mode ? "active" : ""}
                onClick={() => setMode(mode)}
                key={mode}
              >
                {label}
              </button>
            ))}
          </div>
          <button
            className="icon-button"
            title="設定"
            onClick={() => setShowSettings(true)}
          >
            <MdSettings />
          </button>
        </header>
        <Graph state={state} setState={setState} settings={settings} />
        <footer className="floating-footer">
          <div className="apply-wrap" ref={applyWrapRef}>
            <button
              className="apply-btn"
              onClick={handleApplyClick}
              title="適用先プロパティを選択"
            >
              <MdPlayArrow /> 適用
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
                      doApply(t.item);
                      if (!e.shiftKey) setTrackPicker(null);
                    }}
                  >
                    {t.label}
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
        <div
          className={`resizer ${isVertical ? "horizontal" : "vertical"}`}
          onMouseDown={() => {
            resizing.current = true;
          }}
        />
      )}
      {!isBefore && presetsComponent}
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
      />
    </div>
  );
}

createRoot(document.getElementById("root")).render(<App />);
