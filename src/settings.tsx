import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface RewritePrompt {
  id: string;
  name: string;
  text: string;
}

interface Options {
  model: string;
  rewrite_enabled: boolean;
  omit_final_punctuation: boolean;
  selected_prompt_id: string;
  custom_prompts: RewritePrompt[];
}

export default function Settings() {
  const [options, setOptions] = useState<Options>({
    model: "whisper-1",
    rewrite_enabled: false,
    omit_final_punctuation: false,
    selected_prompt_id: "default",
    custom_prompts: [],
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [expandedPromptId, setExpandedPromptId] = useState<string | null>(null);

  const allPrompts = options.custom_prompts.length > 0 
    ? options.custom_prompts 
    : [
        {
          id: "default",
          name: "Default (Built-in)",
          text: "Loading...",
        }
      ];

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
      const customPromptsOnly = options.custom_prompts.filter(p => p.id !== "default");
      
      await invoke("update_options", {
        patch: {
          model: options.model,
          rewrite_enabled: options.rewrite_enabled,
          omit_final_punctuation: options.omit_final_punctuation,
          selected_prompt_id: options.selected_prompt_id,
          custom_prompts: customPromptsOnly,
        },
      });

      await invoke("close_settings_window");
    } catch (e) {
      console.error("Failed to save options:", e);
      setSaving(false);
    }
  };

  const handleCancel = async () => {
    try {
      await invoke("close_settings_window");
    } catch (e) {
      console.error("Failed to close settings window:", e);
    }
  };

  const handleAddPrompt = () => {
    const newId = `custom-${Date.now()}`;
    const newPrompt: RewritePrompt = {
      id: newId,
      name: "New Prompt",
      text: "Enter your custom rewrite prompt here. Use {} as a placeholder for the transcribed text.",
    };
    setOptions({
      ...options,
      custom_prompts: [...options.custom_prompts, newPrompt],
    });
    setExpandedPromptId(newId);
  };

  const handleDeletePrompt = (id: string) => {
    const updatedPrompts = options.custom_prompts.filter(p => p.id !== id);
    setOptions({
      ...options,
      custom_prompts: updatedPrompts,
      selected_prompt_id: options.selected_prompt_id === id ? "default" : options.selected_prompt_id,
    });
    if (expandedPromptId === id) {
      setExpandedPromptId(null);
    }
  };

  const handleUpdatePrompt = (id: string, field: 'name' | 'text', value: string) => {
    const updatedPrompts = options.custom_prompts.map(p =>
      p.id === id ? { ...p, [field]: value } : p
    );
    setOptions({
      ...options,
      custom_prompts: updatedPrompts,
    });
  };

  const isDefaultPrompt = (id: string) => id === "default";

  if (loading) {
    return (
      <div className="settings-container">
        <div className="settings-loading">Loading...</div>
      </div>
    );
  }

  return (
    <div className="settings-container">
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

      {options.rewrite_enabled && (
        <div className="settings-section">
          <label className="settings-label">
            Active Rewrite Prompt
            <select
              className="settings-select"
              value={options.selected_prompt_id}
              onChange={(e) => setOptions({ ...options, selected_prompt_id: e.target.value })}
            >
              {allPrompts.map(prompt => (
                <option key={prompt.id} value={prompt.id}>
                  {prompt.name}
                </option>
              ))}
            </select>
          </label>

          <div className="prompts-list">
            {allPrompts.map(prompt => (
              <div key={prompt.id} className={`prompt-item ${isDefaultPrompt(prompt.id) ? 'prompt-item-default' : ''}`}>
                <div className="prompt-header">
                  <input
                    type="text"
                    className="prompt-name-input"
                    value={prompt.name}
                    onChange={(e) => handleUpdatePrompt(prompt.id, 'name', e.target.value)}
                    disabled={isDefaultPrompt(prompt.id)}
                  />
                  <div className="prompt-actions">
                    <button
                      className="prompt-expand-btn"
                      onClick={() => setExpandedPromptId(expandedPromptId === prompt.id ? null : prompt.id)}
                    >
                      {expandedPromptId === prompt.id ? '▼' : '▶'}
                    </button>
                    {!isDefaultPrompt(prompt.id) && (
                      <button
                        className="prompt-delete-btn"
                        onClick={() => handleDeletePrompt(prompt.id)}
                      >
                        Delete
                      </button>
                    )}
                  </div>
                </div>
                {expandedPromptId === prompt.id && (
                  <textarea
                    className="prompt-text-input"
                    value={prompt.text}
                    onChange={(e) => handleUpdatePrompt(prompt.id, 'text', e.target.value)}
                    disabled={isDefaultPrompt(prompt.id)}
                    rows={8}
                  />
                )}
              </div>
            ))}
          </div>

          <button className="add-prompt-btn" onClick={handleAddPrompt}>
            + Add New Prompt
          </button>
        </div>
      )}

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
