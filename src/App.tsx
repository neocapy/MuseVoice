import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import doneSound from "./done.wav";
import boowompSound from "./sounds/boowomp.mp3";
import bambooHitSound from "./sounds/bamboo_hit.mp3";
import pipeSound from "./sounds/pipe.mp3";

import { StatusCanvas } from "./components/StatusCanvas";
import { useBackendListeners } from "./hooks/useBackendListeners";

// Constants
const COLLAPSE_WIDTH = 72;
const COLLAPSE_HEIGHT = 72;

type FrontendStatus = "loading" | "ready" | "recording" | "processing";
type Model = "whisper-1" | "gpt-4o-transcribe";
type RawChatMode = "raw" | "chat";

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

  // Backend event listeners
  useBackendListeners({
    insertMode,
    rawChatMode,
    transcriptionText,
    isExpanded,
    setStatus,
    setCurrentSamples,
    setWaveformBins,
    setWaveformAvgRms,
    setTranscriptionText,
    setLayoutMode,
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
          <StatusCanvas
            status={status}
            waveformBins={waveformBins}
            waveformAvgRms={waveformAvgRms}
            dpr={dpr}
            onClick={handleMicClick}
          />
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
