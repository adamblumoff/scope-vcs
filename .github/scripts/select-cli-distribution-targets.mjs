#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const PULL_REQUEST_TARGET = "x86_64-unknown-linux-gnu";

export function selectCliDistributionTargets(configuration, mode) {
  if (!Array.isArray(configuration?.targets)) {
    throw new Error("CLI distribution configuration must contain a targets array");
  }
  if (mode !== "pull-request" && mode !== "release") {
    throw new Error(`Unknown CLI distribution mode: ${mode}`);
  }

  const targets = mode === "release"
    ? configuration.targets
    : configuration.targets.filter(({ triple }) => triple === PULL_REQUEST_TARGET);
  if (targets.length === 0) {
    throw new Error(`CLI distribution mode ${mode} selected no targets`);
  }

  return {
    include: targets.map(({ label, runner, triple, artifact, binary, builder, smoke }) => ({
      label,
      runner,
      target: triple,
      artifact,
      binary,
      builder,
      smoke,
    })),
  };
}

function argument(name, fallback = "") {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1] ?? fallback;
}

function main() {
  const targetsPath = argument("--targets", "cli/distribution/targets.json");
  const mode = argument("--mode");
  const configuration = JSON.parse(readFileSync(targetsPath, "utf8"));
  process.stdout.write(`${JSON.stringify(selectCliDistributionTargets(configuration, mode))}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
