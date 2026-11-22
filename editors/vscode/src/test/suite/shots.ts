// Screenshots of the virtual X display the tests run in (xvfb-run). Only when BRAGI_SHOTS names a
// folder; never on a real desktop.

import { execFileSync } from 'child_process';
import * as path from 'path';

export function shot(name: string): void {
  const dir = process.env.BRAGI_SHOTS;
  if (!dir || !process.env.DISPLAY) {
    return;
  }
  execFileSync('import', ['-window', 'root', path.join(dir, `${name}.png`)], { timeout: 20_000 });
}
