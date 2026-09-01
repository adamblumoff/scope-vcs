#!/usr/bin/env node

import { appendFileSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

import { COMPONENTS } from "./plan-production-deployment.mjs";

const API_VERSION = "2022-11-28";
const SOURCE_SHA_PATTERN = /^[0-9a-f]{40}$/;

function argument(name, fallback = "") {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1] ?? fallback;
}

async function githubRequest(path, options = {}, fetchImpl = fetch) {
  const token = process.env.GITHUB_TOKEN;
  const repository = process.env.GITHUB_REPOSITORY;
  if (!token || !repository) throw new Error("GITHUB_TOKEN and GITHUB_REPOSITORY are required");

  const response = await fetchImpl(`https://api.github.com/repos/${repository}${path}`, {
    ...options,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": API_VERSION,
      ...options.headers,
    },
  });
  if (!response.ok) {
    throw new Error(`GitHub API ${response.status} for ${path}: ${await response.text()}`);
  }
  return response.json();
}

function deploymentEvidence(component, deployment) {
  let payload = deployment.payload;
  if (typeof payload === "string") {
    try {
      payload = JSON.parse(payload);
    } catch {
      return null;
    }
  }
  if (
    payload?.component !== component
    || payload.sourceSha !== deployment.sha
    || !SOURCE_SHA_PATTERN.test(payload.sourceSha ?? "")
    || typeof payload.provider !== "string"
    || payload.provider.length === 0
    || typeof payload.evidenceId !== "string"
    || payload.evidenceId.length === 0
  ) {
    return null;
  }
  return {
    sourceSha: payload.sourceSha,
    provider: payload.provider,
    evidenceId: payload.evidenceId,
  };
}

export async function latestSuccessfulDeployments(fetchImpl = fetch) {
  const entries = await Promise.all(COMPONENTS.map(async (component) => {
    const environment = encodeURIComponent(`production/${component}`);
    const deployments = await githubRequest(
      `/deployments?environment=${environment}&per_page=100`,
      {},
      fetchImpl,
    );

    for (const deployment of deployments) {
      const statuses = await githubRequest(
        `/deployments/${deployment.id}/statuses?per_page=100`,
        {},
        fetchImpl,
      );
      if (statuses.some(({ state }) => state === "success")) {
        const evidence = deploymentEvidence(component, deployment);
        if (evidence) return [component, evidence];
      }
    }
    return [component, null];
  }));

  return Object.fromEntries(entries);
}

export async function latestSuccessfulRevisions(fetchImpl = fetch) {
  const deployments = await latestSuccessfulDeployments(fetchImpl);
  return Object.fromEntries(COMPONENTS.map((component) => [
    component,
    deployments[component]?.sourceSha ?? null,
  ]));
}

export async function recordSuccessfulDeployment({
  component,
  sourceSha,
  provider,
  evidenceId,
  logUrl = "",
}, fetchImpl = fetch) {
  if (!COMPONENTS.includes(component)) throw new Error(`Unknown deployment component: ${component}`);
  if (!SOURCE_SHA_PATTERN.test(sourceSha)) throw new Error("sourceSha must be a full lowercase commit SHA");
  if (!provider || !evidenceId) throw new Error("provider and evidenceId are required");

  const environment = `production/${component}`;
  const deployment = await githubRequest("/deployments", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      ref: sourceSha,
      auto_merge: false,
      required_contexts: [],
      environment,
      production_environment: true,
      transient_environment: false,
      description: `Deploy ${component} from ${sourceSha.slice(0, 12)}`,
      payload: { component, sourceSha, provider, evidenceId },
    }),
  }, fetchImpl);

  await githubRequest(`/deployments/${deployment.id}/statuses`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      state: "success",
      environment,
      auto_inactive: false,
      description: `${provider} deployment ${evidenceId}`.slice(0, 140),
      ...(logUrl ? { log_url: logUrl } : {}),
    }),
  }, fetchImpl);

  return deployment.id;
}

export async function recordEvidenceFile(path, logUrl = "", fetchImpl = fetch) {
  let contents;
  try {
    contents = readFileSync(path, "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }

  const records = contents.split(/\r?\n/).filter(Boolean).map((line) => JSON.parse(line));
  const deploymentIds = [];
  for (const record of records) {
    deploymentIds.push(await recordSuccessfulDeployment({ ...record, logUrl }, fetchImpl));
  }
  return deploymentIds;
}

async function main() {
  const command = process.argv[2];
  if (command === "read") {
    const deployments = await latestSuccessfulDeployments();
    const revisions = Object.fromEntries(COMPONENTS.map((component) => [
      component,
      deployments[component]?.sourceSha ?? null,
    ]));
    if (process.env.GITHUB_OUTPUT) {
      appendFileSync(process.env.GITHUB_OUTPUT, `revisions=${JSON.stringify(revisions)}\n`);
      appendFileSync(process.env.GITHUB_OUTPUT, `deployments=${JSON.stringify(deployments)}\n`);
    } else {
      process.stdout.write(`${JSON.stringify({ deployments, revisions })}\n`);
    }
    return;
  }
  if (command === "record") {
    const deploymentId = await recordSuccessfulDeployment({
      component: argument("--component"),
      sourceSha: argument("--source-sha"),
      provider: argument("--provider"),
      evidenceId: argument("--evidence-id"),
      logUrl: argument("--log-url"),
    });
    process.stdout.write(`Recorded GitHub deployment ${deploymentId}.\n`);
    return;
  }
  if (command === "record-file") {
    const deploymentIds = await recordEvidenceFile(
      argument("--file"),
      argument("--log-url"),
    );
    process.stdout.write(`Recorded ${deploymentIds.length} GitHub deployment(s).\n`);
    return;
  }
  throw new Error("usage: production-deployment-progress.mjs <read|record|record-file> [options]");
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
