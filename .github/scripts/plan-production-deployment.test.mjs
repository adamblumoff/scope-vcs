import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { classifyChanges } from "./plan-production-deployment.mjs";

const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url), "utf8"));

function repositoryJson(path) {
  return JSON.parse(readFileSync(new URL(`../../${path}`, import.meta.url), "utf8"));
}

test("documentation-only changes do not deploy", () => {
  assert.deepEqual(classifyChanges(manifest, ["docs/cache.md"]), {
    checksImage: false,
    cache: false,
    worker: false,
    api: false,
    web: false,
    cli: false,
  });
});

test("cache service changes run backend only", () => {
  assert.deepEqual(classifyChanges(manifest, ["cache-service/src/main.rs"]), {
    checksImage: false,
    cache: true,
    worker: false,
    api: false,
    web: false,
    cli: false,
  });
});

test("runner changes publish the image before the backend lane", () => {
  assert.deepEqual(classifyChanges(manifest, ["runner-runtime/src/main.rs"]), {
    checksImage: true,
    cache: false,
    worker: true,
    api: false,
    web: false,
    cli: false,
  });
});

test("web-only changes deploy only web", () => {
  assert.deepEqual(classifyChanges(manifest, ["web/src/routes/+page.svelte"]), {
    checksImage: false,
    cache: false,
    worker: false,
    api: false,
    web: true,
    cli: false,
  });
});

test("shared workspace changes preserve the previous conservative scope", () => {
  assert.deepEqual(classifyChanges(manifest, ["crates/scope-domain/src/lib.rs"]), {
    checksImage: true,
    cache: true,
    worker: true,
    api: true,
    web: true,
    cli: true,
  });
});

test("conductor changes exercise every lane", () => {
  assert.deepEqual(classifyChanges(manifest, [".github/workflows/scope-production-deploy.yml"]), {
    checksImage: true,
    cache: true,
    worker: true,
    api: true,
    web: true,
    cli: true,
  });
});

test("manual component and all scopes are explicit", () => {
  assert.equal(classifyChanges(manifest, [], "web").web, true);
  assert.ok(Object.values(classifyChanges(manifest, [], "all")).every(Boolean));
  assert.throws(() => classifyChanges(manifest, [], "database"), /Unknown deployment scope/);
});

test("deployment manifest is a single coherent production graph", () => {
  const order = ["cache", "worker", "api", "web", "cli"];
  const serviceIds = order.map((service) => manifest.services[service].id);

  assert.equal(manifest.deploymentAuthority, "github-actions");
  assert.equal(manifest.source.nativeAutodeploy, false);
  for (const service of ["api", "worker"]) {
    assert.ok(Number.isInteger(manifest.services[service].bootstrapReplicas));
    assert.ok(manifest.services[service].bootstrapReplicas > 0);
  }
  assert.equal(new Set(serviceIds).size, serviceIds.length);
  for (const [service, configuration] of Object.entries(manifest.services)) {
    for (const dependency of configuration.dependsOn) {
      assert.ok(order.indexOf(dependency) < order.indexOf(service));
    }
  }
});

test("service config does not override Railway scaling or restart defaults", () => {
  for (const path of ["api/railway.json", "worker/railway.json", "cache-service/railway.json"]) {
    const { deploy } = repositoryJson(path);

    assert.equal(deploy.healthcheckTimeout, 60);
    assert.equal(deploy.multiRegionConfig, undefined);
    assert.equal(deploy.restartPolicyType, undefined);
    assert.equal(deploy.restartPolicyMaxRetries, undefined);
  }
});
