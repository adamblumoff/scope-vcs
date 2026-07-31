# Scope: The Big Picture

## The ambition

Scope is not trying to make a slightly better pull-request interface. It is
rebuilding the collaboration layer around source code for a world where agents
can produce changes far faster than humans can review them.

GitHub's collaboration model assumes that creating a contribution is expensive
and that maintainer attention is comparatively available. Agentic coding
reverses that relationship. Producing another patch, proposal, or pull request
is nearly free; understanding whether it deserves attention remains expensive.

Scope should be built around that new constraint:

> Maintainers define what earns access to their attention. Everything else
> remains invisible.

That sentence is both the product promise and the organizing principle for the
system.

## The two boundaries

Scope creates two separate boundaries around a repository.

### The code boundary

Maintainers decide which files are public and which remain private without
splitting the project across repositories or maintaining synchronization
scripts. Public contributors work against a projection containing only the
source they are allowed to see.

This makes controlled outside contribution possible without forcing a
maintainer to expose the entire codebase.

### The attention boundary

Maintainers decide which requests are worthy of human review. A submitted
request is not automatically placed in a maintainer's queue. It must first pass
the repository's qualification process.

Only qualified requests enter maintainer-facing attention surfaces. Rejected,
duplicate, irrelevant, or incomplete requests remain outside the normal
maintainer workflow. They may still exist for contributor feedback, abuse
handling, and operational auditing, but they do not become inbox work.

Private files solve access control. Qualification solves attention control.
Both matter, but qualification is the urgent wedge in an agentic world.

## `.scope/RULES.md`

Every public contributor workspace receives a trusted `.scope/RULES.md`. This
is the maintainer's natural-language contract for what a worthwhile
contribution looks like.

The file can describe whatever matters to that repository:

- work that is currently wanted or explicitly unwanted;
- required evidence, tests, benchmarks, or reproduction steps;
- architectural and product constraints;
- acceptable change size and scope;
- duplicate-work policy;
- security, licensing, or provenance requirements; and
- any other condition the maintainer wants evaluated before review.

Markdown is the right starting format. Maintainers already express these rules
in contribution guides, issue templates, and repository documentation. Scope
should make that practice operational without forcing maintainers to learn a
new policy language or encode judgment as a rigid schema.

`RULES.md` is a control-plane document:

- It is maintained from a trusted source.
- It is projected into public contributor workspaces so humans and agents can
  read it while working.
- It is read-only from the perspective of a submitted request.
- It is not accepted as part of the contributor's source changes.
- Evaluation always uses the trusted version, never a version supplied by the
  contributor.

A local Git workspace cannot literally make a file impossible to delete. The
server therefore owns the real invariant: any submitted delta that modifies,
renames, replaces, or deletes a protected `.scope` path is invalid and is
rejected before qualification. The trusted file is restored from the base
projection whenever Scope constructs evaluation input.

This protection should be implemented as a platform rule, not as prose inside
`RULES.md`.

## Qualification, not proposals

Adding a proposal step does not solve contribution slop. If proposals are cheap
to generate, the proposal queue becomes the same problem in a different form.
Scope should evaluate the actual request before asking a maintainer to look at
it.

The initial request lifecycle should be small and explicit:

1. **Submitted** — an immutable request revision is ready for evaluation.
2. **Evaluating** — the maintainer's configured qualification loop is running.
3. **Qualified** — the request passed and becomes visible to maintainers.
4. **Rejected** — the request failed and remains outside maintainer attention.

An evaluator failure is not a qualification. It should fail closed, remain out
of the maintainer queue, and give the contributor a clear retryable error.

Qualification can be nondeterministic. Scope does not need to interpret
`RULES.md` or pretend every useful judgment can be reduced to a boolean
configuration file. Instead, Scope supplies a stable contract through which a
maintainer-selected loop evaluates:

- the immutable request revision;
- the trusted `.scope/RULES.md`;
- the permitted repository context; and
- relevant request history or external evidence.

The loop returns:

- `qualified` or `rejected`;
- a contributor-facing explanation;
- structured evidence where available; and
- evaluator identity and version information for reproducibility.

The evaluator may run tests, invoke an agent, use an AI model, query an issue
tracker, check licenses, inspect provenance, or combine several of these.
Maintainers own what qualification means; Scope owns the integrity of the
boundary and the delivery of the result.

Scope must apply authentication, rate limits, request-size limits, and resource
budgets before invoking a maintainer's evaluator. Otherwise spam can consume
compute even if it never reaches a human.

## Duplicate detection

Duplicate detection belongs inside the maintainer-configured qualification
loop. It is a semantic problem, so requiring it to be deterministic would
produce a weak system and unnecessary platform complexity.

A maintainer should be able to connect a model or service that compares a
request with open work, prior requests, issues, and known attempts. The result
becomes one input to qualification. Scope does not need a universal duplicate
model in its first version.

The important product behavior is not how the duplicate was discovered. It is
that the contributor receives useful feedback while the maintainer never has
to triage the duplicate.

## Reputation instead of staking

Scope should not launch with credits, bidding, or staking. Those systems create
an economy before the product has earned one, make onboarding confusing, and
force an arbitrary answer to how new accounts acquire credits.

The simpler model is verified reputation:

- People receive a one-to-five-star rating for completed work.
- The parties to a real request may review one another after it closes.
- A rating includes a short reason rather than only a number.
- Reviews are tied to the underlying collaboration and are not anonymous.
- New accounts are **unrated**, not zero-star.
- Reputation helps order already-qualified requests; it never bypasses
  qualification.

A bare global average is not enough. Scope should show the number of verified
reviews and enough context to distinguish a new contributor from an established
one. A 4.8 based on two interactions is not equivalent to a 4.8 based on one
hundred.

The first version should remain simple: one overall rating and a short
structured note from verified participants. Recency weighting, project-local
reputation, separate quality dimensions, appeals, and stronger anti-brigading
systems can be added when real usage demonstrates the need.

Ranking must leave room for newcomers. A dedicated newcomer lane, aging
mechanism, or explicit exploration allocation should prevent established
accounts from permanently occupying every visible position. Reputation ranks
qualified work; it should not turn lack of history into exclusion.

## The integration strategy

GitHub's deepest moat is its integration ecosystem. Scope cannot win by
rebuilding every external tool itself. It needs a first-class SDK and stable
automation surfaces that let other systems participate without giving up
Scope's domain rules.

The first major extension point should be the qualification-provider contract:

```text
evaluate(
  immutable request revision,
  trusted RULES.md,
  permitted repository context
) -> qualified | rejected + explanation + evidence
```

The CLI is the foundation of that portability. Automation should be possible
without a browser through:

- noninteractive commands;
- stable JSON input and output;
- documented exit codes;
- narrowly scoped credentials;
- versioned event and webhook payloads; and
- reproducible request and evaluator identities.

Models, test systems, security scanners, issue trackers, agent runtimes, and
other developer tools should be able to implement the provider contract. The
domain remains responsible for request states, protected inputs, visibility,
and allowed transitions; adapters remain thin.

A strong CLI and SDK do not merely provide convenience. They are how Scope
starts competing with an incumbent integration ecosystem.

## Product experience

### For contributors

A contributor should know the rules before spending effort. Their workspace
contains the public code projection and the repository's trusted `RULES.md`.
When they submit work, they see whether it is evaluating, qualified, or
rejected. Rejection includes enough explanation to improve the request without
requiring maintainer intervention.

The system should encourage better work, not merely hide bad work.

### For maintainers

A maintainer configures the rules and chooses the evaluation loop. Their normal
request queue contains qualified work only. They can trust that every visible
request was evaluated against the correct rule version and immutable request
revision.

The system should reduce attention spent on triage without removing maintainer
control or pretending automated judgment is infallible.

## Sequencing

The product should be built in this order:

1. Continue strengthening public/private repository projections.
2. Project and protect `.scope/RULES.md` in public contributor workspaces.
3. Define the domain-owned request lifecycle and qualification-provider
   contract.
4. Enforce the rule that only qualified requests enter maintainer attention
   surfaces.
5. Expose qualification through a strong CLI and provider SDK.
6. Let maintainers compose duplicate detection and other checks into their
   loops.
7. Add participant-only post-request reputation with a fair newcomer path.
8. Expand the integration ecosystem around the stable contracts.

Agent task scopes, multiple competing attempts, hardened workflow systems, and
more elaborate reputation mechanics are valid later directions. They should
not delay the core attention boundary.

## What Scope should resist

Scope should resist:

- proposal queues that merely relocate slop;
- a large qualification DSL before repeated use cases justify one;
- a platform-owned universal AI evaluator;
- allowing reputation, payment, or popularity to purchase qualification;
- exposing rejected requests in the maintainer's normal workflow;
- trusting contributor-supplied control files;
- building speculative integrations instead of a strong extension contract;
  and
- turning early reputation into an irreversible social hierarchy.

The product wins when maintainers retain control, contributors receive clear
rules and useful feedback, and the scarce resource—human attention—is spent
only where it can create value.
