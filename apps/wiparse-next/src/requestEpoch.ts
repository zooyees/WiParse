/** Latest read wins; changing owner invalidates even an A -> B -> A return. */
export class RequestEpoch {
  private scope = '';
  private epoch = 0;
  setScope(scope: string) { if (scope !== this.scope) { this.scope = scope; this.epoch++; } }
  capture(): number { return this.epoch; }
  begin(): number { return ++this.epoch; }
  isCurrent(ticket: number): boolean { return ticket === this.epoch; }
  invalidate() { this.epoch++; }
}
