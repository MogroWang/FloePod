/**
 * 可中断弹簧动画（Apple「Designing Fluid Interfaces」参数模型）。
 * - response：到达目标的时间感（秒），越小越快
 * - damping：1.0 = 临界阻尼（无过冲）；< 1.0 有回弹（仅用于带惯性的手势）
 * 每次 setTarget 都从「当前值与当前速度」继续，天然可中断、可反向。
 */

export interface SpringHandle {
  setTarget: (to: number) => void;
  stop: () => void;
}

export interface SpringConfig {
  response?: number;
  damping?: number;
  initialVelocity?: number;
}

export function springValue(
  from: number,
  target: number,
  onUpdate: (value: number) => void,
  config: SpringConfig = {},
  onComplete?: () => void,
): SpringHandle {
  const response = config.response ?? 0.35;
  const dampingRatio = config.damping ?? 1;
  const omega = (2 * Math.PI) / response; // 固有角频率
  const k = omega * omega;
  const c = 2 * dampingRatio * omega;

  let x = from;
  let v = config.initialVelocity ?? 0;
  let to = target;
  let raf = 0;
  let last = performance.now();
  let done = false;

  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (reduced) {
    onUpdate(target);
    onComplete?.();
    return { setTarget: (t) => onUpdate(t), stop: () => undefined };
  }

  const tick = (now: number) => {
    const dt = Math.min((now - last) / 1000, 1 / 30);
    last = now;
    // 半隐式欧拉：先更新速度再更新位置，稳定且实现简单
    const a = k * (to - x) - c * v;
    v += a * dt;
    x += v * dt;
    if (Math.abs(to - x) < 0.05 && Math.abs(v) < 0.5) {
      x = to;
      v = 0;
      onUpdate(x);
      done = true;
      // requestAnimationFrame ids are only handles, not a reliable "is running"
      // flag once the callback has fired. Reset it here so a later setTarget can
      // start a fresh animation after this spring has settled.
      raf = 0;
      onComplete?.();
      return;
    }
    onUpdate(x);
    raf = requestAnimationFrame(tick);
  };

  const start = () => {
    if (raf === 0 && !done) {
      last = performance.now();
      raf = requestAnimationFrame(tick);
    }
  };

  raf = requestAnimationFrame(tick);

  return {
    setTarget(t: number) {
      to = t;
      done = false;
      start();
    },
    stop() {
      if (raf !== 0) cancelAnimationFrame(raf);
      raf = 0;
    },
  };
}
