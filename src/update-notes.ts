/** Release notes arrive as Markdown. Settings shows them as plain DOM text, never as HTML. */

export interface NotesBlock {
  kind: 'heading' | 'item' | 'text';
  text: string;
}

const VERSION_HEADING = /^#{1,2}\s/;
const HEADING = /^#{3,6}\s+(.+)$/;
const ITEM = /^[-*]\s+(.+)$/;

export function parseReleaseNotes(body: string): NotesBlock[] {
  const blocks: NotesBlock[] = [];
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || VERSION_HEADING.test(line)) continue;
    const heading = HEADING.exec(line);
    const item = ITEM.exec(line);
    if (heading) blocks.push({ kind: 'heading', text: heading[1] });
    else if (item) blocks.push({ kind: 'item', text: item[1] });
    else blocks.push({ kind: 'text', text: line });
  }
  return blocks;
}
