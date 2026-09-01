#!/usr/bin/env node

import { pathToFileURL } from "node:url";

export const RAILWAY_COMPONENTS = ["cache", "worker", "router", "api", "web", "cli"];

const SOURCE_SHA_PATTERN = /^[0-9a-f]{40}$/;

export function assertHealthyRailwayService(services, serviceId, expectedDeploymentId = "") {
  if (!Array.isArray(services)) throw new Error("Railway service state must be an array");
  const service = services.find(({ id, name }) => id === serviceId || name === serviceId);
  if (!service) throw new Error(`Railway service ${serviceId} is missing`);

  const replicas = service.replicas ?? {};
  if (service.status !== "SUCCESS") {
    throw new Error(`Railway service ${serviceId} is ${service.status || "UNKNOWN"}`);
  }
  if (service.deploymentStopped === true) {
    throw new Error(`Railway service ${serviceId} is stopped`);
  }
  if (!Number.isInteger(replicas.configured) || replicas.configured <= 0) {
    throw new Error(`Railway service ${serviceId} has no configured replicas`);
  }
  if (replicas.running !== replicas.configured) {
    throw new Error(
      `Railway service ${serviceId} has ${replicas.running ?? 0}/${replicas.configured} running replicas`,
    );
  }
  if ((replicas.crashed ?? 0) !== 0) {
    throw new Error(`Railway service ${serviceId} has ${replicas.crashed} crashed replicas`);
  }
  if (expectedDeploymentId && service.deploymentId !== expectedDeploymentId) {
    throw new Error(
      `Railway service ${serviceId} is running deployment ${service.deploymentId || "unknown"}, expected ${expectedDeploymentId}`,
    );
  }
  return service;
}

export function verifyProductionRailwayServices({ deployments, manifest, services }) {
  const verified = [];
  for (const component of RAILWAY_COMPONENTS) {
    const serviceId = manifest?.services?.[component]?.id;
    if (typeof serviceId !== "string" || serviceId.length === 0) {
      throw new Error(`Production manifest is missing Railway service ${component}`);
    }
    const evidence = deployments?.[component];
    if (
      evidence?.provider !== "railway"
      || !SOURCE_SHA_PATTERN.test(evidence.sourceSha ?? "")
      || typeof evidence.evidenceId !== "string"
      || evidence.evidenceId.length === 0
    ) {
      throw new Error(`Production ${component} has no exact Railway deployment evidence`);
    }
    assertHealthyRailwayService(services, serviceId, evidence.evidenceId);
    verified.push({ component, ...evidence });
  }
  return verified;
}

function environmentJson(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return JSON.parse(value);
}

function main() {
  const services = environmentJson("SCOPE_RAILWAY_SERVICES_JSON");
  if (process.env.SCOPE_PRODUCTION_DEPLOYMENTS_JSON) {
    const verified = verifyProductionRailwayServices({
      deployments: environmentJson("SCOPE_PRODUCTION_DEPLOYMENTS_JSON"),
      manifest: environmentJson("SCOPE_DEPLOYMENT_MANIFEST_JSON"),
      services,
    });
    process.stdout.write(`${JSON.stringify(verified)}\n`);
    return;
  }

  const service = assertHealthyRailwayService(
    services,
    process.env.SCOPE_RAILWAY_SERVICE_ID ?? "",
    process.env.SCOPE_EXPECTED_RAILWAY_DEPLOYMENT_ID ?? "",
  );
  process.stdout.write(`${JSON.stringify(service)}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
