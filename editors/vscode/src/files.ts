// Files the chat panel sends with a message: a file from disk (text, an image or a sound), or the
// current selection. Bragi checks them again (bragi-core's `Attachment::from_bytes`); this only
// reads them and refuses early what Bragi would refuse anyway.

import * as fs from 'fs';
import * as path from 'path';
import { BragiError } from './types';

export type FileKind = 'text' | 'image' | 'audio';

/** One file waiting to go with the next message. `text` or `data` (base64) is what Bragi gets. */
export interface ChatFile {
  name: string;
  kind: FileKind;
  text?: string;
  data?: string;
  /** For the panel: the image or sound as a `data:` URL. */
  src?: string;
}

/** The same limits as bragi-core's `attach` module. */
export const MAX_TEXT_BYTES = 256 * 1024;
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
export const MAX_AUDIO_BYTES = 10 * 1024 * 1024;

const IMAGES: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp' };
const AUDIO: Record<string, string> = { wav: 'audio/wav', mp3: 'audio/mpeg' };
export const MEDIA_EXTENSIONS = [...Object.keys(IMAGES), ...Object.keys(AUDIO)];

const kib = (n: number) => Math.ceil(n / 1024);

/** Reads `file` for the chat. Images are PNG, JPEG, GIF or WebP, sound WAV or MP3; anything else must be UTF-8 text. */
export function readChatFile(file: string, read: (p: string) => Buffer = (p) => fs.readFileSync(p)): ChatFile {
  const name = path.basename(file);
  const ext = path.extname(file).slice(1).toLowerCase();
  let bytes: Buffer;
  try {
    bytes = read(file);
  } catch (e) {
    throw new BragiError(`${name} can't be read (${e instanceof Error ? e.message : String(e)})`, 'bad_request');
  }
  const mime = IMAGES[ext] ?? AUDIO[ext];
  if (mime) {
    const image = ext in IMAGES;
    const max = image ? MAX_IMAGE_BYTES : MAX_AUDIO_BYTES;
    if (bytes.length > max) {
      throw new BragiError(`${name} is ${kib(bytes.length)} KiB; the most a message takes is ${kib(max)} KiB for ${image ? 'an image' : 'audio'}`, 'bad_request');
    }
    const data = bytes.toString('base64');
    return { name, kind: image ? 'image' : 'audio', data, src: `data:${mime};base64,${data}` };
  }
  if (bytes.length > MAX_TEXT_BYTES) {
    throw new BragiError(`${name} is ${kib(bytes.length)} KiB; the most a message takes is ${kib(MAX_TEXT_BYTES)} KiB of text`, 'bad_request');
  }
  const text = bytes.toString('utf8');
  if (bytes.includes(0) || text.includes('\uFFFD')) {
    throw new BragiError(`${name} is not a text file (images are PNG, JPEG, GIF or WebP, audio is WAV or MP3)`, 'bad_request');
  }
  return { name, kind: 'text', text };
}

/** The selection as an attachment: `selection in src/math.rs:3-5`. Lines are 1-based. */
export function selectionFile(relPath: string, startLine: number, endLine: number, text: string): ChatFile {
  if (!text.trim()) {
    throw new BragiError('Select some text first.', 'bad_request');
  }
  if (Buffer.byteLength(text) > MAX_TEXT_BYTES) {
    throw new BragiError(`The selection is ${kib(Buffer.byteLength(text))} KiB; the most a message takes is ${kib(MAX_TEXT_BYTES)} KiB of text`, 'bad_request');
  }
  const lines = startLine === endLine ? `${startLine}` : `${startLine}-${endLine}`;
  return { name: `selection in ${relPath}:${lines}`, kind: 'text', text };
}

/** What Bragi's `/v1/code/chat` takes for one file. */
export function wireFile(f: ChatFile): { name: string; text?: string; data?: string } {
  return f.kind === 'text' ? { name: f.name, text: f.text } : { name: f.name, data: f.data };
}

/** Text files inlined for a server that takes only text (the direct target), as Bragi renders them. */
export function inlineText(files: ChatFile[]): string {
  return files.map((f) => `${f.name}:\n\`\`\`\n${f.text ?? ''}\n\`\`\`\n`).join('');
}
