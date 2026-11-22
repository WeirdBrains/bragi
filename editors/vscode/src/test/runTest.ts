// Runs the extension tests in a downloaded VS Code with a throwaway profile (its own user data
// and extensions folders), never the user's VS Code or Cursor profile. On Linux run it under
// xvfb-run with a clean environment (see editors/vscode/README.md).

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { runTests } from '@vscode/test-electron';

async function main(): Promise<void> {
  const root = path.resolve(__dirname, '../../');
  const scratch = fs.mkdtempSync(path.join(process.env.BRAGI_TEST_TMP || os.tmpdir(), 'bragi-vscode-test-'));
  try {
    await runTests({
      version: process.env.BRAGI_VSCODE_VERSION || 'stable',
      cachePath: path.join(root, '.vscode-test'),
      extensionDevelopmentPath: root,
      extensionTestsPath: path.resolve(__dirname, './suite/index'),
      launchArgs: [
        '--disable-extensions',
        '--disable-gpu',
        '--disable-workspace-trust',
        '--skip-welcome',
        '--skip-release-notes',
        `--user-data-dir=${path.join(scratch, 'user')}`,
        `--extensions-dir=${path.join(scratch, 'extensions')}`,
      ],
      extensionTestsEnv: { BRAGI_TEST_TMP: scratch, BRAGI_SHOTS: process.env.BRAGI_SHOTS },
    });
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
}

main().catch((e) => {
  console.error('extension tests failed:', e);
  process.exit(1);
});
