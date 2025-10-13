import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface Options {
  model: string;
  rewrite_enabled: boolean;
  omit_final_punctuation: boolean;
}

export default function Settings() {
  const [options, setOptions] = useState<Options>({
    model: "whisper-1",
    rewrite_enabled: false,
    omit_final_punctuation: false,
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    const loadOptions = async () => {
      try {
        const opts = await invoke<Options>("get_options");
        setOptions(opts);
      } catch (e) {
        console.error("Failed to load options:", e);
      } finally {
        setLoading(false);
      }
    };

    loadOptions();

    const setupListener = async () => {
      const unlisten = await listen<{ full: Options }>("options-changed", (event) => {
        setOptions(event.payload.full);
      });
      return unlisten;
    };

    let unlistenFn: (() => void) | undefined;
    setupListener().then((fn) => {
      unlistenFn = fn;
    });

    return () => {
      if (unlistenFn) unlistenFn();
    };
  }, []);

  const handleSave = async () => {
    setSaving(true);
    try {
      await invoke("update_options", {
        patch: {
          model: options.model,
          rewrite_enabled: options.rewrite_enabled,
          omit_final_punctuation: options.omit_final_punctuation,
        },
      });

      setTimeout(() => {
        getCurrentWindow().close();
      }, 200);
    } catch (e) {
      console.error("Failed to save options:", e);
      setSaving(false);
    }
  };

  const handleCancel = () => {
    getCurrentWindow().close();
  };

  if (loading) {
    return (
      <div className="settings-container">
        <div className="settings-loading">Loading...</div>
      </div>
    );
  }

  return (
    <div className="settings-container">
      <h1 className="settings-title">Settings</h1>

      <div className="settings-section">
        <label className="settings-label">
          Transcription Model
          <select
            className="settings-select"
            value={options.model}
            onChange={(e) => setOptions({ ...options, model: e.target.value })}
          >
            <option value="whisper-1">Whisper</option>
            <option value="gpt-4o-transcribe">GPT-4o Transcribe</option>
          </select>
        </label>
      </div>

      <div className="settings-section">
        <label className="settings-checkbox-label">
          <input
            type="checkbox"
            className="settings-checkbox"
            checked={options.omit_final_punctuation}
            onChange={(e) =>
              setOptions({ ...options, omit_final_punctuation: e.target.checked })
            }
          />
          <span>Omit Final Punctuation</span>
        </label>
        <p className="settings-hint">Remove trailing punctuation from transcriptions</p>
      </div>

      <div className="settings-section">
        <label className="settings-checkbox-label">
          <input
            type="checkbox"
            className="settings-checkbox"
            checked={options.rewrite_enabled}
            onChange={(e) =>
              setOptions({ ...options, rewrite_enabled: e.target.checked })
            }
          />
          <span>Rewrite Transcribed Text</span>
        </label>
        <p className="settings-hint">
          Use GPT-5 to fix dictation artifacts, phonetic spelling, and formatting commands
        </p>
      </div>

      <div className="settings-actions">
        <button className="settings-btn settings-btn-secondary" onClick={handleCancel}>
          Cancel
        </button>
        <button
          className="settings-btn settings-btn-primary"
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>
    </div>
  );
}

 
