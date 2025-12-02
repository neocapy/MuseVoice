import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface TranscriptionHistoryEntry {
  text: string;
  timestamp: number;
}

export default function History() {
  const [entries, setEntries] = useState<TranscriptionHistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);

  const loadHistory = useCallback(async () => {
    try {
      const history = await invoke<TranscriptionHistoryEntry[]>("get_transcription_history");
      setEntries(history);
    } catch (e) {
      console.error("Failed to load history:", e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadHistory();

    let unlistenFn: (() => void) | undefined;
    const setupListener = async () => {
      const unlisten = await listen<string>("transcription-result", () => {
        loadHistory();
      });
      return unlisten;
    };

    setupListener().then((fn) => {
      unlistenFn = fn;
    });

    return () => {
      if (unlistenFn) unlistenFn();
    };
  }, [loadHistory]);

  const handleCopy = async (index: number) => {
    try {
      await invoke("copy_history_entry", { index });
      setCopiedIndex(index);
      setTimeout(() => setCopiedIndex(null), 1500);
    } catch (e) {
      console.error("Failed to copy history entry:", e);
    }
  };

  const handleCopyAll = async () => {
    if (entries.length === 0) return;
    const allText = entries.map(e => e.text).join("\n\n");
    try {
      await invoke("copy_to_clipboard", { text: allText });
      setCopiedIndex(-1);
      setTimeout(() => setCopiedIndex(null), 1500);
    } catch (e) {
      console.error("Failed to copy all entries:", e);
    }
  };

  const handleClose = async () => {
    try {
      await invoke("close_history_window");
    } catch (e) {
      console.error("Failed to close history window:", e);
    }
  };

  const formatTimestamp = (ts: number) => {
    const date = new Date(ts * 1000);
    return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  };

  if (loading) {
    return (
      <div className="history-container">
        <div className="history-loading">Loading...</div>
      </div>
    );
  }

  return (
    <div className="history-container">
      <div className="history-header">
        <h1 className="history-title">Transcription History</h1>
        {entries.length > 0 && (
          <button 
            className="history-copy-all-btn"
            onClick={handleCopyAll}
          >
            {copiedIndex === -1 ? "Copied!" : "Copy All"}
          </button>
        )}
      </div>

      {entries.length === 0 ? (
        <div className="history-empty">
          <p>No transcriptions yet.</p>
          <p className="history-empty-hint">Transcriptions will appear here as you record.</p>
        </div>
      ) : (
        <div className="history-list">
          {[...entries].reverse().map((entry, idx) => {
            const originalIndex = entries.length - 1 - idx;
            return (
              <div key={originalIndex} className="history-entry">
                <div className="history-entry-header">
                  <span className="history-entry-time">{formatTimestamp(entry.timestamp)}</span>
                  <button
                    className="history-entry-copy-btn"
                    onClick={() => handleCopy(originalIndex)}
                  >
                    {copiedIndex === originalIndex ? "Copied!" : "Copy"}
                  </button>
                </div>
                <p className="history-entry-text">{entry.text}</p>
              </div>
            );
          })}
        </div>
      )}

      <div className="history-actions">
        <button className="settings-btn settings-btn-secondary" onClick={handleClose}>
          Close
        </button>
      </div>
    </div>
  );
}
