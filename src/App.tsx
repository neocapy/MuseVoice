import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

import doneSound from "./done.wav";
import boowompSound from "./sounds/boowomp.mp3";
import bambooHitSound from "./sounds/bamboo_hit.mp3";
import pipeSound from "./sounds/pipe.mp3";

import { StatusCanvas } from "./components/StatusCanvas";
import { useBackendListeners } from "./hooks/useBackendListeners";

type FrontendStatus = "loading" | "ready" | "recording" | "processing";
type Model = "whisper-1" | "gpt-4o-transcribe";


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

  return { text: processedText, adjustedPosition: insertPosition + positionAdjustment
 };
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

export default function App() {
  // State
  const [status, setStatus] = useState<FrontendStatus>("loading");
  const [rewriteEnabled, setRewriteEnabled] = useState<boolean>(false);
  const [model, setModel] = useState<Model>("gpt-4o-transcribe");
  const [retryVisible, setRetryVisible] = useState<boolean>(false);

  const [waveformBins, setWaveformBins] = useState<number[]>([]);
  const [waveformAvgRms, setWaveformAvgRms] = useState<number>(0);

  const [transcriptionText, setTranscriptionText] = useState<string>("");
  const [contextMenuVisible, setContextMenuVisible] = useState<boolean>(false);

  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
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

  // Initial status
  useEffect(() => {
    setStatus("ready");
  }, []);

  // Load options from backend once and subscribe to changes
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const opts = await invoke<{ model: Model; rewrite_enabled: boolean }>("get_options");
        if (opts) {
          setModel(opts.model);
          setRewriteEnabled(opts.rewrite_enabled);
        }
      } catch (e) {
        console.error("Failed to load options:", e);
      }

      try {
        unlisten = await listen("options-changed", (event) => {
          const payload = event.payload as any;
          const full = payload?.full as { model: Model; rewrite_enabled: boolean } | undefined;
          const patch = payload?.patch as Partial<{ model: Model; rewrite_enabled: boolean }> | undefined;

          // Prefer patch for minimal updates, fall back to full if needed
          if (patch) {
            if (typeof patch.rewrite_enabled === "boolean") setRewriteEnabled(patch.rewrite_enabled);
            if (patch.model) setModel(patch.model as Model);
          } else if (full) {
            setModel(full.model);
            setRewriteEnabled(full.rewrite_enabled);
          }
        });
      } catch (e) {
        console.error("Failed to subscribe to options-changed:", e);
      }
    })();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

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

  // Backend event listeners - pass insertMode=false since we're always replacing
  useBackendListeners({
    insertMode: false,
    transcriptionText,
    isExpanded: false,
    setStatus,
    setWaveformBins,
    setWaveformAvgRms,
    setTranscriptionText,
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

  // Main canvas click behavior (start/stop recording)
  const handleCanvasClick = useCallback(async () => {
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

  // Global Tab key triggers canvas click
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Tab") {
        e.preventDefault();
        handleCanvasClick();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [handleCanvasClick]);

  // Status label text
  // const statusLabelText = (() => {
  //   switch (status) {
  //     case "loading":
  //       return "Loading";
  //     case "ready":
  //       return "Ready";
  //     case "recording":
  //       return currentSamples !== null ? formatSampleCount(currentSamples) : "Rec";
  //     case "processing":
  //       return "Proc";
  //     default:
  //       return "Ready";
  //   }
  // })();

  // Context menu handlers
  // const onContextMenu = useCallback((e: React.MouseEvent) => {
  //   e.preventDefault();
  //   setContextMenuPos({ x: e.clientX, y: e.clientY });
  //   setContextMenuVisible(true);
  // }, []);

  const closeContextMenu = useCallback(() => {
    setContextMenuVisible(false);
  }, []);

  useEffect(() => {
    if (contextMenuVisible) {
      const handler = () => closeContextMenu();
      document.addEventListener("click", handler);
      return () => document.removeEventListener("click", handler);
    }
  }, [contextMenuVisible, closeContextMenu]);

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

  const onRewriteToggle = useCallback(async () => {
    try {
      await invoke("set_rewrite_enabled", { enabled: !rewriteEnabled });
    } catch (e) {
      console.error("Failed to toggle rewrite:", e);
    }
    closeContextMenu();
  }, [rewriteEnabled, closeContextMenu]);

  const onModelToggle = useCallback(async () => {
    const next = model === "whisper-1" ? "gpt-4o-transcribe" : "whisper-1";
    try {
      await invoke("set_transcription_model", { model: next });
    } catch (e) {
      console.error("Failed to toggle model:", e);
    }
    closeContextMenu();
  }, [model, closeContextMenu]);


  const onRetry = useCallback(async () => {
    try {
      setStatus("processing");
      await invoke<string>("retry_transcription", { origin: "click" });
    } catch (e) {
      console.error("Failed to retry transcription:", e);
      setStatus("ready");
    }
  }, []);

  // Helper to position buttons radially
  // heading in degrees: 0=top, 90=right, 180=bottom, 270=left
  const radialPosition = (heading: number, radius: number) => {
    const rad = (heading - 90) * (Math.PI / 180); // -90 to make 0 degrees = top
    const x = 50 + radius * Math.cos(rad); // 64 = center of 128px window
    const y = 50 + radius * Math.sin(rad);
    return { left: `${x}px`, top: `${y}px` };
  };

  return (
    <div className="circular-app">
      {/* Main canvas - full window */}
      <StatusCanvas
        status={status}
        waveformBins={waveformBins}
        waveformAvgRms={waveformAvgRms}
        dpr={dpr}
        onClick={handleCanvasClick}
      />

      {/* Minimize at heading 30 (top right) */}
      <button
        className="radial-button"
        style={radialPosition(25, 42)}
        onClick={onMinimize}
        title="Minimize"
      >
        −
      </button>

      {/* Close at heading 60 (top right) */}
      <button
        className="radial-button"
        style={radialPosition(65, 42)}
        onClick={onClose}
        title="Close"
      >
        ✕
      </button>

      {/* Drag handle at heading 135 (bottom right) */}
      <div
        className="radial-button drag-handle-btn"
        style={radialPosition(315, 42)}
        data-tauri-drag-region
        title="Drag to move"
      >
        ⋮⋮
      </div>

      {/* Rewrite toggle at heading 240 (left) */}
      <button
        className="radial-button"
        style={radialPosition(135, 42)}
        onClick={onRewriteToggle}
        title="Toggle Rewrite"
      >
        {rewriteEnabled ? "✍️" : "🥩"}
      </button>


      {/* Model toggle at heading 300 (bottom left) */}
      <button
        className="radial-button"
        style={radialPosition(245, 42)}
        onClick={onModelToggle}
        title="Toggle Model"
      >
        {model === "whisper-1" ? "Wh" : "4o"}
      </button>

      {/* Retry button (center overlay) */}
      {retryVisible && (
        <button className="retry-overlay-btn" onClick={onRetry}>
          ⟳
        </button>
      )}

      {/* Hidden textarea for backend compatibility */}
      <textarea
        ref={textareaRef}
        value={transcriptionText}
        onChange={(e) => setTranscriptionText(e.target.value)}
        style={{ display: "none" }}
      />
    </div>
  );
}
