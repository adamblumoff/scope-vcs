#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const ROOT_TOOLCHAIN = "rust-toolchain.toml";
const EXACT_VERSION = /^channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$/m;

function workflowReplica(path, count = 1) {
  return {
    path,
    label: "workflow toolchain",
    pattern: /toolchain:\s*([0-9]+\.[0-9]+\.[0-9]+)/g,
    count,
  };
}

const REPLICAS = [
  workflowReplica(".github/workflows/rust-workspace-checks.yml"),
  workflowReplica(".github/workflows/scope-api-ci.yml"),
  workflowReplica(".github/workflows/scope-cli-build.yml", 3),
  workflowReplica(".github/workflows/scope-integration-ci.yml"),
  workflowReplica(".github/workflows/scope-web-ci.yml"),
  workflowReplica(".github/workflows/scope-railway-staging.yml"),
  {
    path: ".scope/images/checks/Dockerfile",
    label: "Rust base image",
    pattern: /FROM\s+--platform=linux\/amd64\s+rust:([0-9]+\.[0-9]+\.[0-9]+)-bookworm@sha256:[0-9a-f]{64}/g,
    count: 2,
  },
  {
    path: ".scope/images/checks/Dockerfile",
    label: "RUSTUP_TOOLCHAIN",
    pattern: /RUSTUP_TOOLCHAIN=([0-9]+\.[0-9]+\.[0-9]+)-x86_64-unknown-linux-gnu/g,
    count: 1,
  },
  {
    path: ".scope/images/checks/Dockerfile",
    label: "rustc version assertion",
    pattern: /rustc --version[^\n]+?=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/g,
    count: 1,
  },
  {
    path: ".scope/runs/checks.yml",
    label: "RUSTUP_TOOLCHAIN",
    pattern: /RUSTUP_TOOLCHAIN:\s*([0-9]+\.[0-9]+\.[0-9]+)-x86_64-unknown-linux-gnu/g,
    count: 1,
  },
];

export const TOOLCHAIN_FILES = [
  ROOT_TOOLCHAIN,
  ...new Set(REPLICAS.map(({ path }) => path)),
];

export function readToolchainFiles(root = ".") {
  return Object.fromEntries(
    TOOLCHAIN_FILES.map((path) => [path, readFileSync(resolve(root, path), "utf8")]),
  );
}

export function validateRustToolchainSync(files) {
  const root = files[ROOT_TOOLCHAIN];
  const rootMatch = root?.match(EXACT_VERSION);
  if (!rootMatch) return ["rust-toolchain.toml must declare an exact stable channel"];

  const expected = rootMatch[1];
  const errors = [];

  for (const replica of REPLICAS) {
    const content = files[replica.path];
    if (content === undefined) {
      errors.push(`${replica.path}: file is missing`);
      continue;
    }

    const versions = [...content.matchAll(replica.pattern)].map((match) => match[1]);
    if (versions.length !== replica.count) {
      errors.push(
        `${replica.path}: expected ${replica.count} ${replica.label} pin(s), found ${versions.length}`,
      );
      continue;
    }

    const mismatches = [...new Set(versions.filter((version) => version !== expected))];
    if (mismatches.length > 0) {
      errors.push(
        `${replica.path}: ${replica.label} must match Rust ${expected}; found ${mismatches.join(", ")}`,
      );
    }
  }

  const dockerfile = files[".scope/images/checks/Dockerfile"] ?? "";
  const baseDigests = [
    ...dockerfile.matchAll(
      /FROM\s+--platform=linux\/amd64\s+rust:[^\s@]+@(sha256:[0-9a-f]{64})/g,
    ),
  ].map((match) => match[1]);
  if (baseDigests.length === 2 && new Set(baseDigests).size !== 1) {
    errors.push(".scope/images/checks/Dockerfile: Rust stages must use the same base digest");
  }

  return errors;
}

function main() {
  const files = readToolchainFiles();
  const errors = validateRustToolchainSync(files);
  if (errors.length > 0) {
    process.stderr.write(`Rust toolchain replicas are out of sync:\n- ${errors.join("\n- ")}\n`);
    process.exitCode = 1;
    return;
  }

  const version = files[ROOT_TOOLCHAIN].match(EXACT_VERSION)[1];
  process.stdout.write(`Rust toolchain replicas match ${version}.\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
