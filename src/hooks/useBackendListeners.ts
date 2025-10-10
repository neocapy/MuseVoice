import { useEffect } from "react";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

type FrontendStatus = "loading" | "ready" | "recording" | "processing";
type RawChatMode = "raw" | "chat";
type FlowState = "idle" | "recording" | "processing" | "completed" | "error" | "cancelled";
type WaveformChunkPayload = { bins: number[]; avgRms?: number; avg_rms?: number };

interface UseBackendListenersProps {
  insertMode: boolean;
  rawChatMode: RawChatMode;
  transcriptionText: string;
  isExpanded: boolean;
  setStatus: (status: FrontendStatus) => void;
  setCurrentSamples: (samples: number) => void;
  setWaveformBins: (bins: number[]) => void;
  setWaveformAvgRms: (rms: number) => void;
  setTranscriptionText: (text: string) => void;
  setLayoutMode: (mode: "expanded" | "collapsed" | "h-collapsed") => void;
  setRetryVisible: (visible: boolean) => void;
  doneAudio: HTMLAudioElement;
  boowompAudio: HTMLAudioElement;
  bambooHitAudio: HTMLAudioElement;
  pipeAudio: HTMLAudioElement;
  copyToClipboard: (text: string) => Promise<void>;
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
  addSmartSpacing: (text: string, insertPosition: number, fullText: string) => { text: string; adjustedPosition: number };
  removeTrailingPunctuation: (text: string) => string;
}

const COLLAPSE_WIDTH = 72;
const COLLAPSE_HEIGHT = 72;

export function useBackendListeners({
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
}: UseBackendListenersProps) {
  useEffect(() => {
    let unsubs: UnlistenFn[] = [];
    let mounted = true;

    const lastAudioPlayTimeRef = new Map<string, number>();
    const audioPlayCountRef = new Map<string, number>();
    const AUDIO_DEBOUNCE_MS = 150;

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
            const lastPlayTime = lastAudioPlayTimeRef.get(soundFile) || 0;
            if (now - lastPlayTime < AUDIO_DEBOUNCE_MS) {
              return; // debounced
            }
            lastAudioPlayTimeRef.set(soundFile, now);
            audioPlayCountRef.set(soundFile, (audioPlayCountRef.get(soundFile) || 0) + 1);

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
  ]);
}