//! Register original request and actual task before any engine future is polled.
use super::jobs::{Entry, Phase, Snapshot, Tail};
use super::{AppError, Arc, Failure, ManagedRecipe, Request, Ticket};
use tokio::sync::{Mutex as AsyncMutex, oneshot, watch};

impl ManagedRecipe {
    pub(crate) fn submit(self: &Arc<Self>, request: Request) -> Result<Ticket, AppError> {
        let mut state = self.state();
        if state.closed || state.poisoned {
            return Err(AppError::Readiness(
                "managed recipe mutation intake is closed or uncertain".into(),
            ));
        }
        let sequence = state
            .next
            .checked_add(1)
            .ok_or_else(|| AppError::Task("managed recipe job sequence exhausted".into()))?;
        state.next = sequence;
        let request = Arc::new(request);
        let (done, phase) = watch::channel(Snapshot {
            phase: Phase::Running,
            command: None,
        });
        let previous = state.tails.insert(request.deployment(), Tail { phase });
        let owner = Arc::clone(self);
        let (send, receiver) = oneshot::channel();
        let (admit, admitted) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut completion = Completion {
                owner: Arc::clone(&owner),
                sequence,
                done,
                recorded: false,
                phase: Phase::Uncertain,
            };
            let (phase, outcome) = match admitted.await {
                Ok(()) => execute(&owner, sequence, previous, &completion.done).await,
                Err(_) => (Phase::Uncertain, Err(Arc::new(Failure::Unsettled))),
            };
            completion.phase = phase;
            owner.finish(sequence, outcome.clone());
            completion.recorded = true;
            drop(send.send(outcome));
            // A settled error remains an error; this signal only releases FIFO execution.
            drop(completion);
        });
        state.entries.insert(
            sequence,
            Entry {
                request,
                handle: Arc::new(AsyncMutex::new(Some(task))),
                outcome: None,
                join_error: None,
                joined: false,
            },
        );
        drop(state);
        admit
            .send(())
            .map_err(|()| AppError::Task("managed recipe task admission handoff failed".into()))?;
        Ok(Ticket(receiver))
    }
}

async fn execute(
    owner: &ManagedRecipe,
    sequence: u64,
    previous: Option<Tail>,
    done: &watch::Sender<Snapshot>,
) -> (Phase, super::Outcome) {
    let request = {
        let state = owner.state();
        state
            .entries
            .get(&sequence)
            .map(|entry| Arc::clone(&entry.request))
    };
    let Some(request) = request else {
        return (Phase::Uncertain, Err(Arc::new(Failure::Unsettled)));
    };
    if let Err(cause) = predecessor(&request, previous).await {
        // A refused replacement never entered the engine: preserve actual predecessor cause.
        done.send_replace(Snapshot {
            phase: Phase::Running,
            command: cause.command,
        });
        return (Phase::Uncertain, Err(Arc::new(Failure::Unsettled)));
    }
    // Record THIS actual request immediately before its engine future is polled.
    done.send_replace(Snapshot {
        phase: Phase::Running,
        command: Some(request.original()),
    });
    (Phase::Settled, owner.actions.run(&request).await)
}

struct Refused {
    command: Option<recipe_application::CommandIdentity>,
}
async fn predecessor(request: &Request, previous: Option<Tail>) -> Result<(), Refused> {
    let Some(mut previous) = previous else {
        return Ok(());
    };
    loop {
        let snapshot = *previous.phase.borrow_and_update();
        match snapshot.phase {
            Phase::Settled if snapshot.command.is_some() => return Ok(()),
            Phase::Uncertain
                if snapshot
                    .command
                    .is_some_and(|command| request.resumes(command)) =>
            {
                return Ok(());
            }
            Phase::Settled | Phase::Uncertain => {
                return Err(Refused {
                    command: snapshot.command,
                });
            }
            Phase::Running => {
                if previous.phase.changed().await.is_err() {
                    return Err(Refused {
                        command: snapshot.command,
                    });
                }
            }
        }
    }
}

struct Completion {
    owner: Arc<ManagedRecipe>,
    sequence: u64,
    done: watch::Sender<Snapshot>,
    recorded: bool,
    phase: Phase,
}
impl Drop for Completion {
    fn drop(&mut self) {
        if !self.recorded {
            self.owner
                .finish(self.sequence, Err(Arc::new(Failure::Unsettled)));
        }
        // Unwind or refusal preserves uncertainty across the whole FIFO tail.
        // Only a returned actual engine future settles a request (including errors).
        let command = { self.done.borrow().command };
        self.done.send_replace(Snapshot {
            phase: self.phase,
            command,
        });
    }
}
