import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

class MuseVoiceApp {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private statusLabel: HTMLLabelElement;
  private transcriptionTextbox: HTMLTextAreaElement;
  private closeBtn: HTMLButtonElement;
  private appContainer: HTMLDivElement;
  private isExpanded: boolean = true;
  
  // Status states
  private currentStatus: 'loading' | 'ready' | 'recording' | 'processing' = 'loading';

  constructor() {
    this.canvas = document.getElementById('status-canvas') as HTMLCanvasElement;
    this.statusLabel = document.getElementById('status-label') as HTMLLabelElement;
    this.transcriptionTextbox = document.getElementById('transcription-text') as HTMLTextAreaElement;
    this.closeBtn = document.getElementById('close-btn') as HTMLButtonElement;
    this.appContainer = document.querySelector('.app-container') as HTMLDivElement;
    
    this.ctx = this.canvas.getContext('2d')!;
    
    this.init();
  }

  private init(): void {
    this.setupEventListeners();
    this.drawStatusButton();
    this.setStatus('ready');
  }

  private setupEventListeners(): void {
    // Canvas click for record/pause
    this.canvas.addEventListener('click', () => this.handleCanvasClick());
    
    // Close button
    this.closeBtn.addEventListener('click', () => this.handleClose());
    
    // Textbox change events
    this.transcriptionTextbox.addEventListener('input', (e) => this.handleTextboxChange(e));
    this.transcriptionTextbox.addEventListener('keydown', (e) => this.handleTextboxKeydown(e));
    
    // Double-click sidebar to toggle expand/collapse
    document.querySelector('.sidebar')?.addEventListener('dblclick', () => this.toggleExpanded());
    
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

  private handleClose(): void {
    getCurrentWindow().close();
  }

  private handleTextboxChange(event: Event): void {
    const target = event.target as HTMLTextAreaElement;
    // Here you can add logic to handle text changes
    console.log('Text changed:', target.value);
  }

  private handleTextboxKeydown(event: KeyboardEvent): void {
    // Handle special key combinations
    if (event.ctrlKey || event.metaKey) {
      switch (event.key) {
        case 'a':
          // Allow Ctrl/Cmd+A for select all
          break;
        case 'c':
          // Allow Ctrl/Cmd+C for copy
          break;
        case 'v':
          // Allow Ctrl/Cmd+V for paste
          break;
        case 'Enter':
          // Ctrl/Cmd+Enter could trigger some action
          event.preventDefault();
          this.sendTranscriptionToActiveWindow();
          break;
      }
    }
  }

  private toggleExpanded(): void {
    this.isExpanded = !this.isExpanded;
    
    if (this.isExpanded) {
      this.appContainer.classList.remove('collapsed');
      this.appContainer.classList.add('expanded');
    } else {
      this.appContainer.classList.remove('expanded');
      this.appContainer.classList.add('collapsed');
    }
  }

  private startRecording(): void {
    this.setStatus('recording');
    // TODO: Implement actual recording logic
    console.log('Starting recording...');
  }

  private stopRecording(): void {
    this.setStatus('processing');
    // TODO: Implement stopping recording and sending to STT
    console.log('Stopping recording...');
    
    // Simulate processing delay
    setTimeout(() => {
      this.setStatus('ready');
      // TODO: Replace with actual transcription result
      this.updateTranscription('This is a sample transcription result.');
    }, 2000);
  }

  private sendTranscriptionToActiveWindow(): void {
    const text = this.transcriptionTextbox.value;
    if (text.trim()) {
      // TODO: Implement sending text to active window via keyboard events
      console.log('Sending to active window:', text);
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
        this.statusLabel.textContent = 'Recording';
        break;
      case 'processing':
        this.statusLabel.textContent = 'Processing';
        break;
    }
    
    this.drawStatusButton();
  }

  private drawStatusButton(): void {
    const size = 48;
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
    // Auto-expand if collapsed and there's new text
    if (!this.isExpanded && text.trim()) {
      this.toggleExpanded();
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
