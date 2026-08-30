import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

export function parseChangedFileCounts(value) {
  const entries = value.split(',').map((entry) => entry.trim());
  const counts = [...new Set(entries.map((entry) => /^\d+$/.test(entry) ? Number(entry) : Number.NaN))];
  if (!counts.length || counts.some((count) => !Number.isSafeInteger(count) || count < 0)) {
    throw new Error('SCOPE_LOAD_CHANGED_FILE_COUNTS must be a comma-separated list of non-negative integers');
  }
  return counts.sort((left, right) => left - right);
}

export async function writeChangedFiles(directory, count, bytesPerFile, update) {
  if (![count, bytesPerFile, update].every(Number.isSafeInteger)
    || count < 0 || bytesPerFile < 1 || update < 0) {
    throw new Error('changed-file fixture values must be safe integers with positive file size');
  }
  await mkdir(directory, { recursive: true });
  const content = Buffer.alloc(bytesPerFile, 'x');
  const marker = Buffer.from(`scope-load-update:${update}\n`);
  marker.copy(content, 0, 0, Math.min(marker.length, content.length));
  for (let offset = 0; offset < count; offset += 64) {
    await Promise.all(Array.from({ length: Math.min(64, count - offset) }, (_, index) => writeFile(
      join(directory, `${String(offset + index).padStart(6, '0')}.txt`),
      content,
    )));
  }
}
