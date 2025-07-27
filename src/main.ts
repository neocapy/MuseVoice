import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface StatusResponse {
  state: 'idle' | 'recording' | 'transcribing';
  samples: number | null;
}

class MuseVoiceApp {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private statusLabel: HTMLLabelElement;
  private transcriptionTextbox: HTMLTextAreaElement;
  private closeBtn: HTMLButtonElement;
  private minimizeBtn: HTMLButtonElement;
  private appContainer: HTMLDivElement;
  private isExpanded: boolean = true;
  private dpr: number = window.devicePixelRatio || 1;
  private currentStatus: 'loading' | 'ready' | 'recording' | 'processing' = 'loading';
  private statusPollingInterval: number | null = null;
  private currentSamples: number | null = null;
  
  private SIDEBAR_WIDTH = 48;
  private COLLAPSE_WIDTH = 72;
  
  constructor() {
    this.canvas = document.getElementById('status-canvas') as HTMLCanvasElement;
    this.statusLabel = document.getElementById('status-label') as HTMLLabelElement;
    this.transcriptionTextbox = document.getElementById('transcription-text') as HTMLTextAreaElement;
    this.closeBtn = document.getElementById('close-btn') as HTMLButtonElement;
    this.minimizeBtn = document.getElementById('minimize-btn') as HTMLButtonElement;
    this.appContainer = document.querySelector('.app-container') as HTMLDivElement;
    
    this.ctx = this.canvas.getContext('2d')!;
    
    this.init();
  }

  private init(): void {
    this.setupEventListeners();
    this.setupCanvas();
    this.drawStatusButton();
    this.setStatus('ready');
    this.handleWindowResize(); // Initial check
    this.startStatusPolling();
  }

  private startStatusPolling(): void {
    // Poll status every 250ms
    this.statusPollingInterval = window.setInterval(async () => {
      try {
        const status: StatusResponse = await invoke('get_status');
        this.updateFromBackendStatus(status);
      } catch (error) {
        console.error('Failed to get status:', error);
        // If we can't get status, assume something is wrong and show ready state
        this.setStatus('ready');
        this.currentSamples = null;
      }
    }, 250);
  }

  private updateFromBackendStatus(status: StatusResponse): void {
    this.currentSamples = status.samples;
    
    switch (status.state) {
      case 'idle':
        this.setStatus('ready');
        break;
      case 'recording':
        this.setStatus('recording');
        break;
      case 'transcribing':
        this.setStatus('processing');
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
        // Cannot interact while processing
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

  private handleTextboxChange(event: Event): void {
    // Empty as requested
  }

  private handleTextboxKeydown(event: KeyboardEvent): void {
    // Empty as requested
  }

  private handleWindowResize(): void {
    const windowWidth = window.innerWidth;
    const shouldExpand = windowWidth >= this.COLLAPSE_WIDTH;
    
    if (shouldExpand !== this.isExpanded) {
      this.isExpanded = shouldExpand;
      
      if (this.isExpanded) {
        this.appContainer.classList.remove('collapsed');
        this.appContainer.classList.add('expanded');
      } else {
        this.appContainer.classList.remove('expanded');
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
    if (this.statusPollingInterval !== null) {
      clearInterval(this.statusPollingInterval);
      this.statusPollingInterval = null;
    }
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
    this.transcriptionTextbox.value = text;
    // Auto-expand if collapsed and there's new text (only if window is wide enough)
    if (!this.isExpanded && text.trim() && window.innerWidth >= this.COLLAPSE_WIDTH) {
      this.handleWindowResize(); // This will expand if window is wide enough
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
