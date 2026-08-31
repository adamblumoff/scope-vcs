import assert from "node:assert/strict";
import test from "node:test";

import {
  RAILWAY_COMPONENTS,
  assertHealthyRailwayService,
  verifyProductionRailwayServices,
} from "./railway-service-health.mjs";

const SOURCE_SHA = "a".repeat(40);

function healthyService(id) {
  return {
    id,
    name: `name-${id}`,
    status: "SUCCESS",
    deploymentId: `deployment-${id}`,
    deploymentStopped: false,
    replicas: { configured: 2, running: 2, crashed: 0 },
  };
}

test("accepts the exact active healthy deployment", () => {
  const service = healthyService("api");
  assert.equal(
    assertHealthyRailwayService([service], "api", "deployment-api"),
    service,
  );
});

test("rejects unavailable Railway service states", () => {
  const cases = [
    ["missing", [], /is missing/],
    ["failed probe", [{ ...healthyService("api"), status: "CRASHED" }], /is CRASHED/],
    ["stopped", [{ ...healthyService("api"), deploymentStopped: true }], /is stopped/],
    [
      "zero replicas",
      [{ ...healthyService("api"), replicas: { configured: 0, running: 0, crashed: 0 } }],
      /no configured replicas/,
    ],
    [
      "partial replicas",
      [{ ...healthyService("api"), replicas: { configured: 2, running: 1, crashed: 0 } }],
      /1\/2 running replicas/,
    ],
    [
      "crashed replicas",
      [{ ...healthyService("api"), replicas: { configured: 2, running: 2, crashed: 1 } }],
      /1 crashed replicas/,
    ],
    ["wrong deployment", [healthyService("api")], /expected previous-api/],
  ];

  for (const [name, services, expected] of cases) {
    assert.throws(
      () => assertHealthyRailwayService(services, "api", name === "wrong deployment" ? "previous-api" : ""),
      expected,
      name,
    );
  }
});

test("production verification binds every live service to durable Railway evidence", () => {
  const manifest = { services: {} };
  const deployments = {};
  const services = [];
  for (const component of RAILWAY_COMPONENTS) {
    manifest.services[component] = { id: component };
    deployments[component] = {
      sourceSha: SOURCE_SHA,
      provider: "railway",
      evidenceId: `deployment-${component}`,
    };
    services.push(healthyService(component));
  }

  assert.deepEqual(
    verifyProductionRailwayServices({ deployments, manifest, services })
      .map(({ component }) => component),
    RAILWAY_COMPONENTS,
  );

  delete deployments.web;
  assert.throws(
    () => verifyProductionRailwayServices({ deployments, manifest, services }),
    /web has no exact Railway deployment evidence/,
  );
});
