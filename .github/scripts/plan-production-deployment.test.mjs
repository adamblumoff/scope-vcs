import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  classifyChanges,
  planFromDeploymentProgress,
} from "./plan-production-deployment.mjs";

const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url), "utf8"));

function repositoryJson(path) {
  return JSON.parse(readFileSync(new URL(`../../${path}`, import.meta.url), "utf8"));
}

function deploymentSelection(overrides = {}) {
  return {
    checksImage: false,
    cache: false,
    worker: false,
    api: false,
    web: false,
    cli: false,
    cliDistribution: false,
    ...overrides,
  };
}

test("changes select the required deployment lanes", () => {
  const allLanes = {
    checksImage: true,
    cache: true,
    worker: true,
    api: true,
    web: true,
    cli: true,
    cliDistribution: true,
  };
  const cases = [
    ["documentation-only changes do not deploy", ["docs/cache.md"], {}],
    ["cache service changes run backend only", ["cache-service/src/main.rs"], { cache: true }],
    [
      "runner changes publish the image before the backend lane",
      ["runner-runtime/src/main.rs"],
      { checksImage: true, worker: true },
    ],
    [
      "toolchain changes publish the checks image and rebuild Rust services",
      ["rust-toolchain.toml"],
      {
        checksImage: true,
        cache: true,
        worker: true,
        api: true,
        cli: true,
        cliDistribution: true,
      },
    ],
    ["web-only changes deploy only web", ["web/src/routes/+page.svelte"], { web: true }],
    [
      "API implementation changes validate CLI without rebuilding distribution targets",
      ["api/src/main.rs"],
      { api: true, web: true, cli: true },
    ],
    [
      "CLI tests validate CLI without rebuilding distribution targets",
      ["cli/tests/request.rs"],
      { cli: true },
    ],
    [
      "CLI source changes rebuild distribution targets",
      ["cli/src/request.rs"],
      { cli: true, cliDistribution: true },
    ],
    [
      "distribution config changes rebuild distribution targets",
      ["cli/distribution/targets.json"],
      { cli: true, cliDistribution: true },
    ],
    [
      "unrelated shared crates retain broad CLI validation without rebuilding targets",
      ["crates/scope-cache-contract/src/lib.rs"],
      {
        checksImage: true,
        cache: true,
        worker: true,
        api: true,
        web: true,
        cli: true,
      },
    ],
    [
      "shared workspace changes preserve the previous conservative scope",
      ["crates/scope-domain/src/lib.rs"],
      allLanes,
    ],
    [
      "conductor changes exercise every lane",
      [".github/workflows/scope-production-deploy.yml"],
      allLanes,
    ],
  ];

  for (const [name, paths, lanes] of cases) {
    assert.deepEqual(classifyChanges(manifest, paths), deploymentSelection(lanes), name);
  }
});

test("manual component and all scopes are explicit", () => {
  assert.deepEqual(classifyChanges(manifest, [], "web"), deploymentSelection({ web: true }));
  assert.deepEqual(
    classifyChanges(manifest, [], "cli"),
    deploymentSelection({ cli: true, cliDistribution: true }),
  );
  assert.ok(Object.values(classifyChanges(manifest, [], "all")).every(Boolean));
  assert.throws(
    () => classifyChanges(manifest, [], "cliDistribution"),
    /Unknown deployment scope/,
  );
  assert.throws(() => classifyChanges(manifest, [], "database"), /Unknown deployment scope/);
});

test("planner emits the CLI distribution selection as a snake-case workflow output", () => {
  const output = execFileSync(process.execPath, [
    fileURLToPath(new URL("./plan-production-deployment.mjs", import.meta.url)),
    "--manifest",
    fileURLToPath(new URL("../deployment-services.json", import.meta.url)),
    "--scope",
    "cli",
  ], { encoding: "utf8" });

  assert.match(output, /^cli=true$/m);
  assert.match(output, /^cli_distribution=true$/m);
  assert.doesNotMatch(output, /^cliDistribution=/m);
});

test("an unseeded production ledger deploys every component", () => {
  assert.ok(Object.values(planFromDeploymentProgress(manifest, {})).every(Boolean));
});

test("skipped components remain selected across a later backend-only change", () => {
  const selection = planFromDeploymentProgress(manifest, {
    checksImage: [],
    cache: ["cache-service/src/main.rs"],
    worker: [],
    api: [],
    // Web last succeeded before commit A. Its component-specific range still includes A's
    // web change when commit B changes only the cache service after A's web job was skipped.
    web: ["web/src/routes/+page.svelte", "cache-service/src/main.rs"],
    cli: [],
  });

  assert.deepEqual(selection, {
    checksImage: false,
    cache: true,
    worker: false,
    api: false,
    web: true,
    cli: false,
    cliDistribution: false,
  });
});

test("CLI deployment progress selects distribution builds only for binary inputs", () => {
  const broadOnly = planFromDeploymentProgress(manifest, {
    checksImage: [],
    cache: [],
    worker: [],
    api: [],
    web: [],
    cli: ["api/src/main.rs"],
  });
  const binaryChange = planFromDeploymentProgress(manifest, {
    checksImage: [],
    cache: [],
    worker: [],
    api: [],
    web: [],
    cli: ["crates/scope-api-contract/src/lib.rs"],
  });

  assert.deepEqual(broadOnly, deploymentSelection({ cli: true }));
  assert.deepEqual(
    binaryChange,
    deploymentSelection({ cli: true, cliDistribution: true }),
  );
});

test("manual scopes ignore pending production components", () => {
  assert.deepEqual(planFromDeploymentProgress(manifest, {}, "web"), {
    checksImage: false,
    cache: false,
    worker: false,
    api: false,
    web: true,
    cli: false,
    cliDistribution: false,
  });
});

test("deployment manifest is a single coherent production graph", () => {
  const order = ["cache", "worker", "api", "web", "cli"];
  const serviceIds = order.map((service) => manifest.services[service].id);

  assert.equal(manifest.deploymentAuthority, "github-actions");
  assert.equal(manifest.source.nativeAutodeploy, false);
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
