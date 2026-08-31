import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  latestSuccessfulDeployments,
  latestSuccessfulRevisions,
  recordEvidenceFile,
  recordSuccessfulDeployment,
} from "./production-deployment-progress.mjs";

const SOURCE_SHA = "a".repeat(40);
const PREVIOUS_SHA = "b".repeat(40);

function response(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

test("reads the newest successful revision for every component", async () => {
  const previousToken = process.env.GITHUB_TOKEN;
  const previousRepository = process.env.GITHUB_REPOSITORY;
  process.env.GITHUB_TOKEN = "test-token";
  process.env.GITHUB_REPOSITORY = "scope-vcs/scope-vcs";

  const fetchImpl = async (url) => {
    const parsed = new URL(url);
    if (parsed.pathname.endsWith("/deployments")) {
      const component = parsed.searchParams.get("environment").split("/")[1];
      return response([
        {
          id: `${component}-new`,
          sha: SOURCE_SHA,
          payload: {
            component,
            sourceSha: SOURCE_SHA,
            provider: "railway",
            evidenceId: `${component}-new-provider-id`,
          },
        },
        {
          id: `${component}-old`,
          sha: PREVIOUS_SHA,
          payload: JSON.stringify({
            component,
            sourceSha: PREVIOUS_SHA,
            provider: "railway",
            evidenceId: `${component}-old-provider-id`,
          }),
        },
      ]);
    }
    if (parsed.pathname.includes("-new/statuses")) return response([{ state: "failure" }]);
    if (parsed.pathname.includes("-old/statuses")) return response([{ state: "inactive" }, { state: "success" }]);
    throw new Error(`Unexpected request: ${url}`);
  };

  try {
    const revisions = await latestSuccessfulRevisions(fetchImpl);
    assert.equal(revisions.web, PREVIOUS_SHA);
    assert.equal(revisions.cache, PREVIOUS_SHA);
  } finally {
    if (previousToken === undefined) delete process.env.GITHUB_TOKEN;
    else process.env.GITHUB_TOKEN = previousToken;
    if (previousRepository === undefined) delete process.env.GITHUB_REPOSITORY;
    else process.env.GITHUB_REPOSITORY = previousRepository;
  }
});

test("reads exact provider identity from the newest valid successful deployment", async () => {
  const previousToken = process.env.GITHUB_TOKEN;
  const previousRepository = process.env.GITHUB_REPOSITORY;
  process.env.GITHUB_TOKEN = "test-token";
  process.env.GITHUB_REPOSITORY = "scope-vcs/scope-vcs";

  const fetchImpl = async (url) => {
    const parsed = new URL(url);
    if (parsed.pathname.endsWith("/deployments")) {
      const component = parsed.searchParams.get("environment").split("/")[1];
      return response([
        {
          id: `${component}-invalid`,
          sha: SOURCE_SHA,
          payload: { component, sourceSha: PREVIOUS_SHA, provider: "railway", evidenceId: "wrong" },
        },
        {
          id: `${component}-valid`,
          sha: SOURCE_SHA,
          payload: { component, sourceSha: SOURCE_SHA, provider: "railway", evidenceId: `${component}-7` },
        },
      ]);
    }
    return response([{ state: "success" }]);
  };

  try {
    const deployments = await latestSuccessfulDeployments(fetchImpl);
    assert.deepEqual(deployments.web, {
      sourceSha: SOURCE_SHA,
      provider: "railway",
      evidenceId: "web-7",
    });
  } finally {
    if (previousToken === undefined) delete process.env.GITHUB_TOKEN;
    else process.env.GITHUB_TOKEN = previousToken;
    if (previousRepository === undefined) delete process.env.GITHUB_REPOSITORY;
    else process.env.GITHUB_REPOSITORY = previousRepository;
  }
});

test("records source revision and provider evidence before marking success", async () => {
  const previousToken = process.env.GITHUB_TOKEN;
  const previousRepository = process.env.GITHUB_REPOSITORY;
  process.env.GITHUB_TOKEN = "test-token";
  process.env.GITHUB_REPOSITORY = "scope-vcs/scope-vcs";
  const requests = [];
  const fetchImpl = async (url, options) => {
    requests.push({ url, body: JSON.parse(options.body) });
    return requests.length === 1 ? response({ id: 42 }, 201) : response({ id: 43 }, 201);
  };

  try {
    await recordSuccessfulDeployment({
      component: "web",
      sourceSha: SOURCE_SHA,
      provider: "railway",
      evidenceId: "railway-deployment-7",
      logUrl: "https://github.test/run/1",
    }, fetchImpl);
  } finally {
    if (previousToken === undefined) delete process.env.GITHUB_TOKEN;
    else process.env.GITHUB_TOKEN = previousToken;
    if (previousRepository === undefined) delete process.env.GITHUB_REPOSITORY;
    else process.env.GITHUB_REPOSITORY = previousRepository;
  }

  assert.deepEqual(requests[0].body.payload, {
    component: "web",
    sourceSha: SOURCE_SHA,
    provider: "railway",
    evidenceId: "railway-deployment-7",
  });
  assert.equal(requests[0].body.environment, "production/web");
  assert.equal(requests[1].body.state, "success");
  assert.equal(requests[1].body.auto_inactive, false);
});

test("rejects unknown components before writing deployment state", async () => {
  await assert.rejects(
    recordSuccessfulDeployment({
      component: "database",
      sourceSha: SOURCE_SHA,
      provider: "railway",
      evidenceId: "deployment-7",
    }, () => { throw new Error("fetch should not run"); }),
    /Unknown deployment component/,
  );
});

test("rejects abbreviated source revisions before writing deployment state", async () => {
  await assert.rejects(
    recordSuccessfulDeployment({
      component: "web",
      sourceSha: "abc123",
      provider: "railway",
      evidenceId: "deployment-7",
    }, () => { throw new Error("fetch should not run"); }),
    /full lowercase commit SHA/,
  );
});

test("records an ordered Railway evidence stream", async () => {
  const previousToken = process.env.GITHUB_TOKEN;
  const previousRepository = process.env.GITHUB_REPOSITORY;
  process.env.GITHUB_TOKEN = "test-token";
  process.env.GITHUB_REPOSITORY = "scope-vcs/scope-vcs";
  const directory = mkdtempSync(join(tmpdir(), "deployment-progress-"));
  const evidencePath = join(directory, "evidence.ndjson");
  writeFileSync(evidencePath, [
    JSON.stringify({ component: "cache", sourceSha: SOURCE_SHA, provider: "railway", evidenceId: "cache-1" }),
    JSON.stringify({ component: "worker", sourceSha: SOURCE_SHA, provider: "railway", evidenceId: "worker-1" }),
    "",
  ].join("\n"));
  const requests = [];
  const fetchImpl = async (url, options) => {
    requests.push({ url, body: JSON.parse(options.body) });
    const isDeployment = new URL(url).pathname.endsWith("/deployments");
    return response({ id: requests.length }, isDeployment ? 201 : 200);
  };

  try {
    const ids = await recordEvidenceFile(evidencePath, "https://github.test/run/1", fetchImpl);
    assert.deepEqual(ids, [1, 3]);
    assert.deepEqual(
      requests.filter(({ url }) => new URL(url).pathname.endsWith("/deployments"))
        .map(({ body }) => body.payload.component),
      ["cache", "worker"],
    );
  } finally {
    rmSync(directory, { recursive: true });
    if (previousToken === undefined) delete process.env.GITHUB_TOKEN;
    else process.env.GITHUB_TOKEN = previousToken;
    if (previousRepository === undefined) delete process.env.GITHUB_REPOSITORY;
    else process.env.GITHUB_REPOSITORY = previousRepository;
  }
});
