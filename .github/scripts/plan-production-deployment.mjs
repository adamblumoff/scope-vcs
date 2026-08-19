#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const COMPONENTS = ["checksImage", "cache", "worker", "api", "web", "cli"];

function matchesScope(path, scope) {
  return scope.files.includes(path) || scope.prefixes.some((prefix) => path.startsWith(prefix));
}

export function classifyChanges(manifest, paths, requestedScope = "changed") {
  const selection = Object.fromEntries(COMPONENTS.map((component) => [component, false]));

  if (requestedScope !== "changed") {
    if (requestedScope === "all") {
      return Object.fromEntries(COMPONENTS.map((component) => [component, true]));
    }
    if (!COMPONENTS.includes(requestedScope)) {
      throw new Error(`Unknown deployment scope: ${requestedScope}`);
    }
    selection[requestedScope] = true;
    return selection;
  }

  for (const path of paths) {
    if (matchesScope(path, manifest.changeScopes.all)) {
      return Object.fromEntries(COMPONENTS.map((component) => [component, true]));
    }
    for (const component of COMPONENTS) {
      if (matchesScope(path, manifest.changeScopes[component])) selection[component] = true;
    }
  }

  return selection;
}

function changedPaths(base, head) {
  if (!base || /^0+$/.test(base)) {
    return execFileSync("git", ["ls-tree", "-r", "--name-only", head], { encoding: "utf8" })
      .split("\n")
      .filter(Boolean);
  }
  return execFileSync("git", ["diff", "--name-only", `${base}...${head}`], { encoding: "utf8" })
    .split("\n")
    .filter(Boolean);
}

function argument(name, fallback = "") {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1] ?? fallback;
}

function main() {
  const manifestPath = argument("--manifest", ".github/deployment-services.json");
  const requestedScope = argument("--scope", "changed");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const paths = requestedScope === "changed"
    ? changedPaths(argument("--base"), argument("--head", "HEAD"))
    : [];
  const selection = classifyChanges(manifest, paths, requestedScope);
  const outputPath = process.env.GITHUB_OUTPUT;
  const summaryPath = process.env.GITHUB_STEP_SUMMARY;

  for (const [component, selected] of Object.entries(selection)) {
    const outputName = component === "checksImage" ? "checks_image" : component;
    const line = `${outputName}=${selected}\n`;
    if (outputPath) appendFileSync(outputPath, line);
    else process.stdout.write(line);
  }

  if (summaryPath) {
    const selected = Object.entries(selection)
      .filter(([, value]) => value)
      .map(([component]) => component);
    appendFileSync(summaryPath, [
      "## Deployment plan",
      "",
      `Selected: ${selected.length > 0 ? selected.join(", ") : "none"}`,
      "",
      paths.length > 0 ? `Changed files considered: ${paths.length}` : `Requested scope: ${requestedScope}`,
      "",
    ].join("\n"));
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
