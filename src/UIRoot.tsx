import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { emit } from "@tauri-apps/api/event";

import { useBackendListeners } from "./hooks/useBackendListeners";
import doneSound from "./done.wav";
import boowompSound from "./sounds/boowomp.mp3";
import bambooHitSound from "./sounds/bamboo_hit.mp3";
import pipeSound from "./sounds/pipe.mp3";

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
  bgProcessing: "#dbeafe", // soft blue tint
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

  const dpr = useDpr();
  // Adjust theme for recording look: light slate blue bg, icy white waveform
  const theme = defaultTheme; // future: pull from persisted settings

  // Hidden textarea + audio elements kept for compatibility
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const doneAudio = useMemo(() => new Audio(doneSound), []);
  const boowompAudio = useMemo(() => new Audio(boowompSound), []);
  const bambooHitAudio = useMemo(() => new Audio(bambooHitSound), []);
  const pipeAudio = useMemo(() => new Audio(pipeSound), []);

  useEffect(() => {
    doneAudio.preload = "auto";
    boowompAudio.preload = "auto";
    bambooHitAudio.preload = "auto";
    pipeAudio.preload = "auto";
  }, [doneAudio, boowompAudio, bambooHitAudio, pipeAudio]);

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

  useBackendListeners({
    insertMode: false,
    transcriptionText: "",
    isExpanded: false,
    setStatus,
    setWaveformBins,
    setWaveformAvgRms,
    setTranscriptionText: () => {},
    setLayoutMode: () => {},
    setRetryVisible,
    doneAudio,
    boowompAudio,
    bambooHitAudio,
    pipeAudio,
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

      // Background color per status
      let baseBg = theme.bgIdle;
      if (status === "processing") baseBg = "#d9e8ff"; // slate-blue while processing
      if (status === "recording") baseBg = "#d3e4ff"; // light slate blue

      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = baseBg;
      ctx.fillRect(0, 0, w, h);

      // Subtle glass overlay gradient (recording: darker along center line to contrast white waveform)
      const grad = ctx.createLinearGradient(0, 0, 0, h);
      if (status === "recording") {
        grad.addColorStop(0, "rgba(255,255,255,0.18)");
        grad.addColorStop(0.45, "rgba(0,0,0,0.12)");
        grad.addColorStop(0.5, "rgba(0,0,0,0.18)");
        grad.addColorStop(0.55, "rgba(0,0,0,0.12)");
        grad.addColorStop(1, "rgba(255,255,255,0.12)");
      } else {
        grad.addColorStop(0, "rgba(255,255,255,0.28)");
        grad.addColorStop(0.5, theme.glassOverlay);
        grad.addColorStop(1, "rgba(255,255,255,0.08)");
      }
      ctx.fillStyle = grad;
      ctx.fillRect(0, 0, w, h);

      if (status === "recording") {
        // Draw full-bleed waveform (edge to edge)
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
          const t = (db + 45) / 45; // -45 dB -> 0, 0 dB -> 1
          return clamp(t, 0, 1);
        };
        const avg = rmsToDbScale(waveformAvgRms || 0);

        // Amplitude envelope: allow reaching full height; icy white fill
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
        // Fill first, then crisp crest stroke for presence
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
      } else {
        // Idle / processing: 5-dot elliptical orbit with perspective-ish scaling & latch near front
        const cx = w / 2;
        const cy = h / 2;
        const Rx = Math.min(w, h) * 0.32;
        const Ry = Rx * 0.52; // tilt to feel 3D

        // Orientation fix: camera above, so near-front should appear at lower y (positive sin -> lower)
        // We achieve this by inverting the sign on the sin term when mapping to y depth for size/alpha

        // Latching step motion: advance in 72° sectors with ease-in/out per half-sector
        const sector = (Math.PI * 2) / 5; // 72°
        if (status === "processing") {
          // Ensure base motion so it doesn't stall
          const p = (angle % sector) / sector;
          const half = p < 0.5 ? p / 0.5 : (1 - p) / 0.5; // 0->1 then 1->0
          const eased = half * half; // quadratic ease
          const base = 0.35; // base sector/s to keep motion alive
          const stepSpeed = 2.4; // step rate
          angle += (base + stepSpeed * eased) * dt * sector;
        } else {
          // idle: lock to nearest rest position
          const k = Math.round(angle / sector);
          angle = k * sector;
        }

        const N = 5;
        for (let i = 0; i < N; i++) {
          const a = angle + (i * 2 * Math.PI) / N;
          const x = cx + Rx * Math.cos(a);
          const y = cy + Ry * Math.sin(a);
          const depth = 0.5 + 0.5 * (1 + Math.sin(a)); // camera above => sin(a)>0 is closer
          const r = 4 + 6 * depth;
          const alpha = 0.35 + 0.45 * depth;
          ctx.beginPath();
          ctx.fillStyle = `rgba(15, 23, 42, ${alpha.toFixed(3)})`;
          ctx.arc(x, y, r, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      raf = requestAnimationFrame(draw);
    };

    raf = requestAnimationFrame((t) => {
      lastTs = t;
      raf = requestAnimationFrame(draw);
    });
    return () => cancelAnimationFrame(raf);
  }, [status, waveformBins, waveformAvgRms, theme]);

  const onMinimize = useCallback(async () => {
    try {
      await getCurrentWindow().minimize();
    } catch (e) {
      console.error("Failed to minimize window:", e);
    }
  }, []);

  const onClose = useCallback(async () => {
    try {
      await getCurrentWindow().close();
    } catch (e) {
      console.error("Failed to close window:", e);
    }
  }, []);

  const onSettings = useCallback(async () => {
    console.log("Settings clicked (stub). Emitting 'open-settings' event.");
    try {
      await emit("open-settings", { source: "uiroot" });
    } catch (e) {
      console.error("Failed to emit open-settings:", e);
    }
  }, []);

  const onPlayPause = useCallback(async () => {
    try {
      if (status === "ready") {
        await invoke<string>("start_audio_stream", { origin: "ui-button" });
      } else if (status === "recording") {
        await invoke<string>("stop_audio_stream", { origin: "ui-button" });
      } else if (status === "processing") {
        await invoke<string>("cancel_transcription", { origin: "ui-button" });
      }
    } catch (e) {
      console.error("Failed to toggle play/pause:", e);
    }
  }, [status]);

  const onRetry = useCallback(async () => {
    try {
      await invoke<string>("retry_transcription", { origin: "ui-button" });
    } catch (e) {
      console.error("Failed to retry transcription:", e);
    }
  }, []);

  // Whole window drag region; buttons explicitly no-drag
  return (
    <div className="uiroot" data-tauri-drag-region>
      <canvas ref={canvasRef} className="uiroot-canvas" />

      <div className="ui-controls">
        <div className="left-controls">
          <button className="ctrl btn settings no-drag" onClick={onSettings} title="Settings (stub)">
            ⚙️
          </button>
        </div>
        <div className="right-controls">
          <button className="ctrl btn no-drag" onClick={onMinimize} title="Minimize">
            −
          </button>
          <button className="ctrl btn no-drag" onClick={onClose} title="Close">
            ✕
          </button>
        </div>
        <div className="bottom-right">
          <button className="ctrl pill no-drag" onClick={onPlayPause} title="Play / Pause">
            {status === "recording" ? "⏹" : status === "processing" ? "⏹" : "⏺"}
          </button>
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


