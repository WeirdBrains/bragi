// Video in the chat panel (LF-43, V2): which files are clips, what the webview's player gets, and
// why it falls back to the system player. Pure functions, so they are tested without a webview.
// Sampling stays in bragi-core behind `bragi serve`; the extension never runs ffmpeg.

import * as path from 'path';

/** Extensions offered in the attach dialog and accepted on a drop. */
export const VIDEO_EXTENSIONS = ['mp4', 'm4v', 'mov', 'webm', 'mkv', 'avi', 'ogv'];

/** What VS Code's webview (Chromium without proprietary codecs in some builds) usually plays. */
const WEBVIEW_PLAYS: Record<string, string> = { mp4: 'video/mp4', m4v: 'video/mp4', webm: 'video/webm', ogv: 'video/ogg', mov: 'video/mp4' };

export function extensionOf(p: string): string {
  return path.extname(p).slice(1).toLowerCase();
}

export function isVideoPath(p: string): boolean {
  return VIDEO_EXTENSIONS.includes(extensionOf(p));
}

/** The `<video>` type for a clip the webview may play, else undefined (open it outside instead). */
export function webviewType(p: string): string | undefined {
  return WEBVIEW_PLAYS[extensionOf(p)];
}

/** One frame the model saw, as `bragi serve`'s `video` event carries it. */
export interface VideoFrame {
  at_ms: number;
  label: string;
  mime: string;
  /** Base64. */
  data: string;
}

/** `bragi serve`'s `video` event (crates/bragi-serve/src/video.rs). */
export interface VideoEvent {
  type: 'video';
  name: string;
  bubble: string;
  summary: string;
  duration_ms: number;
  sound: boolean;
  clip?: string | null;
  frames: VideoFrame[];
}

/** Why the panel can't play a clip itself; each says what to do. */
export type Fallback = 'missing' | 'format' | 'error';

export function fallbackMessage(f: Fallback): string {
  switch (f) {
    case 'missing':
      return "The clip isn't on this machine any more, so it can't be played. The frames the model saw are below.";
    case 'format':
      return "VS Code can't play this kind of file. Open it in your system player.";
    case 'error':
      return "VS Code couldn't play this clip (its codec, most likely). Open it in your system player.";
  }
}

/** What the webview draws for a clip: the bubble, the player (when it can play) and the frames. */
export interface PlayerMessage {
  type: 'video';
  name: string;
  bubble: string;
  /** The clip's path, for "Open in system player"; absent when it isn't on disk. */
  clip?: string;
  /** The `<video>` type; absent when the panel won't try to play it. */
  mime?: string;
  /** A webview URI for `<video src>`, filled in when the message reaches the panel. */
  src?: string;
  fallback?: Fallback;
  frames: { at_ms: number; label: string; src: string }[];
}

/** The player message for `e`; `exists` says whether the clip is on disk. */
export function playerMessage(e: VideoEvent, exists: (p: string) => boolean): PlayerMessage {
  const frames = (e.frames ?? [])
    .filter((f) => /^image\/(jpeg|png|webp)$/.test(f.mime) && /^[A-Za-z0-9+/=]*$/.test(f.data))
    .map((f) => ({ at_ms: f.at_ms, label: f.label, src: `data:${f.mime};base64,${f.data}` }));
  const base = { type: 'video' as const, name: e.name, bubble: e.bubble, frames };
  const clip = e.clip ?? undefined;
  if (!clip || !path.isAbsolute(clip) || !exists(clip)) {
    return { ...base, fallback: 'missing' };
  }
  const mime = webviewType(clip);
  return mime ? { ...base, clip, mime } : { ...base, clip, fallback: 'format' };
}

/** `m` with the webview URI of its clip, when the panel plays it. */
export function withSource(m: PlayerMessage, toWebview: (p: string) => string): PlayerMessage {
  return m.clip && m.mime && !m.fallback ? { ...m, src: toWebview(m.clip) } : m;
}

/** The folders the webview may read: the extension's media and each played clip's folder, once. */
export function resourceRoots(media: string, clips: Iterable<string>): string[] {
  const out = [media];
  for (const c of clips) {
    const d = path.dirname(c);
    if (!out.includes(d)) {
      out.push(d);
    }
  }
  return out;
}

/** The file a drop on the panel names (a `file:` URI from the explorer or the OS), if it's a clip. */
export function droppedClip(uriList: string): string | undefined {
  for (const line of uriList.split(/\r?\n/)) {
    const t = line.trim();
    if (!t || t.startsWith('#') || !t.startsWith('file:')) {
      continue;
    }
    try {
      const p = decodeURIComponent(new URL(t).pathname);
      const local = process.platform === 'win32' ? p.replace(/^\/([A-Za-z]:)/, '$1').replace(/\//g, '\\') : p;
      if (isVideoPath(local)) {
        return local;
      }
    } catch {
      // Not a URI: skip it.
    }
  }
  return undefined;
}
