import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { runInNewContext } from "node:vm";

import {
  classifyChanges,
  planFromDeploymentProgress,
} from "./plan-production-deployment.mjs";

const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url), "utf8"));
const productionWorkflow = readFileSync(
  new URL("../workflows/scope-production-deploy.yml", import.meta.url),
  "utf8",
);
const integrationCiWorkflow = readFileSync(
  new URL("../workflows/scope-integration-ci.yml", import.meta.url),
  "utf8",
);
const rustChecksWorkflow = readFileSync(
  new URL("../workflows/rust-workspace-checks.yml", import.meta.url),
  "utf8",
);
const cliDeployWorkflow = readFileSync(
  new URL("../workflows/scope-cli-deploy.yml", import.meta.url),
  "utf8",
);
const stagingWorkflow = readFileSync(
  new URL("../workflows/scope-railway-staging.yml", import.meta.url),
  "utf8",
);
const webDeployWorkflow = readFileSync(
  new URL("../workflows/scope-web-deploy.yml", import.meta.url),
  "utf8",
);
const webCiWorkflow = readFileSync(
  new URL("../workflows/scope-web-ci.yml", import.meta.url),
  "utf8",
);

function repositoryJson(path) {
  return JSON.parse(readFileSync(new URL(`../../${path}`, import.meta.url), "utf8"));
}

function deploymentSelection(overrides = {}) {
  return {
    checksImage: false,
    cache: false,
    worker: false,
    router: false,
    api: false,
    web: false,
    cli: false,
    cliDistribution: false,
    ...overrides,
  };
}

function productionJobCondition(jobName) {
  const block = productionWorkflow.match(
    new RegExp(`\\n  ${jobName}:\\n([\\s\\S]*?)(?=\\n  [a-z][a-z-]+:|$)`),
  )?.[1];
  assert.ok(block, `${jobName} job is present`);
  const condition = block.match(/\n    if: >-\n([\s\S]*?)(?=\n    [a-z])/i)?.[1];
  assert.ok(condition, `${jobName} has a multiline condition`);
  return condition.trim().replace(/\s+/g, " ");
}

function evaluateProductionCondition(expression, context) {
  const resolve = (path) => path.split(".").reduce((value, key) => value?.[key], context);
  const executable = expression.replace(
    /cancelled\(\)|(?:github|needs)(?:\.[A-Za-z0-9_-]+)+/g,
    (reference) => JSON.stringify(
      reference === "cancelled()" ? context.cancelled : resolve(reference),
    ),
  );
  const unsupported = executable.replace(
    /true|false|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|&&|\|\||==|!=|!|\(|\)|\s+/g,
    "",
  );
  assert.equal(unsupported, "", `unsupported workflow expression syntax: ${unsupported}`);
  return Boolean(runInNewContext(executable, Object.create(null), { timeout: 100 }));
}

function productionConditionContext(overrides = {}) {
  const backendSelected = overrides.backendSelected ?? false;
  return {
    cancelled: overrides.cancelled ?? false,
    github: {
      event_name: overrides.eventName ?? "push",
      ref: overrides.ref ?? "refs/heads/main",
    },
    needs: {
      "backend-deploy": { result: overrides.backendResult ?? "skipped" },
      "cli-deploy": {
        result: overrides.cliResult ?? (overrides.cliSelected === false ? "skipped" : "success"),
      },
      plan: {
        outputs: {
          api: backendSelected ? "true" : "false",
          cache: backendSelected ? "true" : "false",
          cli: overrides.cliSelected === false ? "false" : "true",
          router: backendSelected ? "true" : "false",
          web: overrides.webSelected === false ? "false" : "true",
          worker: backendSelected ? "true" : "false",
        },
      },
      "production-validation-gate": {
        result: overrides.validationResult ?? "success",
      },
      "web-deploy": {
        result: overrides.webResult ?? (overrides.webSelected === false ? "skipped" : "success"),
      },
    },
  };
}

test("changes select the required deployment lanes", () => {
  const allLanes = {
    checksImage: true,
    cache: true,
    worker: true,
    router: true,
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
        router: true,
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
    ["router changes deploy the Git router", ["repo-router/src/main.rs"], { router: true }],
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
        router: true,
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
    [
      "production health policy changes exercise every lane",
      [".github/scripts/railway-service-health.mjs"],
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
  ], {
    encoding: "utf8",
    env: {
      ...process.env,
      GITHUB_OUTPUT: "",
      GITHUB_STEP_SUMMARY: "",
    },
  });

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
    router: [],
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
    router: false,
    api: false,
    web: true,
    cli: false,
    cliDistribution: false,
  });
});

test("skipped validation ancestors do not suppress a selected backend deployment", () => {
  assert.match(
    productionJobCondition("backend-deploy"),
    /^!cancelled\(\) && github\.event_name/,
  );
});

test("web and CLI deployment conditions are cancellation-safe after optional backend jobs", () => {
  const conditions = Object.fromEntries(["web-deploy", "cli-deploy"].map((job) => [
    job,
    productionJobCondition(job),
  ]));
  for (const condition of Object.values(conditions)) {
    assert.match(condition, /^!cancelled\(\) && github\.event_name/);
    assert.match(condition, /needs\.production-validation-gate\.result == 'success'/);
    assert.match(condition, /needs\.backend-deploy\.result == 'success'/);
    assert.match(condition, /needs\.backend-deploy\.result == 'skipped'/);
  }
  assert.equal(
    conditions["web-deploy"].replace("needs.plan.outputs.web", "needs.plan.outputs.component"),
    conditions["cli-deploy"].replace("needs.plan.outputs.cli", "needs.plan.outputs.component"),
  );
});

test("frontend production deployment eligibility covers optional backend and failure states", () => {
  const fixtures = [
    ["backend selected", { backendSelected: true, backendResult: "success" }, true],
    ["backend skipped", { backendSelected: false, backendResult: "skipped" }, true],
    [
      "failed validation",
      { backendSelected: false, backendResult: "skipped", validationResult: "failure" },
      false,
    ],
    ["failed backend", { backendSelected: true, backendResult: "failure" }, false],
    [
      "canceled workflow",
      { backendSelected: false, backendResult: "skipped", cancelled: true },
      false,
    ],
  ];

  for (const job of ["web-deploy", "cli-deploy"]) {
    const condition = productionJobCondition(job);
    for (const [name, input, expected] of fixtures) {
      assert.equal(
        evaluateProductionCondition(condition, productionConditionContext(input)),
        expected,
        `${job}: ${name}`,
      );
    }
  }
});

test("the final production gate verifies selected and carried-forward services", () => {
  const condition = productionJobCondition("production-health-gate");
  assert.match(condition, /^!cancelled\(\) && github\.event_name/);
  const fixtures = [
    [
      "all Railway components carried forward",
      { backendSelected: false, cliSelected: false, webSelected: false },
      true,
    ],
    [
      "backend selected and frontend carried forward",
      {
        backendSelected: true,
        backendResult: "success",
        cliSelected: false,
        webSelected: false,
      },
      true,
    ],
    [
      "selected web deployment failed",
      { backendSelected: false, cliSelected: false, webResult: "failure" },
      false,
    ],
    [
      "selected backend deployment failed",
      {
        backendSelected: true,
        backendResult: "failure",
        cliSelected: false,
        webSelected: false,
      },
      false,
    ],
    [
      "workflow canceled after deploy jobs",
      { backendSelected: false, cancelled: true, cliSelected: false, webSelected: false },
      false,
    ],
  ];
  for (const [name, input, expected] of fixtures) {
    assert.equal(
      evaluateProductionCondition(condition, productionConditionContext(input)),
      expected,
      name,
    );
  }
});

test("CLI deployment progress selects distribution builds only for binary inputs", () => {
  const broadOnly = planFromDeploymentProgress(manifest, {
    checksImage: [],
    cache: [],
    worker: [],
    router: [],
    api: [],
    web: [],
    cli: ["api/src/main.rs"],
  });
  const binaryChange = planFromDeploymentProgress(manifest, {
    checksImage: [],
    cache: [],
    worker: [],
    router: [],
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
    router: false,
    api: false,
    web: true,
    cli: false,
    cliDistribution: false,
  });
});

test("deployment manifest is a single coherent production graph", () => {
  const order = ["cache", "worker", "router", "api", "web", "cli"];
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
  const configs = {
    "api/railway.json": "/healthz",
    "worker/railway.json": "/healthz",
    "cache-service/railway.json": "/readyz",
    "repo-router/railway.json": "/readyz",
    "cli/railway.json": "/readyz",
    "web/railway.json": "/",
  };
  for (const [path, healthcheckPath] of Object.entries(configs)) {
    const { deploy } = repositoryJson(path);

    assert.equal(deploy.healthcheckPath, healthcheckPath);
    assert.equal(deploy.healthcheckTimeout, 60);
    assert.equal(deploy.multiRegionConfig, undefined);
    assert.equal(deploy.restartPolicyType, undefined);
    assert.equal(deploy.restartPolicyMaxRetries, undefined);
  }
});

test("web and CLI healthcheck configs are staged at Railway upload roots", () => {
  assert.match(cliDeployWorkflow, /cp cli\/railway\.json \.railway-upload\/railway\.json/);
  assert.match(webDeployWorkflow, /cp web\/railway\.json \.railway-upload\/railway\.json/);
  assert.match(
    stagingWorkflow,
    /cp candidate\/web\/railway\.json \.railway-staging-upload\/web-root\/railway\.json/,
  );
});

test("Node workflows cache pnpm and browser downloads by the web lockfile", () => {
  for (const workflow of [integrationCiWorkflow, rustChecksWorkflow, webCiWorkflow]) {
    assert.match(
      workflow,
      /uses: pnpm\/action-setup@[0-9a-f]{40} # v5/,
    );
    assert.match(workflow, /cache: pnpm/);
    assert.match(workflow, /cache-dependency-path: web\/pnpm-lock\.yaml/);
  }

  assert.match(integrationCiWorkflow, /path: ~\/\.cache\/ms-playwright/);
  assert.match(
    integrationCiWorkflow,
    /key: playwright-\$\{\{ runner\.os \}\}-\$\{\{ hashFiles\('web\/pnpm-lock\.yaml'\) \}\}/,
  );
});
