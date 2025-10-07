import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import doneSound from "./done.wav";
import boowompSound from "./sounds/boowomp.mp3";
import bambooHitSound from "./sounds/bamboo_hit.mp3";
import pipeSound from "./sounds/pipe.mp3";

interface StatusResponse {
  state: 'idle' | 'recording' | 'processing' | 'completed' | 'error' | 'cancelled';
  samples: number | null;
}

class MuseVoiceApp {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private statusLabel: HTMLLabelElement;
  private transcriptionTextbox: HTMLTextAreaElement;
  private closeBtn: HTMLButtonElement;
  private minimizeBtn: HTMLButtonElement;
  private modeToggleBtn: HTMLButtonElement;
  private autoCopyBtn: HTMLButtonElement;
  private modelToggleBtn: HTMLButtonElement;
  private rawChatToggleBtn: HTMLButtonElement;
  private retryBtn: HTMLButtonElement;
  private appContainer: HTMLDivElement;
  private isExpanded: boolean = true;
  private dpr: number = window.devicePixelRatio || 1;
  private currentStatus: 'loading' | 'ready' | 'recording' | 'processing' = 'loading';
  private insertMode: boolean = false; // false = replace mode, true = insert mode
  private autoCopyEnabled: boolean = true;
  private model: 'whisper-1' | 'gpt-4o-transcribe' = 'whisper-1';
  private rawChatMode: 'raw' | 'chat' = 'raw'; // raw = current behavior, chat = remove trailing punctuation
  private doneAudio: HTMLAudioElement;
  private boowompAudio: HTMLAudioElement;
  private bambooHitAudio: HTMLAudioElement;
  private pipeAudio: HTMLAudioElement;
  
  // Audio playback state tracking
  private lastAudioPlayTime: Map<string, number> = new Map();
  private audioPlayCount: Map<string, number> = new Map();
  private readonly AUDIO_DEBOUNCE_MS = 150; // Prevent rapid duplicate plays

  private currentSamples: number | null = null;
  private waveformBins: number[] = [];
  private waveformAvgRms: number = 0;
  
  private SIDEBAR_WIDTH = 48;
  private COLLAPSE_WIDTH = 72;
  private COLLAPSE_HEIGHT = 72;
  
  constructor() {
    this.canvas = document.getElementById('status-canvas') as HTMLCanvasElement;
    this.statusLabel = document.getElementById('status-label') as HTMLLabelElement;
    this.transcriptionTextbox = document.getElementById('transcription-text') as HTMLTextAreaElement;
    this.closeBtn = document.getElementById('close-btn') as HTMLButtonElement;
    this.minimizeBtn = document.getElementById('minimize-btn') as HTMLButtonElement;
    this.modeToggleBtn = document.getElementById('mode-toggle-btn') as HTMLButtonElement;
    this.autoCopyBtn = document.getElementById('auto-copy-btn') as HTMLButtonElement;
    this.modelToggleBtn = document.getElementById('model-toggle-btn') as HTMLButtonElement;
    this.rawChatToggleBtn = document.getElementById('raw-chat-toggle-btn') as HTMLButtonElement;
    this.retryBtn = document.getElementById('retry-btn') as HTMLButtonElement;
    this.appContainer = document.querySelector('.app-container') as HTMLDivElement;
    
    this.ctx = this.canvas.getContext('2d')!;
    
    // Initialize audio with enhanced loading
    this.doneAudio = this.createAudioElement(doneSound, 'done.wav');
    this.boowompAudio = this.createAudioElement(boowompSound, 'boowomp.mp3');
    this.bambooHitAudio = this.createAudioElement(bambooHitSound, 'bamboo_hit.mp3');
    this.pipeAudio = this.createAudioElement(pipeSound, 'pipe.mp3');
    
    this.init();
  }

  private createAudioElement(src: string, name: string): HTMLAudioElement {
    console.log(`🎵 Initializing audio element: ${name}`);
    const audio = new Audio(src);
    
    // Enhanced preloading settings
    audio.preload = 'auto';
    audio.volume = 1.0;
    
    // Add event listeners for debugging
    audio.addEventListener('loadstart', () => {
      console.log(`🔄 [${name}] Load started`);
    });
    
    audio.addEventListener('canplaythrough', () => {
      console.log(`✅ [${name}] Can play through (duration: ${audio.duration?.toFixed(2)}s)`);
    });
    
    audio.addEventListener('error', (e) => {
      console.error(`❌ [${name}] Audio error:`, e);
    });
    
    audio.addEventListener('loadeddata', () => {
      console.log(`📊 [${name}] Data loaded (readyState: ${audio.readyState})`);
    });
    
    // Trigger initial load
    audio.load();
    
    return audio;
  }

  private init(): void {
    this.setupEventListeners();
    this.setupCanvas();
    this.drawStatusButton();
    this.setStatus('ready');
    this.handleWindowResize(); // Initial check
    this.setupBackendEventListeners();
    this.checkInitialRetryData(); // Check if there's existing retry data
  }

  private async setupBackendEventListeners(): Promise<void> {
    try {
      // Import Tauri event system
      const { listen } = await import('@tauri-apps/api/event');
      
      // Listen for flow state changes
      await listen('flow-state-changed', (event: any) => {
        console.log('Flow state changed:', event.payload);
        this.updateFromBackendStatus({ state: event.payload, samples: null });
      });
      
      // Listen for sample count updates
      await listen('sample-count', (event: any) => {
        this.currentSamples = event.payload;
        if (this.currentStatus === 'recording') {
          this.setStatus('recording'); // This will update the display with new sample count
        }
      });

      // Listen for waveform chunks
      await listen('waveform-chunk', (event: any) => {
        const payload = event.payload as { bins: number[]; avgRms?: number; avg_rms?: number };
        if (!payload || !payload.bins) return;
        this.waveformBins = payload.bins;
        this.waveformAvgRms = (payload as any).avg_rms ?? payload.avgRms ?? 0;
        if (this.currentStatus === 'recording') {
          this.drawStatusButton();
        }
      });
      
      // Listen for transcription results
      await listen('transcription-result', (event: any) => {
        console.log('Transcription result:', event.payload);
        this.updateTranscription(event.payload);
        this.doneAudio.play().catch(e => console.error("Failed to play done sound:", e));
      });
      
      // Listen for flow errors
      await listen('flow-error', (event: any) => {
        console.error('Flow error:', event.payload);
        this.setStatus('ready');
        // Optionally show error to user
      });
      

      
      // Listen for retry availability changes
      await listen('retry-available', (event: any) => {
        console.log('Retry available:', event.payload);
        this.setRetryButtonVisibility(event.payload);
      });
      
      // Listen for audio feedback events
      await listen('audio-feedback', (event: any) => {
        console.log('Audio feedback:', event.payload);
        this.playAudioFeedback(event.payload).catch(error => {
          console.error('Audio feedback handler error:', error);
        });
      });
      
      console.log('Backend event listeners set up');
    } catch (error) {
      console.error('Failed to set up backend event listeners:', error);
    }
  }

  private updateFromBackendStatus(status: StatusResponse): void {
    this.currentSamples = status.samples;
    
    // Handle state changes - map backend states to frontend states
    switch (status.state) {
      case 'idle':
        this.setStatus('ready');
        break;
      case 'recording':
        this.setStatus('recording');
        break;
      case 'processing':
        this.setStatus('processing');
        break;
      case 'completed':
        this.setStatus('ready');
        break;
      case 'error':
      case 'cancelled':
        this.setStatus('ready');
        break;
    }
  }

  private formatSampleCount(samples: number): string {
    if (samples >= 1000) {
      return Math.floor(samples / 1000) + 'k';
    }
    return samples.toString();
  }

  private setupCanvas(): void {
    const size = this.SIDEBAR_WIDTH;
    // Set canvas size accounting for DPR
    this.canvas.width = size * this.dpr;
    this.canvas.height = size * this.dpr;
    this.canvas.style.width = size + 'px';
    this.canvas.style.height = size + 'px';
    this.ctx.scale(this.dpr, this.dpr);
  }

  private setupEventListeners(): void {
    // Canvas click for record/pause
    this.canvas.addEventListener('click', () => this.handleCanvasClick());
    
    // Close button
    this.closeBtn.addEventListener('click', async () => await this.handleClose());
    
    // Minimize button
    this.minimizeBtn.addEventListener('click', async () => await this.handleMinimize());
    
    // Mode toggle button
    this.modeToggleBtn.addEventListener('click', () => this.handleModeToggle());
    
    // Auto-copy toggle button
    this.autoCopyBtn.addEventListener('click', () => this.handleAutoCopyToggle());
    // Model toggle button
    this.modelToggleBtn.addEventListener('click', () => this.handleModelToggle());
    // RAW/CHAT toggle button
    this.rawChatToggleBtn.addEventListener('click', () => this.handleRawChatToggle());
    
    // Retry button
    this.retryBtn.addEventListener('click', () => this.handleRetryClick());
    
    // Textbox change events (empty as requested)
    this.transcriptionTextbox.addEventListener('input', (e) => this.handleTextboxChange(e));
    this.transcriptionTextbox.addEventListener('keydown', (e) => this.handleTextboxKeydown(e));
    
    // Window resize to handle width-based collapse
    window.addEventListener('resize', () => this.handleWindowResize());
    
    // DPR change detection
    window.matchMedia(`(resolution: ${this.dpr}dppx)`).addEventListener('change', () => {
      this.dpr = window.devicePixelRatio || 1;
      this.setupCanvas();
      this.drawStatusButton();
    });
    
    // Prevent drag on textbox to allow text selection
    this.transcriptionTextbox.addEventListener('mousedown', (e) => {
      e.stopPropagation();
    });
    
    // Global Tab key handler to trigger microphone button
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Tab') {
        e.preventDefault(); // Cancel default Tab behavior
        this.handleCanvasClick(); // Trigger microphone button functionality
      }
    });
  }

  private handleCanvasClick(): void {
    switch (this.currentStatus) {
      case 'ready':
        this.startRecording();
        break;
      case 'recording':
        this.stopRecording();
        break;
      case 'processing':
        // Cancel transcription if clicked while processing
        this.cancelTranscription();
        break;
    }
  }

  private async handleClose(): Promise<void> {
    try {
      await getCurrentWindow().close();
    } catch (error) {
      console.error('Failed to close window:', error);
    }
  }

  private async handleMinimize(): Promise<void> {
    try {
      await getCurrentWindow().minimize();
    } catch (error) {
      console.error('Failed to minimize window:', error);
    }
  }

  private handleModeToggle(): void {
    this.insertMode = !this.insertMode;
    this.modeToggleBtn.textContent = this.insertMode ? 'Ins' : 'Repl';
    this.modeToggleBtn.title = this.insertMode ? 'Insert Mode (Click to switch to Replace)' : 'Replace Mode (Click to switch to Insert)';
  }

  private handleAutoCopyToggle(): void {
    this.autoCopyEnabled = !this.autoCopyEnabled;
    this.autoCopyBtn.textContent = this.autoCopyEnabled ? 'Clip' : 'Local';
    this.autoCopyBtn.title = this.autoCopyEnabled ? 'Auto-copy enabled (Click to disable)' : 'Auto-copy disabled (Click to enable)';
    
    // Update button styling to indicate state
    if (this.autoCopyEnabled) {
      this.autoCopyBtn.style.backgroundColor = 'rgba(99, 102, 241, 0.2)';
      this.autoCopyBtn.style.borderColor = 'rgba(99, 102, 241, 0.6)';
    } else {
      this.autoCopyBtn.style.backgroundColor = 'rgba(99, 102, 241, 0.05)';
      this.autoCopyBtn.style.borderColor = 'rgba(99, 102, 241, 0.2)';
    }
  }

  private handleModelToggle(): void {
    this.model = this.model === 'whisper-1' ? 'gpt-4o-transcribe' : 'whisper-1';
    this.modelToggleBtn.textContent = this.model === 'whisper-1' ? 'Whis' : '4o-t';
    this.modelToggleBtn.title = this.model;
    // Persist choice for session by informing backend
    invoke('set_transcription_model', { model: this.model }).catch((e) => {
      console.error('Failed to set model:', e);
    });
  }

  private handleRawChatToggle(): void {
    this.rawChatMode = this.rawChatMode === 'raw' ? 'chat' : 'raw';
    this.rawChatToggleBtn.textContent = this.rawChatMode === 'raw' ? 'Raw' : 'Chat';
    this.rawChatToggleBtn.title = this.rawChatMode === 'raw' ? 'Raw mode (Click to switch to Chat)' : 'Chat mode (Click to switch to Raw)';
  }

  private async handleRetryClick(): Promise<void> {
    try {
      this.setStatus('processing');
      const result: string = await invoke('retry_transcription', { origin: 'click' });
      console.log('Retry started:', result);
    } catch (error) {
      console.error('Failed to retry transcription:', error);
      this.setStatus('ready');
    }
  }

  private setRetryButtonVisibility(visible: boolean): void {
    this.retryBtn.style.display = visible ? 'flex' : 'none';
  }

  private async checkInitialRetryData(): Promise<void> {
    try {
      const hasRetryData: boolean = await invoke('has_retry_data');
      this.setRetryButtonVisibility(hasRetryData);
    } catch (error) {
      console.error('Failed to check initial retry data:', error);
    }
  }

  private async playAudioFeedback(soundFile: string): Promise<void> {
    const startTime = performance.now();
    const playId = `${soundFile}-${Date.now()}`;
    
    console.log(`🎵 [${playId}] Audio feedback request: ${soundFile}`);
    
    try {
      // Get audio element
      let audio: HTMLAudioElement;
      switch (soundFile) {
        case 'boowomp.mp3':
          audio = this.boowompAudio;
          break;
        case 'bamboo_hit.mp3':
          audio = this.bambooHitAudio;
          break;
        case 'pipe.mp3':
          audio = this.pipeAudio;
          break;
        default:
          console.warn(`❌ [${playId}] Unknown audio feedback sound: ${soundFile}`);
          return;
      }

      // Debouncing check
      const now = Date.now();
      const lastPlayTime = this.lastAudioPlayTime.get(soundFile) || 0;
      if (now - lastPlayTime < this.AUDIO_DEBOUNCE_MS) {
        console.log(`⏸️ [${playId}] Debounced (${now - lastPlayTime}ms ago), skipping`);
        return;
      }
      
      // Update tracking
      this.lastAudioPlayTime.set(soundFile, now);
      const playCount = (this.audioPlayCount.get(soundFile) || 0) + 1;
      this.audioPlayCount.set(soundFile, playCount);
      
      console.log(`🎯 [${playId}] Attempting play #${playCount} - Element state:`, {
        readyState: audio.readyState,
        paused: audio.paused,
        ended: audio.ended,
        currentTime: audio.currentTime,
        duration: audio.duration,
        networkState: audio.networkState
      });
      
      // Check if audio is ready
      if (audio.readyState < HTMLMediaElement.HAVE_ENOUGH_DATA) {
        console.log(`⏳ [${playId}] Audio not ready (readyState: ${audio.readyState}), waiting...`);
        
        // Wait for audio to be ready
        await new Promise<void>((resolve, reject) => {
          const timeout = setTimeout(() => {
            reject(new Error('Audio loading timeout'));
          }, 3000);
          
          const onCanPlay = () => {
            clearTimeout(timeout);
            audio.removeEventListener('canplaythrough', onCanPlay);
            audio.removeEventListener('error', onError);
            resolve();
          };
          
          const onError = (e: Event) => {
            clearTimeout(timeout);
            audio.removeEventListener('canplaythrough', onCanPlay);
            audio.removeEventListener('error', onError);
            reject(new Error(`Audio loading error: ${e}`));
          };
          
          audio.addEventListener('canplaythrough', onCanPlay);
          audio.addEventListener('error', onError);
          
          // Try loading if not already loaded
          if (audio.readyState === HTMLMediaElement.HAVE_NOTHING) {
            audio.load();
          }
        });
      }
      
      // Stop any current playback and reset
      if (!audio.paused) {
        console.log(`🛑 [${playId}] Stopping current playback`);
        audio.pause();
      }
      
      audio.currentTime = 0;
      console.log(`🔄 [${playId}] Reset audio, ready to play`);
      
      // Attempt to play with retry logic
      let playSuccess = false;
      let lastError: any = null;
      
      for (let attempt = 1; attempt <= 3; attempt++) {
        try {
          console.log(`▶️ [${playId}] Play attempt #${attempt}`);
          
          const playPromise = audio.play();
          if (playPromise !== undefined) {
            await playPromise;
          }
          
          playSuccess = true;
          const duration = performance.now() - startTime;
          console.log(`✅ [${playId}] Audio played successfully in ${duration.toFixed(1)}ms`);
          break;
          
        } catch (playError: any) {
          lastError = playError;
          console.warn(`⚠️ [${playId}] Play attempt #${attempt} failed:`, {
            name: playError.name,
            message: playError.message,
            code: playError.code
          });
          
          // Wait a bit before retry
          if (attempt < 3) {
            await new Promise(resolve => setTimeout(resolve, 50 * attempt));
          }
        }
      }
      
      if (!playSuccess) {
        const duration = performance.now() - startTime;
        console.error(`❌ [${playId}] All play attempts failed after ${duration.toFixed(1)}ms:`, lastError);
        
        // Try one last desperate attempt with a fresh load
        try {
          console.log(`🆘 [${playId}] Attempting recovery with fresh load`);
          audio.load();
          await new Promise(resolve => setTimeout(resolve, 100));
          await audio.play();
          console.log(`🎉 [${playId}] Recovery successful!`);
        } catch (recoveryError) {
          console.error(`💀 [${playId}] Recovery also failed:`, recoveryError);
        }
      }
      
    } catch (error: any) {
      const duration = performance.now() - startTime;
      console.error(`💥 [${playId}] Critical audio feedback error after ${duration.toFixed(1)}ms:`, {
        name: error.name,
        message: error.message,
        stack: error.stack
      });
    }
  }

  private async copyToClipboard(text: string): Promise<void> {
    if (!this.autoCopyEnabled || !text.trim()) {
      return;
    }
    
    try {
      // Use invoke to call a backend clipboard command
      await invoke('copy_to_clipboard', { text });
      console.log('Text copied to clipboard:', text.substring(0, 50) + (text.length > 50 ? '...' : ''));
    } catch (error) {
      console.error('Failed to copy text to clipboard:', error);
    }
  }

  private addSmartSpacing(text: string, insertPosition: number, fullText: string): { text: string, adjustedPosition: number } {
    // Characters that don't need a space after them
    const noSpaceAfter = new Set(['(', '[', '{', '"', "'", '`', ' ', '\n', '\t']);
    // Characters that don't need a space before them
    const noSpaceBefore = new Set([')', ']', '}', '.', ',', ';', ':', '!', '?', '"', "'", '`', ' ', '\n', '\t']);
    
    let processedText = text;
    let positionAdjustment = 0;
    
    // Check if we need a space before the insertion
    const charBefore = insertPosition > 0 ? fullText[insertPosition - 1] : '';
    const firstCharOfText = text.length > 0 ? text[0] : '';
    
    if (charBefore && 
        !noSpaceAfter.has(charBefore) && 
        !noSpaceBefore.has(firstCharOfText) && 
        firstCharOfText !== ' ') {
      processedText = ' ' + processedText;
      positionAdjustment += 1;
    }
    
    // Check if we need a space after the insertion
    const charAfter = insertPosition < fullText.length ? fullText[insertPosition] : '';
    const lastCharOfText = text.length > 0 ? text[text.length - 1] : '';
    
    if (charAfter && 
        !noSpaceBefore.has(charAfter) && 
        !noSpaceAfter.has(lastCharOfText) && 
        lastCharOfText !== ' ') {
      processedText = processedText + ' ';
    }
    
    return { 
      text: processedText, 
      adjustedPosition: insertPosition + positionAdjustment 
    };
  }

  private removeTrailingPunctuation(text: string): string {
    // Remove trailing punctuation (periods, exclamation points, question marks, etc.)
    // while preserving internal punctuation and whitespace structure
    return text.replace(/[.!?;,]*\s*$/, '').trimEnd();
  }

  private handleTextboxChange(_event: Event): void {
    // Copy to clipboard if auto-copy is enabled
    if (this.autoCopyEnabled) {
      this.copyToClipboard(this.transcriptionTextbox.value);
    }
  }

  private handleTextboxKeydown(_event: KeyboardEvent): void {
    // Empty as requested
  }

  private handleWindowResize(): void {
    const windowWidth = window.innerWidth;
    const windowHeight = window.innerHeight;
    
    const isHorizontalCollapsed = windowHeight < this.COLLAPSE_HEIGHT;
    const isVerticalCollapsed = !isHorizontalCollapsed && windowWidth < this.COLLAPSE_WIDTH;
    const shouldExpand = !isHorizontalCollapsed && !isVerticalCollapsed;
    
    if (shouldExpand !== this.isExpanded || isHorizontalCollapsed) {
      this.isExpanded = shouldExpand;
      
      if (shouldExpand) {
        this.appContainer.classList.remove('collapsed');
        this.appContainer.classList.remove('h-collapsed');
        this.appContainer.classList.add('expanded');
      } else if (isHorizontalCollapsed) {
        this.appContainer.classList.remove('expanded');
        this.appContainer.classList.remove('collapsed');
        this.appContainer.classList.add('h-collapsed');
      } else {
        this.appContainer.classList.remove('expanded');
        this.appContainer.classList.remove('h-collapsed');
        this.appContainer.classList.add('collapsed');
      }
    }
  }

  private async startRecording(): Promise<void> {
    try {
      this.setStatus('recording');
      const result: string = await invoke('start_audio_stream', { origin: 'click' });
      console.log('Recording started:', result);
    } catch (error) {
      console.error('Failed to start recording:', error);
      this.setStatus('ready');
      // Could show error to user here if needed
    }
  }

  private async stopRecording(): Promise<void> {
    try {
      this.setStatus('processing');
      const result: string = await invoke('stop_audio_stream', { origin: 'click' });
      console.log('Recording stopped:', result);
      // Status will be updated by polling to show transcribing -> ready
    } catch (error) {
      console.error('Failed to stop recording:', error);
      this.setStatus('ready');
      // Could show error to user here if needed
    }
  }

  private async cancelTranscription(): Promise<void> {
    try {
      const result: string = await invoke('cancel_transcription', { origin: 'click' });
      console.log('Transcription cancelled:', result);
      this.setStatus('ready');
    } catch (error) {
      console.error('Failed to cancel transcription:', error);
      // Still set to ready since user clicked to cancel
      this.setStatus('ready');
    }
  }

  public setStatus(status: typeof this.currentStatus): void {
    this.currentStatus = status;
    
    switch (status) {
      case 'loading':
        this.statusLabel.textContent = 'Loading';
        break;
      case 'ready':
        this.statusLabel.textContent = 'Ready';
        break;
      case 'recording':
        if (this.currentSamples !== null) {
          this.statusLabel.textContent = this.formatSampleCount(this.currentSamples);
        } else {
          this.statusLabel.textContent = 'Rec';
        }
        break;
      case 'processing':
        this.statusLabel.textContent = 'Proc';
        break;
    }
    
    this.drawStatusButton();
  }

  public destroy(): void {
    // Cleanup is now handled by the backend event system
    // No polling to clean up
  }

  private drawStatusButton(): void {
    const size = this.SIDEBAR_WIDTH;
    const center = size / 2;
    const radius = 16;
    
    // Clear canvas
    this.ctx.clearRect(0, 0, size, size);
    
    // Set colors based on status
    let fillColor: string;
    let strokeColor: string;
    
    switch (this.currentStatus) {
      case 'loading':
        fillColor = '#9ca3af';
        strokeColor = '#6b7280';
        break;
      case 'ready':
        fillColor = '#6366f1';
        strokeColor = '#4f46e5';
        break;
      case 'recording':
        fillColor = '#ef4444';
        strokeColor = '#dc2626';
        break;
      case 'processing':
        fillColor = '#f59e0b';
        strokeColor = '#d97706';
        break;
    }
    
    // Draw circle
    this.ctx.beginPath();
    this.ctx.arc(center, center, radius, 0, 2 * Math.PI);
    this.ctx.fillStyle = fillColor;
    this.ctx.fill();
    this.ctx.strokeStyle = strokeColor;
    this.ctx.lineWidth = 2;
    this.ctx.stroke();
    
    // Draw icon or waveform based on status
    if (this.currentStatus === 'ready') {
      this.ctx.fillStyle = 'white';
      this.ctx.strokeStyle = 'white';
      this.ctx.lineWidth = 2;
      // Draw microphone icon
      this.drawMicrophoneIcon(center);
    } else if (this.currentStatus === 'recording') {
      this.drawRecordingWaveform(center, radius);
    } else if (this.currentStatus === 'processing') {
      this.ctx.fillStyle = 'white';
      this.ctx.strokeStyle = 'white';
      this.ctx.lineWidth = 2;
      this.drawProcessingIcon(center);
    }
  }

  private drawRecordingWaveform(center: number, radius: number): void {
    const ctx = this.ctx;
    const bins = this.waveformBins && this.waveformBins.length > 0 ? this.waveformBins : new Array(128).fill(0);
    const rmsToDbScale = (r: number): number => {
      const eps = 1e-8;
      const db = 20 * Math.log10(Math.max(r, eps));
      const t = (db + 30) / 30; // -30 dB -> 0, 0 dB -> 1
      return Math.max(0, Math.min(1, t));
    };
    const avg = rmsToDbScale(this.waveformAvgRms || 0);

    // Colors
    const bgBase = { r: 243, g: 233, b: 233 }; // gray-100
    const bgHot = { r: 254, g: 182, b: 182 }; // red-200
    const ringBase = { r: 254, g: 202, b: 202 }; // red-200
    const ringHot = { r: 185, g: 28, b: 28 }; // red-700
    const mix = (a: number, b: number, t: number) => Math.round(a + (b - a) * t);
    const bg = `rgb(${mix(bgBase.r, bgHot.r, avg)}, ${mix(bgBase.g, bgHot.g, avg)}, ${mix(bgBase.b, bgHot.b, avg)})`;
    const ring = `rgb(${mix(ringBase.r, ringHot.r, avg)}, ${mix(ringBase.g, ringHot.g, avg)}, ${mix(ringBase.b, ringHot.b, avg)})`;

    // Redraw circle background with dynamic fill
    ctx.save();
    ctx.beginPath();
    ctx.arc(center, center, radius, 0, 2 * Math.PI);
    ctx.fillStyle = bg;
    ctx.fill();
    ctx.strokeStyle = ring;
    ctx.lineWidth = 2;
    ctx.stroke();

    // Clip to circle to avoid drawing outside
    ctx.clip();

    // Draw filled waveform as symmetric area around center
    const padding = 2; // keep inside border
    const innerR = radius - padding;
    const N = bins.length;
    const leftX = center - innerR;
    const width = innerR * 2;
    const step = width / Math.max(1, N - 1);
    const minHalfPx = 1; // ensure minimum 2px thickness total

    // Build path top edge
    ctx.beginPath();
    for (let i = 0; i < N; i++) {
      const x = leftX + i * step;
      const amp = rmsToDbScale(Math.max(bins[i], 0));
      const half = Math.max(minHalfPx, amp * innerR); // ensure minimum 2px thickness total
      const yTop = center - half;
      if (i === 0) ctx.moveTo(x, yTop);
      else ctx.lineTo(x, yTop);
    }
    // Bottom edge in reverse
    for (let i = N - 1; i >= 0; i--) {
      const x = leftX + i * step;
      const amp = rmsToDbScale(Math.max(bins[i], 0));
      const half = Math.max(minHalfPx, amp * innerR);
      const yBot = center + half;
      ctx.lineTo(x, yBot);
    }
    ctx.closePath();
    ctx.fillStyle = 'rgb(255, 0, 0)';
    ctx.globalAlpha = 0.9;
    ctx.fill();
    ctx.globalAlpha = 1.0;

    ctx.restore();
  }

  private drawMicrophoneIcon(center: number): void {
    // Simple microphone icon
    this.ctx.beginPath();
    this.ctx.roundRect(center - 3, center - 8, 6, 12, 2);
    this.ctx.fill();
    
    // Microphone stand
    this.ctx.beginPath();
    this.ctx.moveTo(center, center + 4);
    this.ctx.lineTo(center, center + 8);
    this.ctx.stroke();
    
    // Base
    this.ctx.beginPath();
    this.ctx.moveTo(center - 4, center + 8);
    this.ctx.lineTo(center + 4, center + 8);
    this.ctx.stroke();
  }

  private drawProcessingIcon(center: number): void {
    // Simple dots animation (static for now)
    const dotSize = 2;
    for (let i = 0; i < 3; i++) {
      this.ctx.beginPath();
      this.ctx.arc(center - 6 + i * 6, center, dotSize, 0, 2 * Math.PI);
      this.ctx.fill();
    }
  }

  public updateTranscription(text: string): void {
    // Clear retry data on successful transcription is now handled by backend
    
    // Apply CHAT mode post-processing if enabled
    let processedText = text;
    if (this.rawChatMode === 'chat') {
      processedText = this.removeTrailingPunctuation(text);
    }
    
    if (this.insertMode) {
      // Insert mode: insert at current cursor position with smart spacing
      const currentText = this.transcriptionTextbox.value;
      const cursorPosition = this.transcriptionTextbox.selectionStart || 0;
      
      // Apply smart spacing
      const { text: spacedText, adjustedPosition } = this.addSmartSpacing(processedText, cursorPosition, currentText);
      
      // Insert the text at cursor position
      const beforeCursor = currentText.substring(0, cursorPosition);
      const afterCursor = currentText.substring(cursorPosition);
      const newText = beforeCursor + spacedText + afterCursor;
      
      this.transcriptionTextbox.value = newText;
      
      // Position cursor at end of inserted text
      const newCursorPosition = adjustedPosition + spacedText.length;
      this.transcriptionTextbox.setSelectionRange(newCursorPosition, newCursorPosition);
    } else {
      // Replace mode: replace all text (original behavior)
      this.transcriptionTextbox.value = processedText;
      // Position cursor at end of text
      const endPosition = processedText.length;
      this.transcriptionTextbox.setSelectionRange(endPosition, endPosition);
    }
    
    // Auto-expand if collapsed and there's new text (only if window is large enough)
    if (
      !this.isExpanded &&
      text.trim() &&
      window.innerWidth >= this.COLLAPSE_WIDTH &&
      window.innerHeight >= this.COLLAPSE_HEIGHT
    ) {
      this.handleWindowResize(); // This will expand if window is wide enough
    }
    
    // Copy to clipboard if auto-copy is enabled
    if (this.autoCopyEnabled) {
      this.copyToClipboard(this.transcriptionTextbox.value);
    }
  }

  public getTranscriptionText(): string {
    return this.transcriptionTextbox.value;
  }

  public clearTranscription(): void {
    this.transcriptionTextbox.value = '';
  }
}

// Declare global app on window
declare global {
  interface Window {
    app: MuseVoiceApp;
  }
}

window.addEventListener("DOMContentLoaded", () => {
  window.app = new MuseVoiceApp();
});

// Export for external access if needed
export { MuseVoiceApp };
