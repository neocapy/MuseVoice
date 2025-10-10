import React, { useEffect, useRef } from "react";

const CANVAS_SIZE = 100;

interface StatusCanvasProps {
  status: "loading" | "ready" | "recording" | "processing";
  waveformBins: number[];
  waveformAvgRms: number;
  dpr: number;
  onClick: () => void;
}

export const StatusCanvas: React.FC<StatusCanvasProps> = ({
  status,
  waveformBins,
  waveformAvgRms,
  dpr,
  onClick,
}) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  // Canvas resolution scaling on DPR change
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const size = CANVAS_SIZE;
    const width = size * dpr;
    const height = size * dpr;

    // Only update backing store size and CSS size when DPR changes
    canvas.width = width;
    canvas.height = height;
    canvas.style.width = `${size}px`;
    canvas.style.height = `${size}px`;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    // Scale once to match CSS pixels to device pixels
    ctx.scale(dpr, dpr);
  }, [dpr]);

  // Canvas drawing effect
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const size = CANVAS_SIZE;
    const center = size / 2;
    const radius = 40;

    // Clear and fill entire canvas with gradient background
    ctx.clearRect(0, 0, size, size);

    // Draw gradient background that fills the entire square
    const gradient = ctx.createLinearGradient(0, 0, size, size);
    gradient.addColorStop(0, '#667eea');
    gradient.addColorStop(1, '#764ba2');
    ctx.fillStyle = gradient;
    ctx.fillRect(0, 0, size, size);

    // Colors
    let fillColor = "#9ca3af";
    let strokeColor = "#6b7280";
    switch (status) {
      case "ready":
        fillColor = "#6366f1";
        strokeColor = "#4f46e5";
        break;
      case "recording":
        fillColor = "#ef4444";
        strokeColor = "#dc2626";
        break;
      case "processing":
        fillColor = "#f5be0b";
        strokeColor = "#d97706";
        break;
      case "loading":
      default:
        fillColor = "#9ca3af";
        strokeColor = "#6b7280";
        break;
    }

    // Draw circle
    ctx.beginPath();
    ctx.arc(center, center, radius, 0, Math.PI * 2);
    ctx.fillStyle = fillColor;
    ctx.fill();
    // No stroke/ring on the main circle

    if (status === "ready") {
      // mic icon
      ctx.fillStyle = "white";
      ctx.strokeStyle = "white";
      ctx.lineWidth = 3;

      // @ts-ignore roundRect may be supported at runtime
      ctx.beginPath();
      // @ts-ignore
      ctx.roundRect(center - 6, center - 16, 12, 24, 3);
      ctx.fill();

      // stand
      ctx.beginPath();
      ctx.moveTo(center, center + 8);
      ctx.lineTo(center, center + 16);
      ctx.stroke();

      // base
      ctx.beginPath();
      ctx.moveTo(center - 8, center + 16);
      ctx.lineTo(center + 8, center + 16);
      ctx.stroke();
    } else if (status === "recording") {
      // waveform
      const bins = waveformBins && waveformBins.length > 0 ? waveformBins : new Array(256).fill(0);
      const rmsToDbScale = (r: number) => {
        const eps = 1e-8;
        const db = 20 * Math.log10(Math.max(r, eps));
        const t = (db + 40) / 40; // -40 dB -> 0, 0 dB -> 1
        return Math.max(0, Math.min(1, t));
      };
      const avg = rmsToDbScale(waveformAvgRms || 0);

      const bgBase = { r: 192, g: 0, b: 0 };
      const bgHot = { r: 255, g: 160, b: 0 };
      const ringBase = { r: 220, g: 38, b: 38 };
      const ringHot = { r: 254, g: 80, b: 80 };
      const mix = (a: number, b: number, t: number) => Math.round(a + (b - a) * t);
      const bg = `rgb(${mix(bgBase.r, bgHot.r, avg)}, ${mix(bgBase.g, bgHot.g, avg)}, ${mix(bgBase.b, bgHot.b, avg)})`;
      const ring = `rgb(${mix(ringBase.r, ringHot.r, avg)}, ${mix(ringBase.g, ringHot.g, avg)}, ${mix(ringBase.b, ringHot.b, avg)})`;

      // redraw circle with dynamic fill
      ctx.save();
      ctx.beginPath();
      ctx.arc(center, center, radius, 0, Math.PI * 2);
      ctx.fillStyle = bg;
      ctx.fill();

      // clip circle
      ctx.clip();

      const padding = 4;
      const innerR = radius - padding;
      const N = bins.length;
      const leftX = center - innerR;
      const width = innerR * 2;
      const step = width / Math.max(1, N - 1);
      const minHalfPx = 2;

      ctx.beginPath();
      for (let i = 0; i < N; i++) {
        const x = leftX + i * step;
        const amp = rmsToDbScale(Math.max(bins[i], 0));
        const half = Math.max(minHalfPx, amp * innerR);
        const yTop = center - half;
        if (i === 0) ctx.moveTo(x, yTop);
        else ctx.lineTo(x, yTop);
      }
      for (let i = N - 1; i >= 0; i--) {
        const x = leftX + i * step;
        const amp = rmsToDbScale(Math.max(bins[i], 0));
        const half = Math.max(minHalfPx, amp * innerR);
        const yBot = center + half;
        ctx.lineTo(x, yBot);
      }
      ctx.closePath();
      ctx.fillStyle = "rgba(255, 255, 255, 1.0)";
      ctx.fill();
      ctx.globalAlpha = 1.0;

      ctx.restore();
    } else if (status === "processing") {
      ctx.fillStyle = "white";
      ctx.strokeStyle = "white";
      ctx.lineWidth = 3;
      const dotSize = 3;
      for (let i = 0; i < 3; i++) {
        ctx.beginPath();
        ctx.arc(center - 12 + i * 12, center, dotSize, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }, [status, waveformBins, waveformAvgRms]);

  return (
    <canvas
      id="status-canvas"
      width={128}
      height={128}
      ref={canvasRef}
      onClick={onClick}
    />
  );
};
