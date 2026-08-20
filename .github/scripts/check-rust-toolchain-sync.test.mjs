import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  readToolchainFiles,
  validateRustToolchainSync,
} from "./check-rust-toolchain-sync.mjs";

test("live Rust pins match the root toolchain", () => {
  assert.deepEqual(validateRustToolchainSync(readToolchainFiles()), []);
});

test("a mismatched checks image fails with the replica name and versions", () => {
  const files = readToolchainFiles();
  const expectedVersion = files["rust-toolchain.toml"].match(
    /channel\s*=\s*"([^"]+)"/,
  )[1];
  files[".scope/images/checks/Dockerfile"] = readFileSync(
    new URL("fixtures/mismatched-checks.Dockerfile", import.meta.url),
    "utf8",
  ).replaceAll("1.98.0", expectedVersion);

  assert.deepEqual(validateRustToolchainSync(files), [
    `.scope/images/checks/Dockerfile: Rust base image must match Rust ${expectedVersion}; found 1.97.0`,
  ]);
});
