# Review workflow

Forge review turns a run's proposed result into a controlled repository
change. A browser request becomes a validated `ControlCommand`, is committed
to an outbox, and is delivered to `review-service`. Cancel, retry, and reject
use durable idempotent repository operations.

Approval has a deliberate two-step boundary. The repository authorizes and
claims the proposal, then the Git adapter checks the recorded input commit and
publishes the proposed result ref with compare-and-swap. Finalization records
`Approved` or `Conflicted`, so a concurrent ref movement cannot be mistaken
for a successful review.
