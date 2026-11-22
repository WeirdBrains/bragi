// The chat panel's page. It only draws what the extension sends and passes clicks back.
(function () {
  const vscode = acquireVsCodeApi();
  const log = document.getElementById('log');
  const input = document.getElementById('input');
  const send = document.getElementById('send');
  const stop = document.getElementById('stop');
  const ctx = document.getElementById('ctx');
  const target = document.getElementById('target');
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
      case 'prefill':
        input.value = m.text;
        input.focus();
        break;
    }
  });

  vscode.postMessage({ type: 'ready' });
})();
