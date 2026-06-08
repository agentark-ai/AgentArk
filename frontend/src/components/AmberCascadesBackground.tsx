import { useEffect, useRef } from "react";

type Fragment = {
  x: number;
  y: number;
  vx: number;
  vy: number;
  size: number;
  alpha: number;
  label: string;
  phase: number;
  rotation: number;
  rotationSpeed: number;
  drift: number;
  nextSwap: number;
};

type SignalNode = {
  x: number;
  y: number;
  vx: number;
  vy: number;
  radius: number;
  alpha: number;
  phase: number;
};

type Pulse = {
  x: number;
  y: number;
  radius: number;
  life: number;
};

const ALPHANUMERICS = "ABCDEFGHJKLMNPQRSTUVWXYZ0123456789";
const TARGET_FRAME_MS = 1000 / 24;
const IDLE_PAUSE_MS = 2 * 60 * 1000;

function rand(min: number, max: number) {
  return min + Math.random() * (max - min);
}

function randomChar() {
  return ALPHANUMERICS[Math.floor(Math.random() * ALPHANUMERICS.length)] || "A";
}

function randomLabel() {
  const length = Math.random() < 0.7 ? 1 : Math.random() < 0.86 ? 2 : 3;
  return Array.from({ length }, randomChar).join("");
}

function wrap(value: number, min: number, max: number) {
  if (value < min) return max;
  if (value > max) return min;
  return value;
}

function initAmberCascades(canvas: HTMLCanvasElement) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return () => {};

  let width = 0;
  let height = 0;
  let dpr = 1;
  let raf = 0;
  let frameTimer = 0;
  let idleTimer = 0;
  let lastTime = 0;
  let stopped = false;
  let idle = false;
  let fragments: Fragment[] = [];
  let nodes: SignalNode[] = [];
  let pulses: Pulse[] = [];
  const reduced =
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

  const createFragment = (): Fragment => {
    const depth = rand(0.65, 1.35);
    return {
      x: Math.random() * width,
      y: Math.random() * height,
      vx: rand(-7, 7) * depth,
      vy: rand(-5, 5) * depth,
      size: rand(8, 15) * depth,
      alpha: rand(0.035, 0.11),
      label: randomLabel(),
      phase: Math.random() * Math.PI * 2,
      rotation: rand(-0.22, 0.22),
      rotationSpeed: rand(-0.035, 0.035),
      drift: rand(4, 16),
      nextSwap: rand(0.8, 2.8),
    };
  };

  const createNode = (): SignalNode => ({
    x: Math.random() * width,
    y: Math.random() * height,
    vx: rand(-4, 4),
    vy: rand(-3, 3),
    radius: rand(0.7, 1.8),
    alpha: rand(0.08, 0.2),
    phase: Math.random() * Math.PI * 2,
  });

  const resize = () => {
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    width = window.innerWidth;
    height = window.innerHeight;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const area = width * height;
    const fragmentCount = Math.max(48, Math.min(145, Math.round(area / 18000)));
    const nodeCount = Math.max(18, Math.min(42, Math.round(area / 52000)));
    fragments = Array.from({ length: fragmentCount }, createFragment);
    nodes = Array.from({ length: nodeCount }, createNode);
    if (reduced || document.visibilityState === "hidden") {
      drawStatic();
    }
  };

  const disturb = (x: number, y: number) => {
    pulses.push({ x, y, radius: 0, life: 1 });
    if (pulses.length > 8) pulses.shift();

    for (const fragment of fragments) {
      const dx = fragment.x - x;
      const dy = fragment.y - y;
      const distance = Math.hypot(dx, dy);
      if (distance > 210 || distance < 0.1) continue;
      const force = (1 - distance / 210) * 28;
      fragment.vx += (dx / distance) * force;
      fragment.vy += (dy / distance) * force;
      fragment.alpha = Math.min(0.16, fragment.alpha + 0.025);
    }
  };

  const drawBackground = () => {
    ctx.clearRect(0, 0, width, height);
    ctx.fillStyle = "#0a0a0a";
    ctx.fillRect(0, 0, width, height);
  };

  const update = (dt: number, timeMs: number) => {
    const t = timeMs * 0.001;

    for (const node of nodes) {
      node.x = wrap(node.x + (node.vx + Math.sin(t * 0.18 + node.phase) * 0.7) * dt, -24, width + 24);
      node.y = wrap(node.y + (node.vy + Math.cos(t * 0.16 + node.phase) * 0.5) * dt, -24, height + 24);
    }

    for (const fragment of fragments) {
      const driftX = Math.sin(t * 0.22 + fragment.phase) * fragment.drift;
      const driftY = Math.cos(t * 0.2 + fragment.phase) * fragment.drift;
      fragment.x = wrap(fragment.x + (fragment.vx + driftX * 0.08) * dt, -60, width + 60);
      fragment.y = wrap(fragment.y + (fragment.vy + driftY * 0.06) * dt, -40, height + 40);
      fragment.rotation += fragment.rotationSpeed * dt;
      fragment.vx *= 0.992;
      fragment.vy *= 0.992;
      fragment.nextSwap -= dt;
      if (fragment.nextSwap <= 0) {
        fragment.label = randomLabel();
        fragment.nextSwap = rand(1.2, 4.4);
      }
    }

    for (let i = pulses.length - 1; i >= 0; i -= 1) {
      const pulse = pulses[i];
      pulse.radius += 72 * dt;
      pulse.life -= 0.78 * dt;
      if (pulse.life <= 0) pulses.splice(i, 1);
    }
  };

  const drawLinks = () => {
    for (let i = 0; i < nodes.length; i += 1) {
      for (let j = i + 1; j < nodes.length; j += 1) {
        const a = nodes[i];
        const b = nodes[j];
        const distance = Math.hypot(a.x - b.x, a.y - b.y);
        if (distance > 180) continue;
        const alpha = (1 - distance / 180) * 0.09 * Math.min(a.alpha, b.alpha);
        ctx.beginPath();
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
        ctx.strokeStyle = `rgba(207, 157, 106, ${alpha})`;
        ctx.lineWidth = 1;
        ctx.stroke();
      }
    }

    for (const node of nodes) {
      ctx.beginPath();
      ctx.arc(node.x, node.y, node.radius, 0, Math.PI * 2);
      ctx.fillStyle = `rgba(230, 184, 124, ${node.alpha})`;
      ctx.fill();
    }
  };

  const drawFragments = (timeMs: number) => {
    const t = timeMs * 0.001;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    for (const fragment of fragments) {
      const bob = Math.sin(t * 0.45 + fragment.phase) * 3;
      const alpha = fragment.alpha * (0.7 + Math.sin(t * 0.35 + fragment.phase) * 0.3);
      ctx.save();
      ctx.translate(fragment.x, fragment.y + bob);
      ctx.rotate(fragment.rotation + Math.sin(t * 0.12 + fragment.phase) * 0.08);
      ctx.font = `500 ${fragment.size}px "IBM Plex Sans", "Inter", "Segoe UI", sans-serif`;
      // No per-glyph shadowBlur: it was the most expensive 2D op in the loop
      // (a blur per fragment per frame) for a glow that is imperceptible at
      // these alphas. The warm tint stays via the fill color.
      ctx.fillStyle = `rgba(230, 214, 190, ${alpha})`;
      ctx.fillText(fragment.label, 0, 0);
      ctx.restore();
    }
  };

  const drawPulses = () => {
    for (const pulse of pulses) {
      ctx.beginPath();
      ctx.arc(pulse.x, pulse.y, pulse.radius, 0, Math.PI * 2);
      ctx.strokeStyle = `rgba(216, 173, 120, ${0.18 * pulse.life})`;
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  };

  const cancelScheduledFrame = () => {
    if (raf) {
      window.cancelAnimationFrame(raf);
      raf = 0;
    }
    if (frameTimer) {
      window.clearTimeout(frameTimer);
      frameTimer = 0;
    }
  };

  const clearIdleTimer = () => {
    if (idleTimer) {
      window.clearTimeout(idleTimer);
      idleTimer = 0;
    }
  };

  const renderFrame = (timeMs: number, animate: boolean) => {
    const dt = Math.min((timeMs - (lastTime || timeMs)) / 1000, 0.05);
    lastTime = timeMs;

    drawBackground();
    if (animate) update(dt, timeMs);
    drawLinks();
    drawFragments(animate ? timeMs : 0);
    drawPulses();
  };

  const scheduleFrame = () => {
    if (stopped || idle || reduced || document.visibilityState === "hidden") return;
    if (raf || frameTimer) return;
    frameTimer = window.setTimeout(() => {
      frameTimer = 0;
      raf = window.requestAnimationFrame((timeMs) => {
        raf = 0;
        renderFrame(timeMs, true);
        scheduleFrame();
      });
    }, TARGET_FRAME_MS);
  };

  const drawStatic = () => {
    lastTime = 0;
    renderFrame(0, false);
  };

  const onPointer = (event: PointerEvent) => {
    markActive();
    if (reduced || document.visibilityState === "hidden") return;
    disturb(event.clientX, event.clientY);
  };

  const markIdle = () => {
    if (stopped || idle) return;
    idle = true;
    cancelScheduledFrame();
    drawStatic();
  };

  const markActive = () => {
    if (stopped || document.visibilityState === "hidden") return;
    const wasIdle = idle;
    idle = false;
    clearIdleTimer();
    idleTimer = window.setTimeout(markIdle, IDLE_PAUSE_MS);
    if (wasIdle && !reduced) {
      lastTime = 0;
      scheduleFrame();
    }
  };

  const onVisibilityChange = () => {
    cancelScheduledFrame();
    if (document.visibilityState === "hidden") {
      clearIdleTimer();
      idle = true;
      return;
    }
    markActive();
    lastTime = 0;
    if (reduced) {
      drawStatic();
    } else {
      scheduleFrame();
    }
  };

  resize();
  window.addEventListener("resize", resize);
  window.addEventListener("keydown", markActive);
  window.addEventListener("wheel", markActive, { passive: true });
  document.addEventListener("visibilitychange", onVisibilityChange);
  canvas.addEventListener("pointerdown", onPointer);
  markActive();
  if (reduced) {
    drawStatic();
  } else {
    scheduleFrame();
  }

  return () => {
    stopped = true;
    cancelScheduledFrame();
    clearIdleTimer();
    window.removeEventListener("resize", resize);
    window.removeEventListener("keydown", markActive);
    window.removeEventListener("wheel", markActive);
    document.removeEventListener("visibilitychange", onVisibilityChange);
    canvas.removeEventListener("pointerdown", onPointer);
  };
}

export function AmberCascadesBackground() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    if (!canvasRef.current) return undefined;
    return initAmberCascades(canvasRef.current);
  }, []);

  return <canvas ref={canvasRef} className="amber-cascades-canvas" aria-hidden="true" />;
}
