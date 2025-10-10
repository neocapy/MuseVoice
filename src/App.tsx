import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

import doneSound from "./done.wav";
import boowompSound from "./sounds/boowomp.mp3";
import bambooHitSound from "./sounds/bamboo_hit.mp3";
import pipeSound from "./sounds/pipe.mp3";

// Constants
const SIDEBAR_WIDTH = 48;
const COLLAPSE_WIDTH = 72;
const COLLAPSE_HEIGHT = 72;

type FrontendStatus = "loading" | "ready" | "recording" | "processing";
type Model = "whisper-1" | "gpt-4o-transcribe";
type RawChatMode = "raw" | "chat";
type WaveformChunkPayload = { bins: number[]; avgRms?: number; avg_rms?: number };
type FlowState = "idle" | "recording" | "processing" | "completed" | "error" | "cancelled";

function formatSampleCount(samples: number): string {
  if (samples >= 1000) return Math.floor(samples / 1000) + "k";
  return samples.toString();
}

function addSmartSpacing(text: string, insertPosition: number, fullText: string) {
  const noSpaceAfter = new Set(["(", "[", "{", '"', "'", "`", " ", "\n", "\t"]);
  const noSpaceBefore = new Set([")", "]", "}", ".", ",", ";", ":", "!", "?", '"', "'", "`", " ", "\n", "\t"]);

  let processedText = text;
  let positionAdjustment = 0;

  const charBefore = insertPosition > 0 ? fullText[insertPosition - 1] : "";
  const firstCharOfText = text.length > 0 ? text[0] : "";

  if (
    charBefore &&
    !noSpaceAfter.has(charBefore) &&
    !noSpaceBefore.has(firstCharOfText) &&
    firstCharOfText !== " "
  ) {
    processedText = " " + processedText;
    positionAdjustment += 1;
  }

  const charAfter = insertPosition < fullText.length ? fullText[insertPosition] : "";
  const lastCharOfText = text.length > 0 ? text[text.length - 1] : "";

  if (charAfter && !noSpaceBefore.has(charAfter) && !noSpaceAfter.has(lastCharOfText) && lastCharOfText !== " ") {
    processedText = processedText + " ";
  }

  return { text: processedText, adjustedPosition: insertPosition + positionAdjustment };
}

function removeTrailingPunctuation(text: string): string {
  return text.replace(/[.!?;,]*\s*$/, "").trimEnd();
}

function useDpr() {
  const [dpr, setDpr] = useState<number>(window.devicePixelRatio || 1);

  useEffect(() => {
    const mm = window.matchMedia(`(resolution: ${dpr}dppx)`);
    const onChange = () => setDpr(window.devicePixelRatio || 1);
    mm.addEventListener("change", onChange);
    return () => mm.removeEventListener("change", onChange);
  }, [dpr]);

  return dpr;
}

function computeLayoutMode(winW: number, winH: number): "expanded" | "collapsed" | "h-collapsed" {
  const isHorizontalCollapsed = winH < COLLAPSE_HEIGHT;
  const isVerticalCollapsed = !isHorizontalCollapsed && winW < COLLAPSE_WIDTH;
  if (!isHorizontalCollapsed && !isVerticalCollapsed) return "expanded";
  if (isHorizontalCollapsed) return "h-collapsed";
  return "collapsed";
}

function useLayoutMode() {
  const [mode, setMode] = useState<"expanded" | "collapsed" | "h-collapsed">(
    computeLayoutMode(window.innerWidth, window.innerHeight)
  );

  useEffect(() => {
    const onResize = () => setMode(computeLayoutMode(window.innerWidth, window.innerHeight));
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return [mode, setMode] as const;
}

export default function App() {
  // State (React-driven)
  const [status, setStatus] = useState<FrontendStatus>("loading");
  const [insertMode, setInsertMode] = useState<boolean>(false);
  const [rewriteEnabled, setRewriteEnabled] = useState<boolean>(false);
  const [model, setModel] = useState<Model>("whisper-1");
  const [rawChatMode, setRawChatMode] = useState<RawChatMode>("raw");
  const [retryVisible, setRetryVisible] = useState<boolean>(false);

  const [currentSamples, setCurrentSamples] = useState<number | null>(null);
  const [waveformBins, setWaveformBins] = useState<number[]>([]);
  const [waveformAvgRms, setWaveformAvgRms] = useState<number>(0);

  const [transcriptionText, setTranscriptionText] = useState<string>("");

  const [layoutMode, setLayoutMode] = useLayoutMode();
  const isExpanded = layoutMode === "expanded";

  // Refs
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // DPR handling for canvas resolution
  const dpr = useDpr();

  // Audio elements
  const doneAudio = useMemo(() => {
    const a = new Audio(doneSound);
    a.preload = "auto";
    a.volume = 1.0;
    return a;
  }, []);
  const boowompAudio = useMemo(() => {
    const a = new Audio(boowompSound);
    a.preload = "auto";
    a.volume = 1.0;
    return a;
  }, []);
  const bambooHitAudio = useMemo(() => {
    const a = new Audio(bambooHitSound);
    a.preload = "auto";
    a.volume = 1.0;
    return a;
  }, []);
  const pipeAudio = useMemo(() => {
    const a = new Audio(pipeSound);
    a.preload = "auto";
    a.volume = 1.0;
    return a;
  }, []);

  const lastAudioPlayTimeRef = useRef<Map<string, number>>(new Map());
  const audioPlayCountRef = useRef<Map<string, number>>(new Map());
  const AUDIO_DEBOUNCE_MS = 150;

  // Update root classes based on layout mode (keep exact CSS behavior)
  useEffect(() => {
    const root = document.getElementById("root");
    if (!root) return;

    // Ensure base class
    if (!root.classList.contains("app-container")) {
      root.classList.add("app-container");
    }

    root.classList.remove("expanded", "collapsed", "h-collapsed");
    root.classList.add(layoutMode);
  }, [layoutMode]);

  // Initial status
  useEffect(() => {
    setStatus("ready");
  }, []);

  // Rewrite enabled -> backend
  useEffect(() => {
    (async () => {
      try {
        await invoke("set_rewrite_enabled", { enabled: rewriteEnabled });
      } catch (e) {
        console.error("Failed to set rewrite enabled:", e);
      }
    })();
  }, [rewriteEnabled]);

  // Retry availability on load
  useEffect(() => {
    (async () => {
      try {
        const hasRetryData = await invoke<boolean>("has_retry_data");
        setRetryVisible(!!hasRetryData);
      } catch (e) {
        console.error("Failed to check initial retry data:", e);
      }
    })();
  }, []);

  // Clipboard helper
  const copyToClipboard = useCallback(async (text: string) => {
    if (!text.trim()) return;
    try {
      await invoke("copy_to_clipboard", { text });
    } catch (e) {
      console.error("Failed to copy text to clipboard:", e);
    }
  }, []);

  // Textarea change
  const onTextareaChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      const val = e.target.value;
      setTranscriptionText(val);
      copyToClipboard(val);
    },
    [copyToClipboard]
  );

  // Tauri backend event listeners
  useEffect(() => {
    let unsubs: UnlistenFn[] = [];
    let mounted = true;

    (async () => {
      try {
        // Flow state changes
        unsubs.push(
          await listen<FlowState>("flow-state-changed", (event) => {
            if (!mounted) return;
            const state = event.payload;
            switch (state) {
              case "idle":
                setStatus("ready");
                break;
              case "recording":
                setStatus("recording");
                break;
              case "processing":
                setStatus("processing");
                break;
              case "completed":
              case "error":
              case "cancelled":
                setStatus("ready");
                break;
            }
          })
        );

        // Sample count updates
        unsubs.push(
          await listen<number>("sample-count", (event) => {
            if (!mounted) return;
            setCurrentSamples(event.payload);
          })
        );

        // Waveform chunks ~20Hz
        unsubs.push(
          await listen<WaveformChunkPayload>("waveform-chunk", (event) => {
            if (!mounted) return;
            const p = event.payload;
            if (!p || !p.bins) return;
            setWaveformBins(p.bins);
            setWaveformAvgRms((p as any).avg_rms ?? p.avgRms ?? 0);
          })
        );

        // Transcription result
        unsubs.push(
          await listen<string>("transcription-result", async (event) => {
            if (!mounted) return;
            const incoming = event.payload || "";
            let processedText = incoming;
            if (rawChatMode === "chat") {
              processedText = removeTrailingPunctuation(incoming);
            }

            if (insertMode && textareaRef.current) {
              const currentText = transcriptionText;
              const cursorPosition = textareaRef.current.selectionStart || 0;
              const { text: spacedText, adjustedPosition } = addSmartSpacing(
                processedText,
                cursorPosition,
                currentText
              );
              const before = currentText.substring(0, cursorPosition);
              const after = currentText.substring(cursorPosition);
              const newText = before + spacedText + after;
              setTranscriptionText(newText);

              // Position cursor at end of inserted text
              const newCursorPosition = adjustedPosition + spacedText.length;
              requestAnimationFrame(() => {
                textareaRef.current?.setSelectionRange(newCursorPosition, newCursorPosition);
              });

              // Copy entire new text
              copyToClipboard(newText);
            } else {
              setTranscriptionText(processedText);
              requestAnimationFrame(() => {
                const end = processedText.length;
                textareaRef.current?.setSelectionRange(end, end);
              });
              copyToClipboard(processedText);
            }

            // Auto-expand if collapsed and window large enough
            if (
              !isExpanded &&
              processedText.trim() &&
              window.innerWidth >= COLLAPSE_WIDTH &&
              window.innerHeight >= COLLAPSE_HEIGHT
            ) {
              setLayoutMode("expanded");
            }

            // Play done sound
            try {
              await doneAudio.play();
            } catch (e) {
              console.error("Failed to play done sound:", e);
            }
          })
        );

        // Flow error
        unsubs.push(
          await listen<any>("flow-error", () => {
            if (!mounted) return;
            setStatus("ready");
          })
        );

        // Retry availability changes
        unsubs.push(
          await listen<boolean>("retry-available", (event) => {
            if (!mounted) return;
            setRetryVisible(!!event.payload);
          })
        );

        // Audio feedback events
        unsubs.push(
          await listen<string>("audio-feedback", async (event) => {
            if (!mounted) return;
            const soundFile = event.payload;

            const now = Date.now();
            const lastMap = lastAudioPlayTimeRef.current;
            const countMap = audioPlayCountRef.current;

            const lastPlayTime = lastMap.get(soundFile) || 0;
            if (now - lastPlayTime < AUDIO_DEBOUNCE_MS) {
              return; // debounced
            }
            lastMap.set(soundFile, now);
            countMap.set(soundFile, (countMap.get(soundFile) || 0) + 1);

            let audio: HTMLAudioElement | null = null;
            switch (soundFile) {
              case "boowomp.mp3":
                audio = boowompAudio;
                break;
              case "bamboo_hit.mp3":
                audio = bambooHitAudio;
                break;
              case "pipe.mp3":
                audio = pipeAudio;
                break;
              default:
                audio = null;
            }
            if (!audio) return;

            try {
              if (!audio.paused) audio.pause();
              audio.currentTime = 0;
              const p = audio.play();
              if (p) await p;
            } catch (e) {
              try {
                audio.load();
                await new Promise((r) => setTimeout(r, 100));
                await audio.play();
              } catch (err) {
                console.error("Audio feedback failed:", err);
              }
            }
          })
        );
      } catch (e) {
        console.error("Failed to set up backend event listeners:", e);
      }
    })();

    return () => {
      mounted = false;
      unsubs.forEach((u) => {
        try {
          u();
        } catch {
          // ignore
        }
      });
    };
  }, [
    insertMode,
    rawChatMode,
    transcriptionText,
    isExpanded,
    setLayoutMode,
    doneAudio,
    boowompAudio,
    bambooHitAudio,
    pipeAudio,
    copyToClipboard,
  ]);

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

  // Microphone button (canvas) click behavior
  const handleMicClick = useCallback(async () => {
    if (status === "ready") {
      try {
        setStatus("recording");
        await invoke<string>("start_audio_stream", { origin: "click" });
      } catch (e) {
        console.error("Failed to start recording:", e);
        setStatus("ready");
      }
    } else if (status === "recording") {
      try {
        setStatus("processing");
        await invoke<string>("stop_audio_stream", { origin: "click" });
      } catch (e) {
        console.error("Failed to stop recording:", e);
        setStatus("ready");
      }
    } else if (status === "processing") {
      try {
        await invoke<string>("cancel_transcription", { origin: "click" });
        setStatus("ready");
      } catch (e) {
        console.error("Failed to cancel transcription:", e);
        setStatus("ready");
      }
    }
  }, [status]);

  // Global Tab key triggers microphone click
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Tab") {
        e.preventDefault();
        handleMicClick();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [handleMicClick]);

  // Derived status label
  const statusLabelText = (() => {
    switch (status) {
      case "loading":
        return "Loading";
      case "ready":
        return "Ready";
      case "recording":
        return currentSamples !== null ? formatSampleCount(currentSamples) : "Rec";
      case "processing":
        return "Proc";
      default:
        return "Ready";
    }
  })();

  // Button handlers
  const onClose = useCallback(async () => {
    try {
      await getCurrentWindow().close();
    } catch (e) {
      console.error("Failed to close window:", e);
    }
  }, []);

  const onMinimize = useCallback(async () => {
    try {
      await getCurrentWindow().minimize();
    } catch (e) {
      console.error("Failed to minimize window:", e);
    }
  }, []);

  const onModeToggle = useCallback(() => {
    setInsertMode((prev) => !prev);
  }, []);

  const onRewriteToggle = useCallback(() => {
    setRewriteEnabled((prev) => !prev);
  }, []);

  const onModelToggle = useCallback(async () => {
    setModel((prev) => {
      const next: Model = prev === "whisper-1" ? "gpt-4o-transcribe" : "whisper-1";
      invoke("set_transcription_model", { model: next }).catch((e) => {
        console.error("Failed to set model:", e);
      });
      return next;
    });
  }, []);

  const onRawChatToggle = useCallback(() => {
    setRawChatMode((prev) => (prev === "raw" ? "chat" : "raw"));
  }, []);

  const onRetry = useCallback(async () => {
    try {
      setStatus("processing");
      await invoke<string>("retry_transcription", { origin: "click" });
    } catch (e) {
      console.error("Failed to retry transcription:", e);
      setStatus("ready");
    }
  }, []);

  // Rewrite button styling to match imperative code
  const rewriteBtnStyle: React.CSSProperties = useMemo(() => {
    return rewriteEnabled
      ? {
          backgroundColor: "rgba(99, 102, 241, 0.2)",
          borderColor: "rgba(99, 102, 241, 0.6)",
        }
      : {
          backgroundColor: "rgba(99, 102, 241, 0.05)",
          borderColor: "rgba(99, 102, 241, 0.2)",
        };
  }, [rewriteEnabled]);

  // Render EXACT same subtree as in index.html under #root (no extra wrapper)
  return (
    <>
      {/* Left Sidebar */}
      <div className="sidebar">
        <div className="drag-handle" data-tauri-drag-region title="Drag to move window"></div>
        <div className="sidebar-content">
          <canvas
            id="status-canvas"
            width={48}
            height={48}
            ref={canvasRef}
            onClick={handleMicClick}
          ></canvas>
          <label id="status-label" className="status-label">
            {statusLabelText}
          </label>
        </div>
        <div className="error-optional">
          <button
            id="retry-btn"
            className="control-btn retry-btn"
            title="Retry last transcription"
            style={{ display: retryVisible ? "flex" : "none" }}
            onClick={onRetry}
          >
            ⟳
          </button>
        </div>
        <div className="model-toggle">
          <button
            id="model-toggle-btn"
            className="control-btn mode-btn"
            title="Toggle transcription model"
            onClick={onModelToggle}
          >
            {model === "whisper-1" ? "Whis" : "4o-t"}
          </button>
          <button
            id="raw-chat-toggle-btn"
            className="control-btn mode-btn"
            title={rawChatMode === "raw" ? "Raw mode (Click to switch to Chat)" : "Chat mode (Click to switch to Raw)"}
            onClick={onRawChatToggle}
          >
            {rawChatMode === "raw" ? "Raw" : "Chat"}
          </button>
        </div>
        <div className="mode-toggle">
          <button
            id="mode-toggle-btn"
            className="control-btn mode-btn"
            title={insertMode ? "Insert Mode (Click to switch to Replace)" : "Replace Mode (Click to switch to Insert)"}
            onClick={onModeToggle}
          >
            {insertMode ? "Ins" : "Repl"}
          </button>
          <button
            id="auto-copy-btn"
            className="control-btn mode-btn"
            title={rewriteEnabled ? "Rewrite enabled (Click to disable)" : "Rewrite disabled (Click to enable)"}
            onClick={onRewriteToggle}
            style={rewriteBtnStyle}
          >
            {rewriteEnabled ? "Re" : "No"}
          </button>
        </div>
        <div className="sidebar-controls">
          <button id="minimize-btn" className="control-btn" title="Minimize" onClick={onMinimize}>
            −
          </button>
          <button id="close-btn" className="control-btn" title="Close" onClick={onClose}>
            ✕
          </button>
        </div>
      </div>

      {/* Right Content Area */}
      <div className="content-area">
        <textarea
          id="transcription-text"
          className="transcription-textbox"
          placeholder="Transcribed text will appear here..."
          spellCheck={false}
          ref={textareaRef}
          value={transcriptionText}
          onChange={onTextareaChange}
          onMouseDown={(e) => e.stopPropagation()}
        ></textarea>
      </div>
    </>
  );
}
