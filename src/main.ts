import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import doneSound from "./done.wav";

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
  private retryBtn: HTMLButtonElement;
  private appContainer: HTMLDivElement;
  private isExpanded: boolean = true;
  private dpr: number = window.devicePixelRatio || 1;
  private currentStatus: 'loading' | 'ready' | 'recording' | 'processing' = 'loading';
  private insertMode: boolean = false; // false = replace mode, true = insert mode
  private autoCopyEnabled: boolean = true;
  private doneAudio: HTMLAudioElement;

  private currentSamples: number | null = null;
  
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
    this.retryBtn = document.getElementById('retry-btn') as HTMLButtonElement;
    this.appContainer = document.querySelector('.app-container') as HTMLDivElement;
    
    this.ctx = this.canvas.getContext('2d')!;
    
    // Initialize audio
    this.doneAudio = new Audio(doneSound);
    this.doneAudio.preload = 'auto';
    
    this.init();
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

  private async handleRetryClick(): Promise<void> {
    try {
      this.setStatus('processing');
      const result: string = await invoke('retry_transcription');
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
      const result: string = await invoke('start_audio_stream');
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
      const result: string = await invoke('stop_audio_stream');
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
      const result: string = await invoke('cancel_transcription');
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
    
    // Draw icon based on status
    this.ctx.fillStyle = 'white';
    this.ctx.strokeStyle = 'white';
    this.ctx.lineWidth = 2;
    
    if (this.currentStatus === 'ready') {
      // Draw microphone icon
      this.drawMicrophoneIcon(center);
    } else if (this.currentStatus === 'recording') {
      // Draw stop square
      this.ctx.fillRect(center - 6, center - 6, 12, 12);
    } else if (this.currentStatus === 'processing') {
      // Draw spinning dots or processing indicator
      this.drawProcessingIcon(center);
    }
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
    if (this.insertMode) {
      // Insert mode: insert at current cursor position with smart spacing
      const currentText = this.transcriptionTextbox.value;
      const cursorPosition = this.transcriptionTextbox.selectionStart || 0;
      
      // Apply smart spacing
      const { text: processedText, adjustedPosition } = this.addSmartSpacing(text, cursorPosition, currentText);
      
      // Insert the text at cursor position
      const beforeCursor = currentText.substring(0, cursorPosition);
      const afterCursor = currentText.substring(cursorPosition);
      const newText = beforeCursor + processedText + afterCursor;
      
      this.transcriptionTextbox.value = newText;
      
      // Position cursor at end of inserted text
      const newCursorPosition = adjustedPosition + processedText.length;
      this.transcriptionTextbox.setSelectionRange(newCursorPosition, newCursorPosition);
    } else {
      // Replace mode: replace all text (original behavior)
      this.transcriptionTextbox.value = text;
      // Position cursor at end of text
      const endPosition = text.length;
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
