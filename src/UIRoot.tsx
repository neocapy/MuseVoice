import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { useBackendListeners } from "./hooks/useBackendListeners";

type FrontendStatus = "loading" | "ready" | "recording" | "processing";

type Theme = {
  bgIdle: string;
  bgProcessing: string;
  bgRecording: string;
  glassOverlay: string;
  controlBg: string;
  controlFg: string;
  controlHoverBg: string;
  outline: string;
};

const defaultTheme: Theme = {
  bgIdle: "#e5e7eb", // light slate-ish gray
  bgProcessing: "#c5cad9", // soft orange tint
  bgRecording: "#fee2e2", // light red tint
  glassOverlay: "rgba(255,255,255,0.18)",
  controlBg: "rgba(255,255,255,0.24)",
  controlFg: "#0f172a",
  controlHoverBg: "rgba(255,255,255,0.35)",
  outline: "rgba(15,23,42,0.12)",
};

function useDpr() {
  const [dpr, setDpr] = useState<number>(window.devicePixelRatio || 1);
  useEffect(() => {
    const onChange = () => setDpr(window.devicePixelRatio || 1);
    const mm = window.matchMedia(`(resolution: ${dpr}dppx)`);
    mm.addEventListener("change", onChange);
    return () => mm.removeEventListener("change", onChange);
  }, [dpr]);
  return dpr;
}

const clamp = (n: number, min: number, max: number) => Math.max(min, Math.min(max, n));

export default function UIRoot() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [status, setStatus] = useState<FrontendStatus>("loading");
  const [waveformBins, setWaveformBins] = useState<number[]>([]);
  const [waveformAvgRms, setWaveformAvgRms] = useState<number>(0);
  const [retryVisible, setRetryVisible] = useState<boolean>(false);
  const waveformUpdateCountRef = useRef<number>(0);

  const dpr = useDpr();
  // Adjust theme for recording look: light slate blue bg, icy white waveform
  const theme = defaultTheme; // future: pull from persisted settings

  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const copyToClipboard = useCallback(async (text: string) => {
    if (!text.trim()) return;
    try {
      await invoke("copy_to_clipboard", { text });
    } catch (e) {
      console.error("Failed to copy text to clipboard:", e);
    }
  }, []);

  const addSmartSpacing = useCallback((text: string, insertPosition: number, fullText: string) => {
    const noSpaceAfter = new Set(["(", "[", "{", '"', "'", "`", " ", "\n", "\t"]);
    const noSpaceBefore = new Set([
      ")",
      "]",
      "}",
      ".",
      ",",
      ";",
      ":",
      "!",
      "?",
      '"',
      "'",
      "`",
      " ",
      "\n",
      "\t",
    ]);

    let processedText = text;
    let positionAdjustment = 0;

    const charBefore = insertPosition > 0 ? fullText[insertPosition - 1] : "";
    const firstCharOfText = text.length > 0 ? text[0] : "";
    if (charBefore && !noSpaceAfter.has(charBefore) && !noSpaceBefore.has(firstCharOfText) && firstCharOfText !== " ") {
      processedText = " " + processedText;
      positionAdjustment += 1;
    }

    const charAfter = insertPosition < fullText.length ? fullText[insertPosition] : "";
    const lastCharOfText = text.length > 0 ? text[text.length - 1] : "";
    if (charAfter && !noSpaceBefore.has(charAfter) && !noSpaceAfter.has(lastCharOfText) && lastCharOfText !== " ") {
      processedText = processedText + " ";
    }

    return { text: processedText, adjustedPosition: insertPosition + positionAdjustment };
  }, []);

  const removeTrailingPunctuation = useCallback((text: string) => text.replace(/[.!?;,]*\s*$/, "").trimEnd(), []);

  useEffect(() => {
    setStatus("ready");
  }, []);

  const wrappedSetWaveformBins = useCallback((bins: number[]) => {
    setWaveformBins(bins);
    waveformUpdateCountRef.current += 1;
  }, []);

  const wrappedSetStatus = useCallback((newStatus: FrontendStatus) => {
    setStatus(newStatus);
    if (newStatus === "recording") {
      waveformUpdateCountRef.current = 0;
    }
  }, []);

  useBackendListeners({
    insertMode: false,
    transcriptionText: "",
    isExpanded: false,
    setStatus: wrappedSetStatus,
    setWaveformBins: wrappedSetWaveformBins,
    setWaveformAvgRms,
    setTranscriptionText: () => {},
    setLayoutMode: () => {},
    setRetryVisible,
    copyToClipboard,
    textareaRef,
    addSmartSpacing,
    removeTrailingPunctuation,
  });

  // Canvas sizing
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const resize = () => {
      const w = Math.max(1, window.innerWidth);
      const h = Math.max(1, window.innerHeight);
      canvas.width = Math.floor(w * dpr);
      canvas.height = Math.floor(h * dpr);
      canvas.style.width = `${w}px`;
      canvas.style.height = `${h}px`;
      const ctx = canvas.getContext("2d");
      if (ctx) {
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        ctx.scale(dpr, dpr);
      }
    };
    resize();
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, [dpr]);

  // Render loop for idle / processing animations and recording waveform
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let raf = 0;
    let lastTs = 0;
    let angle = 0; // radians

    const draw = (ts: number) => {
      const dt = clamp((ts - lastTs) / 1000, 0, 1 / 15);
      lastTs = ts;

      const w = canvas.clientWidth;
      const h = canvas.clientHeight;

      let baseBg = theme.bgIdle;
      if (status === "processing") baseBg = theme.bgProcessing;
      if (status === "recording") {
        const targetBg = "#d3e4ff";
        const updateCount = waveformUpdateCountRef.current;
        const t = clamp(updateCount / 5, 0, 1);
        
        const lerpColor = (from: string, to: string, t: number): string => {
          const fromRgb = {
            r: parseInt(from.slice(1, 3), 16),
            g: parseInt(from.slice(3, 5), 16),
            b: parseInt(from.slice(5, 7), 16),
          };
          const toRgb = {
            r: parseInt(to.slice(1, 3), 16),
            g: parseInt(to.slice(3, 5), 16),
            b: parseInt(to.slice(5, 7), 16),
          };
          const r = Math.round(fromRgb.r + (toRgb.r - fromRgb.r) * t);
          const g = Math.round(fromRgb.g + (toRgb.g - fromRgb.g) * t);
          const b = Math.round(fromRgb.b + (toRgb.b - fromRgb.b) * t);
          return `#${r.toString(16).padStart(2, '0')}${g.toString(16).padStart(2, '0')}${b.toString(16).padStart(2, '0')}`;
        };
        
        baseBg = lerpColor(theme.bgIdle, targetBg, t);
      }

      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = baseBg;
      ctx.fillRect(0, 0, w, h);

      const grad = ctx.createLinearGradient(0, 0, 0, h);
      if (status === "recording") {
        grad.addColorStop(0, "rgba(255,255,255,0.18)");
        grad.addColorStop(0.45, "rgba(0,0,0,0.03)");
        grad.addColorStop(0.5, "rgba(0,0,0,0.05)");
        grad.addColorStop(0.55, "rgba(0,0,0,0.03)");
        grad.addColorStop(1, "rgba(255,255,255,0.12)");
      } else {
        grad.addColorStop(0, "rgba(255,255,255,0.28)");
        grad.addColorStop(0.5, theme.glassOverlay);
        grad.addColorStop(1, "rgba(255,255,255,0.08)");
      }
      ctx.fillStyle = grad;
      ctx.fillRect(0, 0, w, h);

      if (status === "recording") {
        const bins = waveformBins && waveformBins.length > 0 ? waveformBins : new Array(256).fill(0);
        const left = 0;
        const right = w;
        const top = 0;
        const bottom = h;
        const mid = (top + bottom) / 2;
        const width = right - left;
        const step = width / Math.max(1, bins.length - 1);

        const rmsToDbScale = (r: number) => {
          const eps = 1e-8;
          const db = 20 * Math.log10(Math.max(r, eps));
          const t = (db + 45) / 45;
          return clamp(t, 0, 1);
        };
        const avg = rmsToDbScale(waveformAvgRms || 0);

        const amp = Math.max(h * 0.48, 32) * (0.7 + 0.45 * avg);
        const fill = `rgba(255, 255, 255, ${0.96})`;

        ctx.save();
        ctx.beginPath();
        for (let i = 0; i < bins.length; i++) {
          const x = left + i * step;
          const mag = rmsToDbScale(Math.max(bins[i], 0));
          const y = mid - mag * amp;
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        for (let i = bins.length - 1; i >= 0; i--) {
          const x = left + i * step;
          const mag = rmsToDbScale(Math.max(bins[i], 0));
          const y = mid + mag * amp;
          ctx.lineTo(x, y);
        }
        ctx.closePath();
        ctx.fillStyle = fill;
        ctx.fill();
        ctx.beginPath();
        for (let i = 0; i < bins.length; i++) {
          const x = left + i * step;
          const mag = rmsToDbScale(Math.max(bins[i], 0));
          const y = mid - mag * amp;
          if (i === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.strokeStyle = "rgba(255,255,255,0.85)";
        ctx.lineWidth = 1;
        ctx.stroke();
        ctx.restore();
      } else if (status === "processing") {
        const cx = w / 2;
        const cy = h / 2;
        const radius = 8;

        const sector = (Math.PI * 2) / 3;
        const p = (angle % sector) / sector;
        const half = p < 0.5 ? p / 0.5 : (1 - p) / 0.5;
        const eased = half * half;
        const base = 0.35;
        const stepSpeed = 2.4;
        angle += (base + stepSpeed * eased) * dt * sector;

        const N = 3;
        for (let i = 0; i < N; i++) {
          const a = angle + (i * 2 * Math.PI) / N;
          const x = cx + radius * Math.cos(a);
          const y = cy + radius * Math.sin(a);
          ctx.beginPath();
          ctx.fillStyle = "rgba(15, 23, 42, 0.5)";
          ctx.arc(x, y, 4, 0, Math.PI * 2);
          ctx.fill();
        }
      } else {
        const cx = w / 2;
        const cy = h / 2;
        ctx.beginPath();
        ctx.fillStyle = "rgba(185, 28, 28, 0.75)";
        ctx.arc(cx, cy, 8, 0, Math.PI * 2);
        ctx.fill();
      }

      raf = requestAnimationFrame(draw);
    };

    raf = requestAnimationFrame((t) => {
      lastTs = t;
      raf = requestAnimationFrame(draw);
    });
    return () => cancelAnimationFrame(raf);
  }, [status, waveformBins, waveformAvgRms, theme]);

  const onRetry = useCallback(async () => {
    try {
      await invoke<string>("retry_transcription");
    } catch (e) {
      console.error("Failed to retry transcription:", e);
    }
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    let mouseDownPos: { x: number; y: number; time: number } | null = null;

    const onMouseDown = async (e: MouseEvent) => {
      if (e.button !== 0) return;
      mouseDownPos = { x: e.clientX, y: e.clientY, time: Date.now() };
      
      try {
        await getCurrentWindow().startDragging();
      } catch (err) {
        console.error("Failed to start dragging:", err);
      }
    };

    const onMouseUp = async (e: MouseEvent) => {
      if (!mouseDownPos || e.button !== 0) return;

      const dx = e.clientX - mouseDownPos.x;
      const dy = e.clientY - mouseDownPos.y;
      const dt = Date.now() - mouseDownPos.time;
      const dist = Math.sqrt(dx * dx + dy * dy);

      if (dist < 5 && dt < 300) {
        try {
          if (status === "ready") {
            await invoke<string>("start_audio_stream");
          } else if (status === "recording") {
            await invoke<string>("stop_audio_stream");
          } else if (status === "processing") {
            await invoke<string>("cancel_transcription");
          }
        } catch (err) {
          console.error("Failed to toggle recording via canvas click:", err);
        }
      }

      mouseDownPos = null;
    };

    const onContextMenu = async (e: MouseEvent) => {
      e.preventDefault();
      try {
        await invoke("show_context_menu");
      } catch (err) {
        console.error("Failed to show context menu:", err);
      }
    };

    canvas.addEventListener("mousedown", onMouseDown);
    canvas.addEventListener("mouseup", onMouseUp);
    canvas.addEventListener("contextmenu", onContextMenu);

    return () => {
      canvas.removeEventListener("mousedown", onMouseDown);
      canvas.removeEventListener("mouseup", onMouseUp);
      canvas.removeEventListener("contextmenu", onContextMenu);
    };
  }, [status]);

  return (
    <div className="uiroot">
      <canvas ref={canvasRef} className="uiroot-canvas" />

      <div className="ui-controls">
        <div className="bottom-right">
          {retryVisible && (
            <button className="ctrl pill ghost no-drag" onClick={onRetry} title="Retry last">
              Retry
            </button>
          )}
        </div>
      </div>

      {/* Hidden textarea for compatibility; not used for input */}
      <textarea ref={textareaRef} style={{ display: "none" }} />
    </div>
  );
}


