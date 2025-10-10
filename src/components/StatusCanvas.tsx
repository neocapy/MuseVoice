import React, { useEffect, useRef } from "react";

const SIDEBAR_WIDTH = 48;

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

    const size = SIDEBAR_WIDTH;
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

    const size = SIDEBAR_WIDTH;
    const center = size / 2;
    const radius = 16;

    // Clear
    ctx.clearRect(0, 0, size, size);

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
        fillColor = "#f59e0b";
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
    ctx.strokeStyle = strokeColor;
    ctx.lineWidth = 2;
    ctx.stroke();

    if (status === "ready") {
      // mic icon
      ctx.fillStyle = "white";
      ctx.strokeStyle = "white";
      ctx.lineWidth = 2;

      // @ts-ignore roundRect may be supported at runtime
      ctx.beginPath();
      // @ts-ignore
      ctx.roundRect(center - 3, center - 8, 6, 12, 2);
      ctx.fill();

      // stand
      ctx.beginPath();
      ctx.moveTo(center, center + 4);
      ctx.lineTo(center, center + 8);
      ctx.stroke();

      // base
      ctx.beginPath();
      ctx.moveTo(center - 4, center + 8);
      ctx.lineTo(center + 4, center + 8);
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

      const bgBase = { r: 243, g: 233, b: 233 };
      const bgHot = { r: 254, g: 182, b: 182 };
      const ringBase = { r: 254, g: 202, b: 202 };
      const ringHot = { r: 185, g: 28, b: 28 };
      const mix = (a: number, b: number, t: number) => Math.round(a + (b - a) * t);
      const bg = `rgb(${mix(bgBase.r, bgHot.r, avg)}, ${mix(bgBase.g, bgHot.g, avg)}, ${mix(bgBase.b, bgHot.b, avg)})`;
      const ring = `rgb(${mix(ringBase.r, ringHot.r, avg)}, ${mix(ringBase.g, ringHot.g, avg)}, ${mix(ringBase.b, ringHot.b, avg)})`;

      // redraw circle with dynamic fill
      ctx.save();
      ctx.beginPath();
      ctx.arc(center, center, radius, 0, Math.PI * 2);
      ctx.fillStyle = bg;
      ctx.fill();
      ctx.strokeStyle = ring;
      ctx.lineWidth = 2;
      ctx.stroke();

      // clip circle
      ctx.clip();

      const padding = 2;
      const innerR = radius - padding;
      const N = bins.length;
      const leftX = center - innerR;
      const width = innerR * 2;
      const step = width / Math.max(1, N - 1);
      const minHalfPx = 1;

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
      ctx.fillStyle = "rgb(255, 0, 0)";
      ctx.globalAlpha = 0.9;
      ctx.fill();
      ctx.globalAlpha = 1.0;

      ctx.restore();
    } else if (status === "processing") {
      ctx.fillStyle = "white";
      ctx.strokeStyle = "white";
      ctx.lineWidth = 2;
      const dotSize = 2;
      for (let i = 0; i < 3; i++) {
        ctx.beginPath();
        ctx.arc(center - 6 + i * 6, center, dotSize, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }, [status, waveformBins, waveformAvgRms]);

  return (
    <canvas
      id="status-canvas"
      width={48}
      height={48}
      ref={canvasRef}
      onClick={onClick}
    />
  );
};