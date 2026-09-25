use super::{
    DestroyAction, FakeInstance, InstanceState, Terminal, lock, send_event, terminal_result,
};
use async_trait::async_trait;
use tokio::sync::broadcast;
use vm_trait::{
    PrivateHttpRequest, PrivateHttpResponse, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance,
};

#[async_trait]
impl VmInstance for FakeInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        let started = {
            let mut state = lock(&self.state);
            match &*state {
                InstanceState::Provisioned => {
                    let ingress = self.provider.reserve_ingress(&self.spec.network)?;
                    *state = InstanceState::Running {
                        ingress: ingress.clone(),
                    };
                    drop(state);
                    Some(ingress)
                }
                InstanceState::Running { .. } => {
                    drop(state);
                    None
                }
                InstanceState::Exited => {
                    drop(state);
                    return Err(VmError::InvalidState("an exited VM cannot be restarted"));
                }
                InstanceState::Destroyed => {
                    drop(state);
                    return Err(VmError::Destroyed);
                }
            }
        };

        if let Some(ingress) = started {
            send_event(&self.events, VmEvent::Started { ingress });
            if let Some(authority) = &self.spec.runtime_authority {
                send_event(
                    &self.events,
                    VmEvent::RuntimeAuthorityAcknowledged {
                        session_id: authority.session_id(),
                        generation: authority.generation(),
                    },
                );
            }
            send_event(&self.events, VmEvent::Ready);
        }
        Ok(())
    }

    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        let exit = match mode {
            StopMode::Graceful { .. } => VmExit {
                code: Some(0),
                signal: None,
            },
            StopMode::Force => VmExit {
                code: None,
                signal: Some(9),
            },
            _ => {
                return Err(VmError::Unsupported {
                    feature: "stop mode".to_owned(),
                    provider: "fake".to_owned(),
                });
            }
        };

        self.finish_running(exit);
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut terminal = self.terminal.subscribe();
        loop {
            let current = terminal.borrow_and_update().clone();
            if let Some(result) = current {
                return terminal_result(result);
            }

            terminal
                .changed()
                .await
                .map_err(|source| VmError::Provider {
                    provider: "fake".to_owned(),
                    code: "terminal-channel-closed".to_owned(),
                    source: Box::new(source),
                })?;
        }
    }

    async fn invoke_private_http(
        &self,
        request: PrivateHttpRequest,
    ) -> Result<PrivateHttpResponse, VmError> {
        if !matches!(&*lock(&self.state), InstanceState::Running { .. }) {
            return Err(VmError::InvalidState("private HTTP requires a running VM"));
        }
        let responder = lock(&self.provider.private_http_responder)
            .clone()
            .ok_or_else(|| VmError::Unsupported {
                feature: "private HTTP handler transport".to_owned(),
                provider: "fake".to_owned(),
            })?;
        responder.invoke(request).await
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        let action = {
            let mut state = lock(&self.state);
            match &*state {
                InstanceState::Provisioned => {
                    *state = InstanceState::Destroyed;
                    DestroyAction::BeforeStart
                }
                InstanceState::Running { ingress } => {
                    let ingress = ingress.clone();
                    let exit = VmExit {
                        code: None,
                        signal: Some(9),
                    };
                    *state = InstanceState::Destroyed;
                    DestroyAction::Running { ingress, exit }
                }
                InstanceState::Exited => {
                    *state = InstanceState::Destroyed;
                    DestroyAction::NoTerminalChange
                }
                InstanceState::Destroyed => DestroyAction::NoTerminalChange,
            }
        };

        match action {
            DestroyAction::BeforeStart => {
                self.terminal.send_replace(Some(Terminal::Destroyed));
            }
            DestroyAction::Running { ingress, exit } => {
                self.provider.release_ingress(&ingress);
                self.terminal
                    .send_replace(Some(Terminal::Exited(exit.clone())));
                send_event(&self.events, VmEvent::Exited(exit));
            }
            DestroyAction::NoTerminalChange => {}
        }

        lock(&self.provider.ids).remove(&self.id);
        Ok(())
    }
}

impl FakeInstance {
    fn finish_running(&self, exit: VmExit) {
        let ingress = {
            let mut state = lock(&self.state);
            if let InstanceState::Running { ingress } = &*state {
                let ingress = ingress.clone();
                *state = InstanceState::Exited;
                drop(state);
                Some(ingress)
            } else {
                drop(state);
                None
            }
        };

        if let Some(ingress) = ingress {
            self.provider.release_ingress(&ingress);
            self.terminal
                .send_replace(Some(Terminal::Exited(exit.clone())));
            send_event(&self.events, VmEvent::Exited(exit));
        }
    }
}
