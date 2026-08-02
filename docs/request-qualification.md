# Request qualification

Scope does not qualify contribution requests today. A contributor decides when a draft is ready
to submit, submission happens once, and the request remains mutable until it is merged or closed.
Maintainers receive every submitted request without an automated admission gate.

Verified participant ratings provide reputation context after a request reaches a terminal state.
Global reputation context exposes only each participant's total rating score and rating count. It
does not expose an average, rank requests, control submission, or grant permissions. Individual
ratings and their reasons remain visible only wherever the owning request is visible; they are not
published as global profile history.

A future qualification system may use an agent to evaluate a request against the repository's
`.scope/RULES.md`. That work needs its own product design for explanations, overrides, failure
modes, and rule changes. Until then, qualification is deliberately not part of the request domain,
API, queue, or UI.
