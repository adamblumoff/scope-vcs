import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { selectCliDistributionTargets } from "./select-cli-distribution-targets.mjs";

const configuration = JSON.parse(
  readFileSync(new URL("../../cli/distribution/targets.json", import.meta.url), "utf8"),
);

test("pull requests build only the Linux x64 smoke target", () => {
  const plan = selectCliDistributionTargets(configuration, "pull-request");

  assert.deepEqual(plan.include.map(({ target }) => target), ["x86_64-unknown-linux-gnu"]);
  assert.equal(plan.include[0].smoke, true);
});

test("release runs retain every configured native target", () => {
  const plan = selectCliDistributionTargets(configuration, "release");

  assert.deepEqual(
    plan.include.map(({ target }) => target),
    configuration.targets.map(({ triple }) => triple),
  );
});

test("unknown modes fail instead of silently dropping release targets", () => {
  assert.throws(
    () => selectCliDistributionTargets(configuration, "nightly"),
    /Unknown CLI distribution mode/,
  );
});
