export class FakeSpeechXhr extends EventTarget {
  static instances: FakeSpeechXhr[] = [];
  status = 200; statusText = 'OK'; responseText = '';
  sent = false; aborted = false;
  private listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();
  constructor() { super(); FakeSpeechXhr.instances.push(this); }
  open() {}
  setRequestHeader() {}
  send() { this.sent = true; }
  abort() { this.aborted = true; this.dispatchEvent(new Event('abort')); }
  override addEventListener(type: string, listener: EventListenerOrEventListenerObject | null, options?: AddEventListenerOptions | boolean): void {
    if (listener) { if (!this.listeners.has(type)) this.listeners.set(type, new Set()); this.listeners.get(type)!.add(listener); }
    super.addEventListener(type, listener, options);
  }
  override removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null, options?: EventListenerOptions | boolean): void {
    if (listener) this.listeners.get(type)?.delete(listener);
    super.removeEventListener(type, listener, options);
  }
  get listenerCount() { return [...this.listeners.values()].reduce((sum, listeners) => sum + listeners.size, 0); }
  complete(text: string) {
    this.responseText = JSON.stringify({ success: true, data: { text, provider: 'configured-asr', model: 'configured-model' } });
    this.dispatchEvent(new Event('load'));
  }
}
