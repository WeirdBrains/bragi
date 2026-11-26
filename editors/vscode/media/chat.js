// The chat panel's page. It only draws what the extension sends and passes clicks back.
(function () {
  const vscode = acquireVsCodeApi();
  const log = document.getElementById('log');
  const input = document.getElementById('input');
  const send = document.getElementById('send');
  const stop = document.getElementById('stop');
  const ctx = document.getElementById('ctx');
  const target = document.getElementById('target');
  const clip = document.getElementById('clip');
  const clipName = document.getElementById('clip-name');
  const clipChip = document.getElementById('clip-chip');
  let answer = null;

  function busy(on) {
    send.hidden = on;
    stop.hidden = !on;
  }

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  function codeBlock(body) {
    const block = el('div', 'code');
    const bar = el('div', 'bar');
    const ins = el('button', '', 'Insert');
    ins.onclick = () => vscode.postMessage({ type: 'insert', code: body });
    const cp = el('button', '', 'Copy');
    cp.onclick = () => vscode.postMessage({ type: 'copy', code: body });
    bar.append(ins, cp);
    block.append(bar, el('pre', '', body));
    return block;
  }

  // Text with fenced code blocks (a fence may be longer than three backticks); each block gets
  // Insert and Copy. An unclosed block (still streaming) is drawn as code so far.
  function render(into, text) {
    into.textContent = '';
    let prose = [];
    let code = [];
    let fence = null;
    const flushProse = () => {
      const t = prose.join('\n');
      if (t.trim()) into.appendChild(el('div', 'text', t));
      prose = [];
    };
    const flushCode = () => {
      into.appendChild(codeBlock(code.length ? code.join('\n') + '\n' : ''));
      code = [];
    };
    for (const line of text.split('\n')) {
      if (fence === null) {
        const m = /^\s*(`{3,})/.exec(line);
        if (m) {
          flushProse();
          fence = m[1];
        } else {
          prose.push(line);
        }
      } else {
        const t = line.trim();
        if (t.startsWith(fence) && /^`+$/.test(t)) {
          flushCode();
          fence = null;
        } else {
          code.push(line);
        }
      }
    }
    if (fence !== null) flushCode();
    flushProse();
  }

  // Why the panel can't play a clip, matching src/video.ts fallbackMessage.
  const FALLBACK = {
    missing: "The clip isn't on this machine any more, so it can't be played. The frames the model saw are below.",
    format: "VS Code can't play this kind of file. Open it in your system player.",
    error: "VS Code couldn't play this clip (its codec, most likely). Open it in your system player.",
  };

  // A clip in the chat: its line, the player (or why not, with the system player), and the
  // frames the model saw; a click on one plays from there.
  function videoCard(m) {
    const card = el('div', 'video');
    card.appendChild(el('div', 'meta', m.bubble));
    let player = null;
    const why = el('div', 'fallback');
    why.hidden = true;
    const external = el('button', '', 'Open in system player');
    external.hidden = !m.clip;
    external.onclick = () => vscode.postMessage({ type: 'openClip', path: m.clip });
    const fallBack = (reason) => {
      why.textContent = FALLBACK[reason] || FALLBACK.error;
      why.hidden = false;
      if (player) {
        player.remove();
        player = null;
      }
    };
    if (m.src && !m.fallback) {
      player = document.createElement('video');
      player.controls = true;
      player.preload = 'metadata';
      const source = document.createElement('source');
      source.src = m.src;
      if (m.mime) source.type = m.mime;
      // A codec the webview lacks fails on the <source>, not the <video>.
      source.addEventListener('error', () => fallBack('error'));
      player.addEventListener('error', () => fallBack('error'));
      player.appendChild(source);
      card.appendChild(player);
    } else {
      fallBack(m.fallback || 'error');
    }
    card.append(why);
    const bar = el('div', 'bar');
    bar.appendChild(external);
    card.appendChild(bar);
    if (m.frames && m.frames.length) {
      card.appendChild(el('div', 'meta', 'Frames the model saw'));
      const strip = el('div', 'strip');
      for (const f of m.frames) {
        const b = el('button', 'frame');
        b.title = 'Play from ' + f.label;
        const img = document.createElement('img');
        img.src = f.src;
        img.alt = f.label;
        b.append(img, el('span', '', f.label));
        b.onclick = () => {
          if (!player) return;
          player.currentTime = f.at_ms / 1000;
          void player.play().catch(() => fallBack('error'));
        };
        strip.appendChild(b);
      }
      card.appendChild(strip);
    }
    return card;
  }

  function scroll() {
    log.scrollTop = log.scrollHeight;
  }

  function submit() {
    const prompt = input.value.trim();
    if (!prompt || !stop.hidden) return;
    input.value = '';
    vscode.postMessage({ type: 'send', prompt, context: ctx.checked });
  }

  send.onclick = submit;
  stop.onclick = () => vscode.postMessage({ type: 'stop' });
  document.getElementById('new').onclick = () => vscode.postMessage({ type: 'new' });
  target.onclick = () => vscode.postMessage({ type: 'pickModel' });
  document.getElementById('video').onclick = () => vscode.postMessage({ type: 'attachVideo' });
  document.getElementById('clip-remove').onclick = () => vscode.postMessage({ type: 'detachVideo' });
  document.getElementById('clip-limits').onclick = () => vscode.postMessage({ type: 'videoLimits' });
  // A clip dropped on the panel (from the OS, or the Explorer with Shift held).
  document.body.addEventListener('dragover', (e) => {
    e.preventDefault();
  });
  document.body.addEventListener('drop', (e) => {
    e.preventDefault();
    const dt = e.dataTransfer;
    if (!dt) return;
    const uris = dt.getData('text/uri-list') || dt.getData('application/vnd.code.uri-list') || '';
    vscode.postMessage({ type: 'dropVideo', uris });
  });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  });

  window.addEventListener('message', (event) => {
    const m = event.data;
    switch (m.type) {
      case 'user': {
        const u = el('div', 'msg user');
        u.appendChild(el('div', 'text', m.text));
        if (m.context) u.appendChild(el('div', 'meta', 'with ' + m.context));
        log.appendChild(u);
        answer = el('div', 'msg bot');
        answer.appendChild(el('div', 'meta', 'Waiting for a node…'));
        log.appendChild(answer);
        busy(true);
        scroll();
        break;
      }
      case 'start':
        if (answer) answer.firstChild.textContent = [m.model, m.node ? 'on ' + m.node.slice(0, 16) : ''].filter(Boolean).join(' ');
        break;
      case 'status':
        if (answer) answer.firstChild.textContent = m.text;
        break;
      case 'answer':
        if (answer) {
          let body = answer.querySelector('.body');
          if (!body) {
            body = el('div', 'body');
            answer.appendChild(body);
          }
          render(body, m.text);
          scroll();
        }
        break;
      case 'done':
        busy(false);
        answer = null;
        break;
      case 'error':
        if (answer) answer.appendChild(el('div', 'error', m.message));
        else log.appendChild(el('div', 'error', m.message));
        busy(false);
        answer = null;
        scroll();
        break;
      case 'reset':
        log.textContent = '';
        busy(false);
        answer = null;
        break;
      case 'target':
        target.textContent = m.label;
        break;
      case 'attached':
        clip.hidden = false;
        clipName.textContent = m.name;
        clipChip.textContent = m.chip || 'reading the model\'s video limits…';
        clipChip.classList.toggle('warn', m.ready === false);
        break;
      case 'detached':
        clip.hidden = true;
        break;
      case 'video': {
        // The clip was the user's: it goes in their message.
        const users = log.querySelectorAll('.msg.user');
        (users.length ? users[users.length - 1] : log).appendChild(videoCard(m));
        scroll();
        break;
      }
      case 'prefill':
        input.value = m.text;
        input.focus();
        break;
    }
  });

  vscode.postMessage({ type: 'ready' });
})();
