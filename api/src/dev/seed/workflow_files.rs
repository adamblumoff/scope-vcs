//! Workflow files carried by the seeded demo repository. They exist as real files on main
//! so the runs workflow filter is populated the same way it is for a genuine repository.

pub(super) const PUBLIC_DEMO_CHECKS_WORKFLOW: &str = r#"name: Checks
on:
  manual: true
  push:
    branches:
      - main
container:
  image: ghcr.io/scope/dev-seed-ci@sha256:0000000000000000000000000000000000000000000000000000000000000000
timeout: 30m
jobs:
  build:
    steps:
      - name: Build
        run: cargo build --workspace
  test:
    needs: [build]
    steps:
      - name: Test
        run: cargo test --workspace
  deploy:
    needs: [test]
    steps:
      - name: Package
        run: scripts/package.sh
      - name: Push image
        run: scripts/push-image.sh
      - name: Roll out
        run: scripts/roll-out.sh
"#;

pub(super) const PUBLIC_DEMO_LINT_WORKFLOW: &str = r#"name: Lint
on:
  manual: true
  push:
    branches:
      - main
container:
  image: ghcr.io/scope/dev-seed-ci@sha256:0000000000000000000000000000000000000000000000000000000000000000
timeout: 10m
jobs:
  lint:
    steps:
      - name: Lint
        run: scripts/lint.sh
"#;
