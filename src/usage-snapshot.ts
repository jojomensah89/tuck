/** Reject snapshots requested before a newer event or a newer pull. */
export class UsageSnapshotGate {
  private revision = 0;
  private request = 0;
  changed(): void { this.revision++; }
  begin(): { revision: number; request: number } {
    return { revision: this.revision, request: ++this.request };
  }
  accepts(ticket: { revision: number; request: number }): boolean {
    return ticket.revision === this.revision && ticket.request === this.request;
  }
}
