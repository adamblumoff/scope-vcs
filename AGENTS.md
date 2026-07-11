This product is still in pre-alpha and we have zero users and zero valuable data. Every change should be destructive, no legacy or backwards compatible bullshit, this application has no users or releases, treat it as such. If you add backwards compatibility, I will go apeshit on your dumbass.

Maintainer = owner or members

Do not modify the root `README.md` unless the user explicitly asks you to.

When coding, feel free to spin up medium-level subagents to get non-overlapping code work done and then review outputs at the end. If the work is larger, go ahead and split into worktrees accordingly and coordinate their efforts. Before doing this though please inform the user how you're going to use these subagents.

We want the architecture centered around durable domain code: the layer that defines the core concepts, rules, allowed transitions, invariants, and required side effects independent of any delivery mechanism. Outer layers should stay thin and predictable: translate inputs and outputs, call domain behavior, persist or render results, and surface errors without inventing their own rules.

Keep sources of truth singular, make side effects explicit, and prefer small behavior-owned modules over broad catch-all files. When ownership gets blurry, refactor toward clearer boundaries; when code exists only for speculation, compatibility, or half-owned future surfaces, delete it.

Please don't use cards for ui, only use them if absolutely necessary.

Once you approach around 1000 LOC in a single file, do an audit of the file and modularize accordingly unless there is a good justification. 

In general, I trust you with refactors as they don't effect the behavior of the application. However, on behavior making changes I want to be very involved and make sure we go slow and methodically. 

Autoreview timeout should be set to 15 minutes, please do other work in parallel while waiting.
