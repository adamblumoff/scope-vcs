# Scope: The Big Picture

## The ambition

Scope is not trying to make a slightly better pull-request interface. It is
rebuilding the collaboration layer around source code for a world where agents
can produce changes far faster than humans can review them.

GitHub's collaboration model assumes that creating a contribution is expensive
and that maintainer attention is comparatively available. Agentic coding
reverses that relationship. Producing another patch or request is nearly free;
understanding whether it deserves attention remains expensive.

Scope should be built around that new constraint:

> Maintainers define what earns their attention. Scope starts with an explicit
> submission boundary and can automate admission only after the underlying
> collaboration loop has earned that complexity.

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

Today, the contributor owns the first attention decision. A request can remain
a private draft while its participants work on it. The contributor submits it
once when they want maintainer review, and every submitted request enters the
Open queue. The request remains mutable until it is merged or closed.

This is intentionally similar to draft and open GitHub pull requests. Scope
does not qualify requests, run an automated admission gate, or ask contributors
to perform a separate review ceremony today.

The long-term attention boundary may add repository-defined qualification. If
it does, only qualified requests should enter the maintainer's normal queue.
That is a later product layer, not part of the current request lifecycle.

Private files solve access control. Submission currently solves attention
intent. Qualification may later solve attention admission.

## What exists today

The current foundation is deliberately small:

- Public and private repository projections enforce the code boundary.
- Trusted `.scope` control paths cannot be changed through contributor
  requests.
- Every repository has a tracked `.scope/RULES.md`, even when the file is
  empty.
- Scope links those rules into repository-level Codex and Claude context when
  their files or tool directories signal that the repository uses them.
- `scope push` requires the rules file and required agent-context links to be
  committed and current.
- Requests move through Draft, Open, Closed, and Merged. Submission is a
  one-way Draft-to-Open transition; Open requests remain mutable.
- The request queue has Your work, Open, and Closed sections.
- Credits, staking, assessment, hold, and request-changes ceremony have been
  removed.
- Verified request participants can rate one another after closeout, and a
  small aggregate rating context is available without turning reputation into
  admission or ranking.

## `.scope/RULES.md`

Every public contributor workspace receives a trusted `.scope/RULES.md`. This
is the maintainer's natural-language contract for what a worthwhile
contribution looks like.

The file may be empty. When a repository has rules, they can describe:

- work that is currently wanted or explicitly unwanted;
- required evidence, tests, benchmarks, or reproduction steps;
- architectural and product constraints;
- acceptable change size and scope;
- duplicate-work policy;
- security, licensing, or provenance requirements; and
- any other condition the maintainer wants contributors or their agents to
  follow.

Markdown is the right starting format. Maintainers already express these rules
in contribution guides, issue templates, and repository documentation. Scope
makes the file reliably available without forcing maintainers to learn a new
policy language.

`RULES.md` is a control-plane document:

- It is maintained from a trusted source.
- It is projected into public contributor workspaces.
- Repository-level agent context points agents to it while they work.
- It is read-only from the perspective of a contributor request.
- It is not accepted as part of the contributor's source changes.

A local Git workspace cannot literally make a file impossible to delete. The
server therefore owns the real invariant: a contributor delta that modifies,
renames, replaces, or deletes a protected `.scope` path is invalid. Scope's CLI
also validates the committed rules and agent-context links before push.

These protections are platform rules, not prose inside `RULES.md`.

## The request lifecycle today

The request lifecycle has four states:

1. **Draft** — participants can work without surfacing the request to the
   maintainer queue.
2. **Open** — the author has submitted once and the request is visible for
   review.
3. **Closed** — the request ended without merging.
4. **Merged** — the request was applied to the repository.

Submission is contributor discretion. There is no platform-defined notion of
"qualified" today. After submission, participants can continue pushing
revisions, editing request identity, and using discussions. Scope records those
changes without forcing the request back through another submission state.

The queue follows the same simple model:

- **Your work** contains requests involving the signed-in user, including
  drafts that should not appear elsewhere.
- **Open** contains submitted requests visible to the viewer.
- **Closed** contains closed and merged history visible to the viewer.

## Reputation without an economy

Scope should not launch with credits, bidding, staking, or reputation-based
admission. Those systems create an economy before the product has earned one,
make onboarding confusing, and disadvantage new participants.

The current model is verified participant ratings:

- People can leave a one-to-five-star rating after a real request closes or
  merges.
- A rating includes a short reason.
- Reviews are tied to the underlying collaboration and are not anonymous.
- New accounts are unrated, not zero-star.
- Global context exposes only a total score and rating count.
- Ratings do not rank requests, control submission, or grant permissions.

This is enough until real use shows which additional reputation signals are
valuable. Averages, ranking, recency weighting, project-local reputation,
appeals, and anti-brigading machinery should not be added speculatively.

## Qualification is a later layer

Automated qualification remains a plausible long-term answer to contribution
volume, but Scope should not add an agent evaluator merely because
`.scope/RULES.md` exists. The simple request workflow needs real use first.

Future design work must answer:

- what revision and rule version were evaluated;
- how an Open request is updated and re-evaluated;
- how failures, retries, overrides, and rule changes behave;
- what explanation and evidence the contributor receives;
- how evaluator identity and version are recorded;
- how compute, authentication, rate limits, and abuse are bounded; and
- whether qualification gates submission or only maintainer attention.

If repeated use proves that qualification is needed, the likely extension
contract is:

```text
evaluate(
  request revision,
  trusted RULES.md,
  permitted repository context
) -> qualified | rejected + explanation + evidence
```

The evaluator may run tests, invoke an agent, query an issue tracker, inspect
provenance, or combine several checks. Maintainers should own what
qualification means; Scope should own protected inputs, allowed transitions,
visibility, resource limits, and delivery of the result.

Duplicate detection belongs inside that future maintainer-configured loop. It
is a semantic input to qualification, not a universal platform model that
Scope needs today.

## The integration strategy

GitHub's deepest moat is its integration ecosystem. Scope cannot win by
rebuilding every external tool itself. It needs stable automation surfaces
that let other systems participate without giving up Scope's domain rules.

The CLI is the foundation of that portability. Automation should be possible
without a browser through:

- noninteractive commands;
- stable JSON input and output;
- documented exit codes;
- narrowly scoped credentials;
- versioned event and webhook payloads; and
- reproducible request and actor identities.

The next automation work should strengthen the workflows Scope already has:
repository access, rules synchronization, request creation and revision,
discussion, merge, close, and ratings. A qualification-provider SDK should wait
until the qualification product contract is designed.

## Product experience

### For contributors

A contributor should know the rules before spending effort. Their workspace
contains the public code projection and the repository's trusted rules, and
their agent receives the same repository-level context. They can keep work in
Draft, submit it once, and continue improving the Open request in response to
review.

The system should encourage better work without pretending automated judgment
is already trustworthy enough to gate participation.

### For maintainers

A maintainer sees submitted work in one Open queue and terminal work in Closed.
They do not receive private drafts as inbox work. Discussions, revisions,
merge, close, and participant ratings remain attached to the request instead
of being split across lifecycle ceremonies.

The system should reduce workflow overhead now and leave room for a stronger
attention gate later.

## Sequencing

### Shipped foundation

1. Enforce public/private repository projections and protected control paths.
2. Project `.scope/RULES.md`, inject it into repository-level agent context,
   and require the committed links on push.
3. Replace credits and review ceremony with Draft, Open, Closed, and Merged.
4. Make submission one-way while keeping Open requests mutable.
5. Ship the Your work, Open, and Closed request queue.
6. Add verified participant ratings and minimal aggregate reputation context.

### Now

7. Dogfood and harden the complete contribution loop: draft privacy, revision
   pushes, discussions, search, merge, close, ratings, and consistent Git/API/
   CLI/web behavior.
8. Strengthen noninteractive CLI and event contracts around those proven
   workflows so agents and integrations can use Scope reliably.

### Later, after real usage

9. Decide whether contribution volume requires qualification and design its
   explanations, overrides, failure modes, rule-version behavior, and resource
   limits.
10. If justified, add a qualification-provider contract and gate maintainer
    attention without complicating contributor drafts.
11. Let maintainers compose duplicate detection and other checks into that
    contract.
12. Expand the integration ecosystem around stable, exercised contracts.

Agent task scopes, multiple competing attempts, hosted evaluators, and more
elaborate reputation mechanics are valid later directions. They should not
delay proving the simple contribution loop.

## What Scope should resist

Scope should resist:

- qualification machinery before real request volume demonstrates the need;
- proposal queues that merely relocate slop;
- a large qualification DSL before repeated use cases justify one;
- a platform-owned universal AI evaluator;
- allowing reputation, payment, or popularity to control admission;
- treating contributor-supplied control files as trusted input;
- rebuilding credits, staking, assessment, or mutable submission ceremony;
- building speculative integrations instead of strong exercised contracts;
  and
- turning early reputation into an irreversible social hierarchy.

The product wins when contributors can work freely, maintainers can recognize
intentional submissions, repository rules reach both humans and agents, and
future automation is added only where it measurably protects human attention.
