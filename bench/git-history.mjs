import { open } from 'node:fs/promises';

export async function writeLinearHistoryStream(path, baseOid, count) {
  const handle = await open(path, 'w');
  try {
    let chunk = '';
    for (let index = 0; index < count; index += 1) {
      const sequence = index + 2;
      const message = `History ${sequence}`;
      const content = `${sequence}\n`;
      chunk += `commit refs/heads/main\nauthor Scope Bench <bench@scope.local> 1700000000 +0000\ncommitter Scope Bench <bench@scope.local> 1700000000 +0000\ndata ${Buffer.byteLength(message)}\n${message}\n`;
      if (index === 0) chunk += `from ${baseOid.trim()}\n`;
      chunk += `M 100644 inline history.txt\ndata ${Buffer.byteLength(content)}\n${content}\n`;
      if (chunk.length >= 1024 * 1024) {
        await handle.write(chunk);
        chunk = '';
      }
    }
    await handle.write(`${chunk}done\n`);
  } finally {
    await handle.close();
  }
}
